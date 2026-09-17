//! Reconcile `agents/` → Anthropic → SQLite. Idempotent; safe to run on every boot.
//!
//! - Environments: created once, `cloud` and `self_hosted` alike. If the
//!   create response for a `self_hosted` environment ever carries an
//!   `environment_key` it is returned in `SyncReport::new_environment_keys`
//!   instead of being stored — the rig owns that key (CLAUDE.md). In
//!   practice the API returns none (confirmed 2026-09-17): environment keys
//!   are generated in the Console and go straight into `worker/sdk/.env` on
//!   the rig, so the warn branch below is the one that fires.
//! - Skills: uploaded if unknown, given a new version if the content hash
//!   changed, otherwise untouched. Run before agents so their ids can be
//!   pinned into agent definitions (`resolve_skills`).
//! - Agents: created if unknown, updated (new version) if the definition hash
//!   changed, otherwise untouched. Policy columns are refreshed every run.
//! - Credentials: after agents (the vault row references the agent row) —
//!   see `registry::credentials`.

use super::credentials::{self, CredentialReport};
use super::{repo_skill_ref, Registry, SKILL_REF_KEY, SLUG_METADATA_KEY};
use crate::anthropic::types::AgentDefinition;
use crate::anthropic::Client;
use crate::db::{AgentGithubRow, AgentRow, Db, SkillRow};
use crate::error::{Error, Result};
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;

#[derive(Debug, Default, PartialEq, Eq)]
pub struct SyncReport {
    pub environments_created: usize,
    pub skills_created: usize,
    pub skills_updated: usize,
    pub skills_unchanged: usize,
    pub agents_created: usize,
    pub agents_updated: usize,
    pub agents_unchanged: usize,
    pub credentials: CredentialReport,
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
                            "self_hosted environment created; the API does not return an environment_key — \
                             generate one in the Console (Environments → {slug} → Generate environment key) \
                             and put it in worker/sdk/.env on the rig"
                        ),
                    }
                }
            }
        }
    }

    // name -> (skill_id, version_id) for everything under agents/skills/.
    let mut synced_skills: BTreeMap<String, (String, String)> = BTreeMap::new();
    for (name, skill) in &reg.skills {
        let existing = db.skill(name)?;
        let (skill_id, version_id) = match existing {
            Some(row) if row.content_sha256 == skill.sha256 => {
                report.skills_unchanged += 1;
                (row.skill_id, row.version_id)
            }
            Some(row) => {
                let version = api.create_skill_version(&row.skill_id, skill).await?;
                if version.name != *name {
                    // The stored id points at some other skill — a SQLite row
                    // that outlived a directory rename, say. Stop rather than
                    // keep versioning the wrong one.
                    return Err(Error::Config(format!(
                        "skill `{name}` is stored as {} but Anthropic calls that skill `{}`",
                        row.skill_id, version.name
                    )));
                }
                tracing::info!(name, id = %row.skill_id, version = %version.id, "skill updated");
                report.skills_updated += 1;
                (row.skill_id, version.id)
            }
            None => {
                let created = api.create_skill(skill).await?;
                tracing::info!(name, id = %created.id, version = %created.latest_version_id, "skill created");
                report.skills_created += 1;
                (created.id, created.latest_version_id)
            }
        };
        db.upsert_skill(&SkillRow {
            name: name.clone(),
            skill_id: skill_id.clone(),
            version_id: version_id.clone(),
            content_sha256: skill.sha256.clone(),
        })?;
        synced_skills.insert(name.clone(), (skill_id, version_id));
    }

    for (slug, file) in &reg.agents {
        let agent_def = resolve_skills(&file.agent, &synced_skills)?;
        let hash = definition_hash(&agent_def)?;
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
                let agent = api.update_agent(&row.agent_id, &agent_def).await?;
                tracing::info!(slug, id = %agent.id, version = agent.version, "agent updated");
                report.agents_updated += 1;
                (agent.id, agent.version)
            }
            None => {
                let agent = api.create_agent(&agent_def).await?;
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
        let github = file.github.as_ref().map(|g| AgentGithubRow {
            token_env: g.token_env.clone(),
            mounts: g.mount.clone(),
        });
        db.set_agent_github(slug, github.as_ref())?;
    }

    report.credentials = credentials::sync(reg, api, db).await?;

    tracing::info!(?report, "registry sync complete");
    Ok(report)
}

/// Rewrite `{"type": "custom", "skill": "<name>"}` entries to the wire form
/// `{"type": "custom", "skill_id": …, "version": …}`, pinned to the version id
/// this sync just uploaded (not `"latest"`): a skill change then flows through
/// the agent's definition hash into a new agent version, and a session pinned
/// to an agent version gets the matching skill snapshot. Every other entry is
/// passed through unchanged. `load_dir` already rejected unknown names, so a
/// miss here is a bug, not a config error.
fn resolve_skills(
    def: &AgentDefinition,
    synced: &BTreeMap<String, (String, String)>,
) -> Result<AgentDefinition> {
    let mut out = def.clone();
    for entry in &mut out.skills {
        let Some(name) = repo_skill_ref(entry) else {
            continue;
        };
        let (skill_id, version_id) = synced
            .get(name)
            .ok_or_else(|| Error::Config(format!("skill `{name}` referenced but not synced")))?;
        let mut resolved = entry.clone();
        let obj = resolved
            .as_object_mut()
            .expect("repo_skill_ref matched an object");
        obj.remove(SKILL_REF_KEY);
        obj.insert("skill_id".into(), json!(skill_id));
        obj.insert("version".into(), json!(version_id));
        *entry = resolved;
    }
    Ok(out)
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

    #[test]
    fn resolve_skills_pins_repo_refs_and_passes_the_rest_through() {
        let def: AgentDefinition = serde_json::from_str(
            r#"{"name":"A","model":{"id":"claude-opus-5"},"tools":[],
                "skills":[
                  {"type":"custom","skill":"blueweb-customer-site"},
                  {"type":"anthropic","skill_id":"xlsx"},
                  {"type":"custom","skill_id":"skill_literal","version":"latest"}
                ]}"#,
        )
        .unwrap();
        let synced = BTreeMap::from([(
            "blueweb-customer-site".to_owned(),
            ("skill_01".to_owned(), "skillver_07".to_owned()),
        )]);
        let out = resolve_skills(&def, &synced).unwrap();
        assert_eq!(
            out.skills[0],
            serde_json::json!({"type":"custom","skill_id":"skill_01","version":"skillver_07"})
        );
        assert_eq!(out.skills[1], def.skills[1]);
        assert_eq!(out.skills[2], def.skills[2]);

        // A different skill version is a different agent definition.
        let other = BTreeMap::from([(
            "blueweb-customer-site".to_owned(),
            ("skill_01".to_owned(), "skillver_08".to_owned()),
        )]);
        assert_ne!(
            definition_hash(&out).unwrap(),
            definition_hash(&resolve_skills(&def, &other).unwrap()).unwrap()
        );

        let err = resolve_skills(&def, &BTreeMap::new())
            .unwrap_err()
            .to_string();
        assert!(err.contains("blueweb-customer-site"), "{err}");
    }
}
