//! `agents/` on disk: the committed, diffable fleet definition.
//!
//! ```text
//! agents/<slug>.json                 -> AgentFile
//! agents/environments/<slug>.json    -> EnvironmentFile
//! ```

pub mod sync;

use crate::anthropic::types::{AgentDefinition, EnvironmentDefinition};
use crate::error::{Error, Result};
use crate::money::Cents;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Session-time policy the control plane applies. Budget is per session;
/// effort is *not* here because it only takes effect on the agent (see
/// `AgentDefinition::model`).
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    pub max_list_cost_cents: Cents,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AgentFile {
    pub slug: String,
    pub default_environment: String,
    pub policy: Policy,
    /// Verbatim `POST /v1/agents` body.
    pub agent: AgentDefinition,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentFile {
    pub slug: String,
    /// Verbatim `POST /v1/environments` body.
    pub environment: EnvironmentDefinition,
}

impl EnvironmentFile {
    /// `config.type`: `"cloud"` or `"self_hosted"`.
    pub fn kind(&self) -> Result<&str> {
        self.environment
            .config
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| Error::Registry {
                path: PathBuf::from(format!("environments/{}.json", self.slug)),
                reason: "environment.config.type is required".into(),
            })
    }
}

#[derive(Debug, Clone, Default)]
pub struct Registry {
    pub agents: BTreeMap<String, AgentFile>,
    pub environments: BTreeMap<String, EnvironmentFile>,
}

pub const SLUG_METADATA_KEY: &str = "iron_fleet_slug";

pub fn load_dir(dir: &Path) -> Result<Registry> {
    let mut reg = Registry::default();

    let env_dir = dir.join("environments");
    for path in json_files(&env_dir)? {
        let file: EnvironmentFile = read_json(&path)?;
        check_slug_matches_filename(&path, &file.slug)?;
        file.kind()?;
        if reg.environments.insert(file.slug.clone(), file).is_some() {
            return Err(Error::Registry {
                path,
                reason: "duplicate environment slug".into(),
            });
        }
    }

    for path in json_files(dir)? {
        let mut file: AgentFile = read_json(&path)?;
        check_slug_matches_filename(&path, &file.slug)?;
        if file.policy.max_list_cost_cents.is_zero() {
            return Err(Error::Registry {
                path,
                reason: "policy.max_list_cost_cents must be greater than zero".into(),
            });
        }
        if !reg.environments.contains_key(&file.default_environment) {
            return Err(Error::Registry {
                path,
                reason: format!(
                    "default_environment `{}` has no file in environments/",
                    file.default_environment
                ),
            });
        }
        // The slug rides on the Anthropic agent as metadata so the resource can be
        // matched back to this file even if the SQLite registry is lost.
        file.agent
            .metadata
            .entry(SLUG_METADATA_KEY.to_owned())
            .or_insert_with(|| file.slug.clone());
        if reg.agents.insert(file.slug.clone(), file).is_some() {
            return Err(Error::Registry {
                path,
                reason: "duplicate agent slug".into(),
            });
        }
    }

    if reg.agents.is_empty() {
        return Err(Error::Registry {
            path: dir.to_path_buf(),
            reason: "no agent definitions found".into(),
        });
    }
    Ok(reg)
}

fn json_files(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::Registry {
        path: dir.to_path_buf(),
        reason: e.to_string(),
    })?;
    let mut out: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_file() && p.extension().is_some_and(|x| x == "json"))
        .collect();
    out.sort();
    Ok(out)
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::Registry {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    serde_json::from_str(&text).map_err(|e| Error::Registry {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })
}

fn check_slug_matches_filename(path: &Path, slug: &str) -> Result<()> {
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if stem != slug {
        return Err(Error::Registry {
            path: path.to_path_buf(),
            reason: format!("slug `{slug}` does not match file name `{stem}`"),
        });
    }
    if slug.is_empty()
        || !slug
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(Error::Registry {
            path: path.to_path_buf(),
            reason: "slug must be lowercase letters, digits and hyphens".into(),
        });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn repo_agents_dir() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../agents")
    }

    #[test]
    fn committed_fleet_loads_and_matches_claude_md() {
        let reg = load_dir(&repo_agents_dir()).unwrap();
        let cap = |s: &str| reg.agents[s].policy.max_list_cost_cents.get();
        let effort = |s: &str| reg.agents[s].agent.model.effort.unwrap().as_str();
        assert_eq!((cap("jarvis"), effort("jarvis")), (50, "low"));
        assert_eq!(
            (cap("blueweb-client"), effort("blueweb-client")),
            (1000, "high")
        );
        assert_eq!((cap("blueweb-ops"), effort("blueweb-ops")), (200, "medium"));
        assert_eq!((cap("gpu-compute"), effort("gpu-compute")), (500, "medium"));
        assert!(
            reg.agents["blueweb-ops"].agent.tools.is_empty(),
            "no code tools"
        );
        assert_eq!(reg.agents["gpu-compute"].default_environment, "rig-gpu");
        assert_eq!(reg.environments["cloud-default"].kind().unwrap(), "cloud");
        assert_eq!(
            reg.agents["jarvis"].agent.metadata[SLUG_METADATA_KEY],
            "jarvis"
        );
    }

    #[test]
    fn zero_budget_is_a_load_error() {
        let dir = std::env::temp_dir().join(format!("iron-fleet-zero-cap-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("environments")).unwrap();
        std::fs::copy(
            repo_agents_dir().join("environments/cloud-default.json"),
            dir.join("environments/cloud-default.json"),
        )
        .unwrap();
        std::fs::write(
            dir.join("zero.json"),
            r#"{"slug":"zero","default_environment":"cloud-default",
                "policy":{"max_list_cost_cents":"0"},
                "agent":{"name":"z","model":{"id":"claude-opus-5"}}}"#,
        )
        .unwrap();
        let err = load_dir(&dir).unwrap_err().to_string();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(err.contains("greater than zero"), "{err}");
    }

    #[test]
    fn numeric_budget_is_a_load_error() {
        let bad: std::result::Result<AgentFile, _> = serde_json::from_str(
            r#"{"slug":"x","default_environment":"cloud-default",
                "policy":{"max_list_cost_cents":50},
                "agent":{"name":"x","model":{"id":"claude-opus-5"}}}"#,
        );
        assert!(bad.is_err());
    }
}
