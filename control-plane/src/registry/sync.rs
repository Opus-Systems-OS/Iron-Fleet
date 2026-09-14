//! Reconcile `agents/` → Anthropic → SQLite. Idempotent; safe to run on every boot.
//!
//! - Environments: created once, `cloud` and `self_hosted` alike. A freshly
//!   created `self_hosted` environment's `environment_key` is returned to the
//!   caller in `SyncReport::new_environment_keys` instead of being stored —
//!   the rig owns that key (CLAUDE.md) — so the caller can surface it exactly
//!   once and the operator copies it into `RIG_ENVIRONMENT_KEY` on the rig.
//! - Agents: created if unknown, updated (new version) if the definition hash
//!   changed, otherwise untouched. Policy columns are refreshed every run.

use super::{Registry, SLUG_METADATA_KEY};
use crate::anthropic::Client;
use crate::db::{AgentRow, Db};
use crate::error::{Error, Result};
use sha2::{Digest, Sha256};

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub environments_created: usize,
    pub agents_created: usize,
    pub agents_updated: usize,
    pub agents_unchanged: usize,
    /// `(slug, environment_id, environment_key)` for each `self_hosted`
    /// environment provisioned *this run*. The caller must print these once
    /// and never pass them to `tracing` (they'd end up in aggregated logs).
    pub new_environment_keys: Vec<(String, String, String)>,
}

pub async fn sync(reg: &Registry, api: &Client, db: &Db) -> Result<SyncReport> {
    let mut report = SyncReport::default();

    for (slug, file) in &reg.environments {
        let kind = file.kind()?;
        let existing = db.environment(slug)?;
        match existing.and_then(|e| e.environment_id) {
            Some(id) => {
                tracing::debug!(slug, id, kind, "environment already provisioned");
                db.upsert_environment(slug, kind, Some(&id))?;
            }
            None => {
                let env = api.create_environment(&file.environment).await?;
                tracing::info!(slug, id = %env.id, kind, "environment created");
                db.upsert_environment(slug, kind, Some(&env.id))?;
                report.environments_created += 1;
                if kind == "self_hosted" {
                    match &env.environment_key {
                        Some(key) => report.new_environment_keys.push((
                            slug.clone(),
                            env.id.clone(),
                            key.clone(),
                        )),
                        None => tracing::warn!(
                            slug,
                            id = %env.id,
                            "self_hosted environment created but the API returned no environment_key; \
                             the rig cannot authenticate as this environment until one is issued"
                        ),
                    }
                }
            }
        }
    }

    for (slug, file) in &reg.agents {
        let hash = definition_hash(&file.agent)?;
        let effort = file
            .agent
            .model
            .effort
            .map(|e| e.as_str().to_owned())
            .unwrap_or_else(|| "default".to_owned());
        debug_assert_eq!(file.agent.metadata.get(SLUG_METADATA_KEY), Some(slug));

        let existing = db.agent(slug)?;
        let (agent_id, version) = match existing {
            Some(row) if row.definition_sha256 == hash => {
                report.agents_unchanged += 1;
                (row.agent_id, row.agent_version)
            }
            Some(row) => {
                let agent = api.update_agent(&row.agent_id, &file.agent).await?;
                tracing::info!(slug, id = %agent.id, version = agent.version, "agent updated");
                report.agents_updated += 1;
                (agent.id, agent.version)
            }
            None => {
                let agent = api.create_agent(&file.agent).await?;
                tracing::info!(slug, id = %agent.id, version = agent.version, "agent created");
                report.agents_created += 1;
                (agent.id, agent.version)
            }
        };

        db.upsert_agent(&AgentRow {
            slug: slug.clone(),
            agent_id,
            agent_version: version,
            definition_sha256: hash,
            max_list_cost_cents: file.policy.max_list_cost_cents,
            effort,
            default_environment: file.default_environment.clone(),
            synced_at: String::new(),
        })?;
    }

    tracing::info!(?report, "registry sync complete");
    Ok(report)
}

/// Hash of the canonical (serde, BTreeMap-ordered) JSON of the agent body.
fn definition_hash(def: &crate::anthropic::types::AgentDefinition) -> Result<String> {
    let value = serde_json::to_value(def).map_err(|e| Error::Config(e.to_string()))?;
    let canonical = canonical_json(&value);
    Ok(hex::encode(Sha256::digest(canonical.as_bytes())))
}

/// serde_json::Value objects are BTreeMaps unless `preserve_order` is on (it is
/// not), so `to_string` is already key-sorted; this just makes that explicit.
fn canonical_json(v: &serde_json::Value) -> String {
    serde_json::to_string(v).expect("Value is always serializable")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::anthropic::types::*;

    #[test]
    fn hash_is_stable_across_key_order_and_changes_with_content() {
        let a: AgentDefinition = serde_json::from_str(
            r#"{"name":"A","model":{"id":"claude-opus-5","effort":"low"},"tools":[{"type":"agent_toolset_20260401"}]}"#,
        )
        .unwrap();
        let b: AgentDefinition = serde_json::from_str(
            r#"{"tools":[{"type":"agent_toolset_20260401"}],"model":{"effort":"low","id":"claude-opus-5"},"name":"A"}"#,
        )
        .unwrap();
        let c: AgentDefinition = serde_json::from_str(
            r#"{"name":"A","model":{"id":"claude-opus-5","effort":"high"},"tools":[{"type":"agent_toolset_20260401"}]}"#,
        )
        .unwrap();
        assert_eq!(definition_hash(&a).unwrap(), definition_hash(&b).unwrap());
        assert_ne!(definition_hash(&a).unwrap(), definition_hash(&c).unwrap());
    }
}
