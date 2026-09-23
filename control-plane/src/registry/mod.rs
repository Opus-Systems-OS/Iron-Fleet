//! `agents/` on disk: the committed, diffable fleet definition.
//!
//! ```text
//! agents/<slug>.json                 -> AgentFile
//! agents/environments/<slug>.json    -> EnvironmentFile
//! agents/skills/<name>/SKILL.md ...  -> SkillDir (a custom skill, uploaded whole)
//! ```

pub mod credentials;
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
    /// Sandbox secrets, provisioned into a per-agent vault at sync
    /// (`registry::credentials`). Registry-level, like `policy`: not part of
    /// the agent body, so never part of its definition hash.
    #[serde(default)]
    pub credentials: Vec<CredentialSpec>,
    /// GitHub access for repository mounts (`http::sessions`).
    #[serde(default)]
    pub github: Option<GithubSpec>,
    /// Verbatim `POST /v1/agents` body.
    pub agent: AgentDefinition,
}

/// One vault credential. The secret itself is **not** in this file:
/// `from_env` names the control-plane env var that holds it, read at sync
/// time — never via `${VAR}` text substitution, which would put the value
/// inside this `Debug`-derived struct.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum CredentialSpec {
    /// An env var in the sandbox whose real value is substituted at egress,
    /// in request headers only, on `allowed_hosts` only. Right for CLIs that
    /// send the token verbatim (`gh`, `wrangler`); useless for git-over-HTTPS,
    /// which GitHub only accepts as base64 Basic auth.
    EnvironmentVariable {
        /// The environment variable the sandbox sees, e.g. `GH_TOKEN`.
        secret_name: String,
        from_env: String,
        /// Bare hostnames or `*.` wildcards.
        allowed_hosts: Vec<String>,
    },
    /// A bearer token for one of the agent's `mcp_servers`, injected when the
    /// session connects to that exact URL. This is how the agent pushes to
    /// GitHub: through the GitHub MCP server, not `git push`.
    StaticBearer {
        mcp_server_url: String,
        from_env: String,
    },
}

impl CredentialSpec {
    /// The immutable key Anthropic dedupes on within a vault (`secret_name`
    /// or `mcp_server_url`); also this credential's key in `agent_credentials`.
    pub fn key(&self) -> &str {
        match self {
            CredentialSpec::EnvironmentVariable { secret_name, .. } => secret_name,
            CredentialSpec::StaticBearer { mcp_server_url, .. } => mcp_server_url,
        }
    }

