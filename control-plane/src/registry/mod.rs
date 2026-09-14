//! `agents/` on disk: the committed, diffable fleet definition.
//!
//! ```text
//! agents/<slug>.json                 -> AgentFile
//! agents/environments/<slug>.json    -> EnvironmentFile
//! agents/skills/<name>/SKILL.md ...  -> SkillDir (a custom skill, uploaded whole)
//! ```

pub mod sync;

use crate::anthropic::types::{AgentDefinition, EnvironmentDefinition};
use crate::error::{Error, Result};
use crate::money::Cents;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};
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

/// One custom skill: the directory `agents/skills/<name>/`, read whole. Uploaded
/// to the Skills API as a full snapshot (a version is never a delta), so this
/// holds every file's bytes and a hash over all of them.
#[derive(Debug, Clone)]
pub struct SkillDir {
    /// Directory name == `SKILL.md` frontmatter `name` (Anthropic makes the
    /// latter immutable from the first upload, so the two must agree up front).
    pub name: String,
    /// `(path relative to the skill dir, bytes)`, sorted by path.
    pub files: Vec<(String, Vec<u8>)>,
    /// SHA-256 over every `(path, bytes)` pair; any byte change is a new version.
    pub sha256: String,
}

#[derive(Debug, Clone, Default)]
pub struct Registry {
    pub agents: BTreeMap<String, AgentFile>,
    pub environments: BTreeMap<String, EnvironmentFile>,
    pub skills: BTreeMap<String, SkillDir>,
}

pub const SLUG_METADATA_KEY: &str = "iron_fleet_slug";

/// The one field in an agent's "verbatim `POST /v1/agents` body" that is not
/// on the wire: `{"type": "custom", "skill": "<dir name>"}` names a directory
/// under `agents/skills/`, and sync rewrites it to the real `skill_id` +
/// `version` once that skill is uploaded (`sync::resolve_skills`). Entries that
/// already carry a `skill_id` (Anthropic's `xlsx` etc., or a literal id) pass
/// through untouched.
pub const SKILL_REF_KEY: &str = "skill";

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

    let skills_dir = dir.join("skills");
    if skills_dir.is_dir() {
        for path in skill_dirs(&skills_dir)? {
            let skill = load_skill_dir(&path)?;
            reg.skills.insert(skill.name.clone(), skill);
        }
    }

    for path in json_files(dir)? {
        let mut file: AgentFile = read_json(&path)?;
        check_slug_matches_filename(&path, &file.slug)?;
        for entry in &file.agent.skills {
            if let Some(name) = repo_skill_ref(entry) {
                if !reg.skills.contains_key(name) {
                    return Err(Error::Registry {
                        path,
                        reason: format!("skill `{name}` has no directory in skills/"),
                    });
                }
            }
        }
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

/// `skills/<name>/` directories, sorted. Non-directories are ignored so a
/// stray `.DS_Store` cannot break the fleet.
fn skill_dirs(dir: &Path) -> Result<Vec<PathBuf>> {
    let entries = std::fs::read_dir(dir).map_err(|e| Error::Registry {
        path: dir.to_path_buf(),
        reason: e.to_string(),
    })?;
    let mut out: Vec<PathBuf> = entries
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.is_dir())
        .collect();
    out.sort();
    Ok(out)
}

/// If `entry` is `{"type": "custom", "skill": "<name>"}`, the name.
pub fn repo_skill_ref(entry: &Value) -> Option<&str> {
    if entry.get("type").and_then(Value::as_str) != Some("custom") {
        return None;
    }
    entry.get(SKILL_REF_KEY).and_then(Value::as_str)
}