    pub fn env_var(&self) -> &str {
        match self {
            CredentialSpec::EnvironmentVariable { from_env, .. }
            | CredentialSpec::StaticBearer { from_env, .. } => from_env,
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GithubSpec {
    /// Env var holding the token used as `authorization_token` for every
    /// mount on this agent. Read per session create; never stored.
    pub token_env: String,
    /// Repositories cloned into every session of this agent.
    #[serde(default)]
    pub mount: Vec<String>,
}

/// The only URL form the `github_repository` resource accepts:
/// `https://github.com/<owner>/<repo>`, no `.git`, no trailing path.
pub fn is_github_repo_url(url: &str) -> bool {
    let Some(rest) = url.strip_prefix("https://github.com/") else {
        return false;
    };
    let mut parts = rest.split('/');
    let (Some(owner), Some(repo), None) = (parts.next(), parts.next(), parts.next()) else {
        return false;
    };
    let ok = |s: &str| {
        !s.is_empty()
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.')
    };
    ok(owner) && ok(repo) && !repo.ends_with(".git")
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
        if let Err(reason) = check_credentials(&file.credentials, &file.agent.mcp_servers) {
            return Err(Error::Registry { path, reason });
        }
        if let Err(reason) = check_mcp_toolset_policies(&file.agent.tools) {
            return Err(Error::Registry { path, reason });
        }
        if let Some(gh) = &file.github {
            if let Err(reason) = check_github(gh) {
                return Err(Error::Registry { path, reason });
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
    // Rosters name other agents, so they are checked once every file is in.
    for (slug, file) in &reg.agents {
        if let Err(reason) = check_roster(slug, file, &reg.agents) {
            return Err(Error::Registry {
                path: dir.join(format!("{slug}.json")),
                reason,
            });
        }
    }
    Ok(reg)
}

/// The roster key that names a member in `agents/` — rewritten to the
/// member's synced `id` + `version` by `sync::resolve_roster`.
pub const ROSTER_SLUG_KEY: &str = "slug";
/// Anthropic's limit on unique agents in `multiagent.agents`.
pub const ROSTER_MAX_AGENTS: usize = 20;

/// A roster must be a coordinator block whose `agent` members are named by
/// slug (the fleet stays reproducible: ids are Anthropic's, slugs are ours),
/// exist in `agents/`, and have no roster of their own — Anthropic allows
/// one level of delegation and rejects the rest at create time; failing at
/// load time says which file is wrong. `self` and `advisor` entries pass
/// through.
fn check_roster(
    slug: &str,
    file: &AgentFile,
    agents: &BTreeMap<String, AgentFile>,
) -> std::result::Result<(), String> {
    let Some(multi) = &file.agent.multiagent else {
        return Ok(());
    };
    if multi.get("type").and_then(Value::as_str) != Some("coordinator") {
        return Err(r#"agent.multiagent.type must be "coordinator""#.into());
    }
    let entries = multi
        .get("agents")
        .and_then(Value::as_array)
        .ok_or("agent.multiagent.agents must be an array")?;
    if entries.is_empty() || entries.len() > ROSTER_MAX_AGENTS {
        return Err(format!(
            "agent.multiagent.agents must list 1 to {ROSTER_MAX_AGENTS} agents"
        ));
    }
    let mut seen = std::collections::BTreeSet::new();
    for entry in entries {
        match entry.get("type").and_then(Value::as_str) {
            Some("agent") => {
                if entry.get("id").is_some() {
                    return Err(format!(
                        "roster entries name members by `{ROSTER_SLUG_KEY}`, not `id`: {entry}"
                    ));
                }
                let member = entry
                    .get(ROSTER_SLUG_KEY)
                    .and_then(Value::as_str)
                    .ok_or_else(|| format!("roster entry needs a `{ROSTER_SLUG_KEY}`: {entry}"))?;
                if member == slug {
                    return Err(
                        r#"a coordinator lists itself as {"type": "self"}, not by slug"#.into(),
                    );
                }
                let Some(member_file) = agents.get(member) else {
                    return Err(format!("roster member `{member}` has no file in agents/"));
                };
                if member_file.agent.multiagent.is_some() {
                    return Err(format!(
                        "roster member `{member}` is itself a coordinator; delegation is one level deep"
                    ));
                }
                if !seen.insert(member) {
                    return Err(format!("roster lists `{member}` twice"));
                }
            }
            Some("self") | Some("advisor") => {}
            _ => return Err(format!("unknown roster entry: {entry}")),
        }
    }
    Ok(())
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

/// Limits from platform.claude.com/docs/managed-agents/vaults: 20 credentials
/// per vault, 16 hosts per credential, `secret_name` unique within a vault.
fn check_credentials(
    specs: &[CredentialSpec],
    mcp_servers: &[Value],
) -> std::result::Result<(), String> {
    if specs.len() > 20 {
        return Err("credentials: at most 20 per agent (one vault)".into());
    }
    let mut seen = std::collections::BTreeSet::new();
    for c in specs {
        if !seen.insert(c.key()) {
            return Err(format!("credentials: duplicate key `{}`", c.key()));
        }
        if c.env_var().is_empty() {
            return Err(format!("credentials: `{}` has an empty from_env", c.key()));
        }
        match c {
            CredentialSpec::EnvironmentVariable {
                secret_name,
                allowed_hosts,
                ..
            } => {
                let name_ok = secret_name
                    .bytes()
                    .next()
                    .is_some_and(|b| b.is_ascii_uppercase())
                    && secret_name
                        .bytes()
                        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_');
                if !name_ok {
                    return Err(format!(
                        "credentials: secret_name `{secret_name}` must look like an environment variable (A-Z, 0-9, _)"
                    ));
                }
                if allowed_hosts.is_empty() || allowed_hosts.len() > 16 {
                    return Err(format!(
                        "credentials: `{secret_name}` needs 1-16 allowed_hosts"
                    ));
                }
                if let Some(bad) = allowed_hosts
                    .iter()
                    .find(|h| h.contains("://") || h.contains('/') || h.contains(':'))
                {
                    return Err(format!(
                        "credentials: `{secret_name}` allowed_hosts entry `{bad}` must be a bare hostname"
                    ));
                }
            }
            CredentialSpec::StaticBearer { mcp_server_url, .. } => {
                // A bearer credential only ever applies to a server the agent's
                // own definition references, by exact URL; one that matches
                // nothing is a typo, not a spare.
                let declared = mcp_servers
                    .iter()
                    .any(|s| s.get("url").and_then(Value::as_str) == Some(mcp_server_url));
                if !declared {
                    return Err(format!(
                        "credentials: static_bearer for `{mcp_server_url}` matches no agent.mcp_servers url"
                    ));
                }
            }
        }
    }
    Ok(())
}

/// MCP toolsets default to `always_ask` on Anthropic's side, which pauses the
/// session (`stop_reason: requires_action`) until a client sends a
/// `user.tool_confirmation` — and nothing in this fleet does, so an
/// unconfigured MCP toolset is a session that silently hangs on its first
/// MCP call (found the hard way, 2026-09-14). Require the policy to be
/// spelled out; `always_allow` is the only one that never stalls until a
/// confirmation loop exists.
fn check_mcp_toolset_policies(tools: &[Value]) -> std::result::Result<(), String> {
    for t in tools {
        if t.get("type").and_then(Value::as_str) != Some("mcp_toolset") {
            continue;
        }
        let name = t
            .get("mcp_server_name")
            .and_then(Value::as_str)
            .unwrap_or("?");
        let policy = t
            .pointer("/default_config/permission_policy/type")
            .and_then(Value::as_str);
        match policy {
            Some("always_allow") | Some("auto") => {}
            Some(other) => {
                return Err(format!(
                    "tools: mcp_toolset `{name}` policy `{other}` would pause sessions; nothing in the fleet answers tool confirmations"
                ))
            }
            None => {
                return Err(format!(
                    "tools: mcp_toolset `{name}` needs default_config.permission_policy (the platform default, always_ask, pauses sessions forever here)"
                ))
            }
        }
    }
    Ok(())
}

fn check_github(gh: &GithubSpec) -> std::result::Result<(), String> {
    if gh.token_env.is_empty() {
        return Err("github.token_env must not be empty".into());
    }
    if let Some(bad) = gh.mount.iter().find(|u| !is_github_repo_url(u)) {
        return Err(format!(
            "github.mount entry `{bad}` must be https://github.com/<owner>/<repo>"
        ));
    }
    Ok(())
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
        assert_eq!((cap("jarvis"), effort("jarvis")), (200, "medium"));
        assert_eq!(reg.agents["jarvis"].default_environment, "jarvis-lab");
        // Read-only token: repo mounts + `gh` on api.github.com, nothing else.
        let gh = reg.agents["jarvis"].github.as_ref().expect("jarvis mounts the org");
        assert_eq!(gh.token_env, "JARVIS_GITHUB_READ_TOKEN");
        assert!(gh.mount.iter().any(|m| m.ends_with("/Jarvis")));
        assert!(gh.mount.iter().all(|m| is_github_repo_url(m)));
        assert_eq!(reg.environments["jarvis-lab"].kind().unwrap(), "cloud");
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

    /// A scratch `agents/` with cloud-default and one file per `(slug, multiagent)`.
    fn scratch_roster_dir(tag: &str, agents: &[(&str, Option<serde_json::Value>)]) -> PathBuf {
        let dir =
            std::env::temp_dir().join(format!("iron-fleet-roster-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("environments")).unwrap();
        std::fs::copy(
            repo_agents_dir().join("environments/cloud-default.json"),
            dir.join("environments/cloud-default.json"),
        )
        .unwrap();
        for (slug, multi) in agents {
            let mut agent = serde_json::json!({
                "name": slug,
                "model": {"id": "claude-sonnet-5"},
                "tools": [{"type": "agent_toolset_20260401"}]
            });
            if let Some(m) = multi {
                agent["multiagent"] = m.clone();
            }
            let file = serde_json::json!({
                "slug": slug,
                "default_environment": "cloud-default",
                "policy": {"max_list_cost_cents": "100"},
                "agent": agent
            });
            std::fs::write(dir.join(format!("{slug}.json")), file.to_string()).unwrap();
        }
        dir
    }

    fn roster(entries: serde_json::Value) -> Option<serde_json::Value> {
        Some(serde_json::json!({"type": "coordinator", "agents": entries}))
    }

    #[test]
    fn a_roster_names_members_by_slug_and_loads() {
        let dir = scratch_roster_dir(
            "ok",
            &[
                ("designer", None),
                ("programmer", None),
                (
                    "director",
                    roster(serde_json::json!([
                        {"type": "agent", "slug": "designer"},
                        {"type": "agent", "slug": "programmer"},
                        {"type": "self"}
                    ])),
                ),
            ],
        );
        let reg = load_dir(&dir).unwrap();
        assert!(reg.agents["director"].agent.multiagent.is_some());
        assert!(reg.agents["designer"].agent.multiagent.is_none());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_rosters_are_load_errors_naming_the_coordinator() {
        let cases: Vec<(&str, Option<serde_json::Value>, &str)> = vec![
            (
                "missing",
                roster(serde_json::json!([{"type": "agent", "slug": "nobody"}])),
                "no file in agents/",
            ),
            (
                "by-id",
                roster(serde_json::json!([{"type": "agent", "id": "agent_01X"}])),
                "not `id`",
            ),
            (
                "self-slug",
                roster(serde_json::json!([{"type": "agent", "slug": "director"}])),
                "lists itself",
            ),
            (
                "twice",
                roster(serde_json::json!([
                    {"type": "agent", "slug": "designer"},
                    {"type": "agent", "slug": "designer"}
                ])),
                "twice",
            ),
            ("empty", roster(serde_json::json!([])), "1 to 20"),
            (
                "not-coordinator",
                Some(serde_json::json!({"type": "worker", "agents": []})),
                "coordinator",
            ),
            (
                "nested",
                roster(serde_json::json!([{"type": "agent", "slug": "lead"}])),
                "one level deep",
            ),
        ];
        for (tag, multi, needle) in cases {
            let dir = scratch_roster_dir(
                tag,
                &[
                    ("designer", None),
                    (
                        "lead",
                        roster(serde_json::json!([{"type": "agent", "slug": "designer"}])),
                    ),
                    ("director", multi),
                ],
            );
            let err = load_dir(&dir).unwrap_err().to_string();
            assert!(err.contains(needle), "{tag}: {err}");
            assert!(err.contains("director.json"), "{tag}: {err}");
            let _ = std::fs::remove_dir_all(&dir);
        }
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
    fn blueweb_client_declares_sandbox_credentials_and_mounts() {
        unsafe {
            std::env::set_var("MCP_FLEET_URL", "https://mcp-fleet.internal.example/mcp");
        }
        let reg = load_dir(&repo_agents_dir()).unwrap();
        let bw = &reg.agents["blueweb-client"];
        let keys: Vec<&str> = bw.credentials.iter().map(|c| c.key()).collect();
        assert_eq!(
            keys,
            [
                "GH_TOKEN",
                "CLOUDFLARE_API_TOKEN",
                "https://api.githubcopilot.com/mcp/"
            ]
        );
        // The GitHub MCP server is how it pushes (GitHub's git endpoint only
        // takes Basic auth, which a vault placeholder can't survive), so the
        // server, its toolset, and its bearer credential must all be present.
        assert_eq!(
            bw.agent.mcp_servers[0]["url"],
            "https://api.githubcopilot.com/mcp/"
        );
        assert!(bw
            .agent
            .tools
            .iter()
            .any(|t| t["type"] == "mcp_toolset" && t["mcp_server_name"] == "github"));
        let gh = bw
            .github
            .as_ref()
            .expect("blueweb-client mounts repositories");
        assert_eq!(gh.token_env, "BLUEWEB_GITHUB_TOKEN");
        assert_eq!(gh.mount, ["https://github.com/Opus-Systems-OS/Iron-Fleet"]);
        assert_eq!(
            bw.default_environment, "blueweb-web",
            "the image with gh + wrangler"
        );
        assert_eq!(
            reg.environments["blueweb-web"].environment.config["packages"]["apt"],
            serde_json::json!(["gh"])
        );
        // Only the agents that need them: no secret or mount leaks to the rest.
        for slug in ["blueweb-ops", "gpu-compute"] {
            assert!(reg.agents[slug].credentials.is_empty(), "{slug}");
            assert!(reg.agents[slug].github.is_none(), "{slug}");
        }
        // jarvis holds exactly one secret — the org read token, as GH_TOKEN on
        // api.github.com — never blueweb's write token.
        let jc = &reg.agents["jarvis"].credentials;
        assert_eq!(jc.len(), 1);
        assert_eq!(jc[0].key(), "GH_TOKEN");
        assert_eq!(jc[0].env_var(), "JARVIS_GITHUB_READ_TOKEN");
        // The token itself is never in a registry file.
        for file in json_files(&repo_agents_dir()).unwrap() {
            let text = std::fs::read_to_string(&file).unwrap();
            assert!(
                !text.contains("ghp_") && !text.contains("github_pat_"),
                "{}",
                file.display()
            );
        }
    }

    #[test]
    fn credentials_and_github_blocks_are_validated() {
        let spec = |name: &str, hosts: &[&str]| CredentialSpec::EnvironmentVariable {
            secret_name: name.into(),
            from_env: "X".into(),
            allowed_hosts: hosts.iter().map(|s| s.to_string()).collect(),
        };
        let none: &[Value] = &[];
        assert!(check_credentials(&[spec("GH_TOKEN", &["api.github.com"])], none).is_ok());
        let err = check_credentials(&[spec("gh-token", &["api.github.com"])], none).unwrap_err();
        assert!(err.contains("environment variable"), "{err}");
        let err = check_credentials(&[spec("A", &["h"]), spec("A", &["h"])], none).unwrap_err();
        assert!(err.contains("duplicate"), "{err}");
        let err = check_credentials(&[spec("A", &[])], none).unwrap_err();
        assert!(err.contains("1-16 allowed_hosts"), "{err}");
        let many: Vec<&str> = vec!["h"; 17];
        assert!(check_credentials(&[spec("A", &many)], none).is_err());
        let err = check_credentials(&[spec("A", &["https://api.github.com/"])], none).unwrap_err();
        assert!(err.contains("bare hostname"), "{err}");

        // An MCP toolset must say how its calls are permitted.
        let ts = |policy: Option<&str>| {
            let mut t = serde_json::json!({"type":"mcp_toolset","mcp_server_name":"github"});
            if let Some(p) = policy {
                t["default_config"] = serde_json::json!({"permission_policy": {"type": p}});
            }
            vec![serde_json::json!({"type":"agent_toolset_20260401"}), t]
        };
        assert!(check_mcp_toolset_policies(&ts(Some("always_allow"))).is_ok());
        assert!(check_mcp_toolset_policies(&ts(Some("auto"))).is_ok());
        let err = check_mcp_toolset_policies(&ts(None)).unwrap_err();
        assert!(err.contains("always_ask"), "{err}");
        let err = check_mcp_toolset_policies(&ts(Some("always_ask"))).unwrap_err();
        assert!(err.contains("would pause"), "{err}");

        // A bearer credential must point at a server the agent actually declares.
        let bearer = CredentialSpec::StaticBearer {
            mcp_server_url: "https://api.githubcopilot.com/mcp/".into(),
            from_env: "X".into(),
        };
        let err = check_credentials(std::slice::from_ref(&bearer), none).unwrap_err();
        assert!(err.contains("matches no agent.mcp_servers"), "{err}");
        let servers = [
            serde_json::json!({"type":"url","name":"github","url":"https://api.githubcopilot.com/mcp/"}),
        ];
        assert!(check_credentials(std::slice::from_ref(&bearer), &servers).is_ok());

        assert!(is_github_repo_url(
            "https://github.com/Opus-Systems-OS/Iron-Fleet"
        ));
        for bad in [
            "https://github.com/Opus-Systems-OS/Iron-Fleet.git",
            "https://github.com/Opus-Systems-OS",
            "https://github.com/Opus-Systems-OS/Iron-Fleet/tree/main",
            "git@github.com:Opus-Systems-OS/Iron-Fleet.git",
            "http://github.com/Opus-Systems-OS/Iron-Fleet",
        ] {
            assert!(!is_github_repo_url(bad), "{bad}");
        }
        let err = check_github(&GithubSpec {
            token_env: "T".into(),
            mount: vec!["https://github.com/Opus-Systems-OS/Iron-Fleet.git".into()],
        })
        .unwrap_err();
        assert!(err.contains("github.mount"), "{err}");

        // Unknown keys in either block are load errors, like everywhere else.
        let bad: std::result::Result<AgentFile, _> = serde_json::from_str(
            r#"{"slug":"x","default_environment":"cloud-default",
                "policy":{"max_list_cost_cents":"5"},
                "credentials":[{"type":"environment_variable","secret_name":"A","from_env":"B","allowed_hosts":["h"],"secret_value":"nope"}],
                "agent":{"name":"x","model":{"id":"claude-opus-5"}}}"#,
        );
        assert!(
            bad.is_err(),
            "a literal secret in the registry file must not parse"
        );
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