pub fn load_skill_dir(root: &Path) -> Result<SkillDir> {
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_owned();
    check_slug_matches_filename(root, &name)?;

    let mut files = Vec::new();
    collect_files(root, root, &mut files)?;
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let skill_md = files
        .iter()
        .find(|(p, _)| p == "SKILL.md")
        .ok_or_else(|| Error::Registry {
            path: root.to_path_buf(),
            reason: "skill directory has no SKILL.md".into(),
        })?;
    let frontmatter_name = frontmatter_name(&skill_md.1).ok_or_else(|| Error::Registry {
        path: root.join("SKILL.md"),
        reason: "SKILL.md frontmatter has no `name:`".into(),
    })?;
    if frontmatter_name != name {
        return Err(Error::Registry {
            path: root.join("SKILL.md"),
            reason: format!(
                "frontmatter name `{frontmatter_name}` does not match directory `{name}`"
            ),
        });
    }

    let mut hasher = Sha256::new();
    for (path, bytes) in &files {
        hasher.update(path.as_bytes());
        hasher.update([0]);
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    Ok(SkillDir {
        name,
        files,
        sha256: hex::encode(hasher.finalize()),
    })
}

fn collect_files(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) -> Result<()> {
    let io = |e: std::io::Error| Error::Registry {
        path: dir.to_path_buf(),
        reason: e.to_string(),
    };
    for entry in std::fs::read_dir(dir).map_err(io)? {
        let path = entry.map_err(io)?.path();
        let file_name = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or_default();
        if matches!(file_name, ".DS_Store" | "node_modules" | ".git") {
            continue;
        }
        if path.is_dir() {
            collect_files(root, &path, out)?;
        } else if path.is_file() {
            let rel = path
                .strip_prefix(root)
                .expect("path is under root")
                .to_string_lossy()
                .replace(std::path::MAIN_SEPARATOR, "/");
            let bytes = std::fs::read(&path).map_err(|e| Error::Registry {
                path: path.clone(),
                reason: e.to_string(),
            })?;
            out.push((rel, bytes));
        }
    }
    Ok(())
}

/// `name:` from the YAML frontmatter (`---` … `---`) at the top of SKILL.md.
/// Deliberately minimal: the frontmatter is two required scalar keys, not a
/// YAML document worth a dependency.
fn frontmatter_name(skill_md: &[u8]) -> Option<String> {
    let text = std::str::from_utf8(skill_md).ok()?;
    let mut lines = text.lines();
    if lines.next()?.trim() != "---" {
        return None;
    }
    for line in lines {
        if line.trim() == "---" {
            break;
        }
        if let Some(rest) = line.strip_prefix("name:") {
            let v = rest.trim().trim_matches('"').trim_matches('\'');
            if !v.is_empty() {
                return Some(v.to_owned());
            }
        }
    }
    None
}

fn read_json<T: serde::de::DeserializeOwned>(path: &Path) -> Result<T> {
    let text = std::fs::read_to_string(path).map_err(|e| Error::Registry {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })?;
    let text = substitute_env_vars(path, &text)?;
    serde_json::from_str(&text).map_err(|e| Error::Registry {
        path: path.to_path_buf(),
        reason: e.to_string(),
    })
}

/// Expands `${VAR_NAME}` to `std::env::var("VAR_NAME")` before parsing, so a
/// committed file can reference a secret (an MCP server's bearer token, say)
/// without that secret ever being committed — CLAUDE.md: "never commit an
/// API key, environment key, or GitHub token." This runs on raw text before
/// JSON parsing, so a substituted value must not itself need JSON escaping
/// (no `"`, backslash, or control characters) — fine for tokens and URLs,
/// not a general templating engine.
fn substitute_env_vars(path: &Path, text: &str) -> Result<String> {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find("${") {
        out.push_str(&rest[..start]);
        let after = &rest[start + 2..];
        let end = after.find('}').ok_or_else(|| Error::Registry {
            path: path.to_path_buf(),
            reason: "unterminated ${...}".into(),
        })?;
        let name = &after[..end];
        let value = std::env::var(name).map_err(|_| Error::Registry {
            path: path.to_path_buf(),
            reason: format!("${{{name}}} is not set"),
        })?;
        out.push_str(&value);
        rest = &after[end + 1..];
    }
    out.push_str(rest);
    Ok(out)
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
        // jarvis.json references ${MCP_FLEET_URL} so the real value never
        // gets committed. No other test reads this name, so setting it here
        // doesn't race parallel test execution.
        unsafe {
            std::env::set_var("MCP_FLEET_URL", "https://mcp-fleet.internal.example/mcp");
        }

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

        // mcp_servers and its matching mcp_toolset entry must be mutually
        // consistent, per docs.claude.com/managed-agents/mcp-connector: "The
        // API rejects agent definitions with unreferenced servers or
        // dangling toolsets."
        let mcp_servers = &reg.agents["jarvis"].agent.mcp_servers;
        assert_eq!(mcp_servers.len(), 1);
        assert_eq!(mcp_servers[0]["name"], "fleet");
        assert_eq!(
            mcp_servers[0]["url"],
            "https://mcp-fleet.internal.example/mcp"
        );
        assert!(
            mcp_servers[0].get("authorization_token").is_none(),
            "auth is a session-time vault_ids concern, not an agent-level field"
        );
        let toolset = reg.agents["jarvis"]
            .agent
            .tools
            .iter()
            .find(|t| t["type"] == "mcp_toolset")
            .expect("jarvis declares a matching mcp_toolset entry");
        assert_eq!(toolset["mcp_server_name"], "fleet");
    }

    #[test]
    fn env_var_substitution_expands_and_reports_missing() {
        unsafe {
            std::env::set_var("IRON_FLEET_TEST_SUBSTITUTE_VAR", "shhh");
        }
        let p = Path::new("test.json");
        assert_eq!(
            substitute_env_vars(p, r#"{"a":"${IRON_FLEET_TEST_SUBSTITUTE_VAR}!"}"#).unwrap(),
            r#"{"a":"shhh!"}"#
        );
        let err = substitute_env_vars(p, "${IRON_FLEET_TEST_DOES_NOT_EXIST}").unwrap_err();
        assert!(err.to_string().contains("IRON_FLEET_TEST_DOES_NOT_EXIST"));
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
    fn committed_skill_loads_and_blueweb_client_references_it() {
        unsafe {
            std::env::set_var("MCP_FLEET_URL", "https://mcp-fleet.internal.example/mcp");
        }
        let reg = load_dir(&repo_agents_dir()).unwrap();
        let skill = &reg.skills["blueweb-customer-site"];
        assert!(skill.files.iter().any(|(p, _)| p == "SKILL.md"));
        assert!(skill.files.iter().any(|(p, _)| p == "scripts/new-site.sh"));
        assert!(
            skill.files.iter().all(|(p, _)| !p.contains(".DS_Store")),
            "Finder litter must not be uploaded"
        );
        // The skill is mounted somewhere under the sandbox, never at the
        // author's ~/.claude path — nothing in it may assume that path.
        for (p, bytes) in &skill.files {
            assert!(
                !String::from_utf8_lossy(bytes).contains(".claude/skills"),
                "{p} hardcodes a local skills path"
            );
        }
        let refs: Vec<&str> = reg.agents["blueweb-client"]
            .agent
            .skills
            .iter()
            .filter_map(repo_skill_ref)
            .collect();
        assert_eq!(refs, ["blueweb-customer-site"]);
    }

    fn scratch_skill(tag: &str, dir_name: &str, skill_md: &str) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("iron-fleet-skill-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let skill = root.join(dir_name);
        std::fs::create_dir_all(skill.join("scripts")).unwrap();
        std::fs::write(skill.join("SKILL.md"), skill_md).unwrap();
        std::fs::write(skill.join("scripts/run.sh"), "#!/bin/sh\n").unwrap();
        std::fs::write(skill.join(".DS_Store"), "junk").unwrap();
        skill
    }

    #[test]
    fn skill_dir_hash_tracks_content_and_paths_are_relative() {
        let md = "---\nname: demo-skill\ndescription: d\n---\nbody\n";
        let skill = scratch_skill("hash", "demo-skill", md);
        let a = load_skill_dir(&skill).unwrap();
        assert_eq!(a.name, "demo-skill");
        let paths: Vec<&str> = a.files.iter().map(|(p, _)| p.as_str()).collect();
        assert_eq!(paths, ["SKILL.md", "scripts/run.sh"]);

        let b = load_skill_dir(&skill).unwrap();
        assert_eq!(a.sha256, b.sha256, "hash is stable across loads");

        std::fs::write(skill.join("scripts/run.sh"), "#!/bin/sh\necho hi\n").unwrap();
        let c = load_skill_dir(&skill).unwrap();
        assert_ne!(
            a.sha256, c.sha256,
            "a byte change anywhere is a new version"
        );
        std::fs::remove_dir_all(skill.parent().unwrap()).unwrap();
    }

    #[test]
    fn skill_dir_name_must_match_frontmatter_and_have_skill_md() {
        let skill = scratch_skill(
            "mismatch",
            "demo-skill",
            "---\nname: other-name\ndescription: d\n---\n",
        );
        let err = load_skill_dir(&skill).unwrap_err().to_string();
        assert!(err.contains("does not match directory"), "{err}");

        std::fs::remove_file(skill.join("SKILL.md")).unwrap();
        let err = load_skill_dir(&skill).unwrap_err().to_string();
        assert!(err.contains("no SKILL.md"), "{err}");
        std::fs::remove_dir_all(skill.parent().unwrap()).unwrap();
    }

    #[test]
    fn unknown_skill_reference_is_a_load_error() {
        let dir = std::env::temp_dir().join(format!("iron-fleet-no-skill-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("environments")).unwrap();
        std::fs::copy(
            repo_agents_dir().join("environments/cloud-default.json"),
            dir.join("environments/cloud-default.json"),
        )
        .unwrap();
        std::fs::write(
            dir.join("x.json"),
            r#"{"slug":"x","default_environment":"cloud-default",
                "policy":{"max_list_cost_cents":"5"},
                "agent":{"name":"x","model":{"id":"claude-opus-5"},
                         "skills":[{"type":"custom","skill":"nope"}]}}"#,
        )
        .unwrap();
        let err = load_dir(&dir).unwrap_err().to_string();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(err.contains("skill `nope` has no directory"), "{err}");
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
