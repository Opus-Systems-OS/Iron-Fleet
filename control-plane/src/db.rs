//! SQLite: agent registry, budget policy, usage rollups. Nothing else lives
//! here — session state belongs to Anthropic and is always read live.

use crate::error::Result;
use crate::money::Cents;
use rusqlite::{params, Connection, OptionalExtension};
use std::path::Path;
use std::sync::{Arc, Mutex};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS environments (
  slug          TEXT PRIMARY KEY,
  kind          TEXT NOT NULL,            -- "cloud" | "self_hosted"
  anthropic_id  TEXT,                     -- NULL until provisioned by sync
  synced_at     TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS agents (
  slug                 TEXT PRIMARY KEY,
  anthropic_id         TEXT NOT NULL,
  anthropic_version    INTEGER NOT NULL,
  definition_sha256    TEXT NOT NULL,
  max_list_cost_cents  TEXT NOT NULL,     -- stored as the string it is on the wire
  effort               TEXT NOT NULL,
  default_environment  TEXT NOT NULL REFERENCES environments(slug),
  synced_at            TEXT NOT NULL
);
-- Custom skills uploaded from agents/skills/<name>/, one row per directory.
-- version_id is what agent definitions pin to (see registry::sync::resolve_skills).
CREATE TABLE IF NOT EXISTS skills (
  name            TEXT PRIMARY KEY,
  anthropic_id    TEXT NOT NULL,
  version_id      TEXT NOT NULL,
  content_sha256  TEXT NOT NULL,
  synced_at       TEXT NOT NULL
);
-- One vault per agent that declares `credentials` in its registry file;
-- attached only to that agent's sessions (unlike mcp_fleet_vault, which
-- rides on every session).
CREATE TABLE IF NOT EXISTS agent_vaults (
  slug       TEXT PRIMARY KEY REFERENCES agents(slug),
  vault_id   TEXT NOT NULL,
  synced_at  TEXT NOT NULL
);
-- The `github` block of agents/<slug>.json, refreshed every sync like the
-- policy columns: which env var holds the mount token (never the token) and
-- which repos every session of this agent mounts (JSON array of URLs).
CREATE TABLE IF NOT EXISTS agent_github (
  slug        TEXT PRIMARY KEY REFERENCES agents(slug),
  token_env   TEXT NOT NULL,
  mounts      TEXT NOT NULL,
  synced_at   TEXT NOT NULL
);
-- config_sha256 covers the secret value and its allowed_hosts, so rotation
-- is detected without ever storing the secret itself.
CREATE TABLE IF NOT EXISTS agent_credentials (
  slug           TEXT NOT NULL,
  secret_name    TEXT NOT NULL,
  credential_id  TEXT NOT NULL,
  config_sha256  TEXT NOT NULL,
  synced_at      TEXT NOT NULL,
  PRIMARY KEY (slug, secret_name)
);
CREATE TABLE IF NOT EXISTS session_usage (
  session_id        TEXT PRIMARY KEY,
  agent_slug        TEXT NOT NULL,
  environment_slug  TEXT,
  list_cost_cents   TEXT,
  input_tokens      INTEGER,
  output_tokens     INTEGER,
  active_seconds    REAL,
  budget_reached    INTEGER NOT NULL DEFAULT 0,
  last_event_type   TEXT,
  observed_at       TEXT NOT NULL
);
-- Singleton: the one vault holding the mcp-fleet static_bearer credential,
-- provisioned once (unlike agents/environments, vault creation has no
-- dedupe key of its own — this row is what makes it idempotent).
CREATE TABLE IF NOT EXISTS mcp_fleet_vault (
  id             INTEGER PRIMARY KEY CHECK (id = 1),
  vault_id       TEXT NOT NULL,
  credential_id  TEXT NOT NULL,
  mcp_server_url TEXT NOT NULL,
  synced_at      TEXT NOT NULL
);
"#;

#[derive(Clone)]
pub struct Db {
    conn: Arc<Mutex<Connection>>,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentRow {
    pub slug: String,
    pub agent_id: String,
    pub agent_version: u32,
    #[serde(skip)]
    pub definition_sha256: String,
    pub max_list_cost_cents: Cents,
    pub effort: String,
    pub default_environment: String,
    pub synced_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct EnvironmentRow {
    pub slug: String,
    pub kind: String,
    pub environment_id: Option<String>,
    pub synced_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct UsageRow {
    pub session_id: String,
    pub agent_slug: String,
    pub environment_slug: Option<String>,
    pub list_cost_cents: Option<Cents>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub active_seconds: Option<f64>,
    pub budget_reached: bool,
    pub last_event_type: String,
    pub observed_at: String,
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct AgentUsageRow {
    pub agent_slug: String,
    pub session_count: u64,
    pub total_list_cost_cents: u64,
    pub budget_reached_count: u64,
}

#[derive(Debug, Clone)]
pub struct SkillRow {
    pub name: String,
    pub skill_id: String,
    pub version_id: String,
    pub content_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentGithubRow {
    pub token_env: String,
    pub mounts: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AgentCredentialRow {
    pub credential_id: String,
    pub config_sha256: String,
}

#[derive(Debug, Clone)]
pub struct McpFleetVaultRow {
    pub vault_id: String,
    pub mcp_server_url: String,
}

#[derive(Debug, Clone)]
pub struct UsageSnapshot {
    pub session_id: String,
    pub agent_slug: String,
    pub environment_slug: Option<String>,
    pub list_cost_cents: Option<Cents>,
    pub input_tokens: Option<u64>,
    pub output_tokens: Option<u64>,
    pub active_seconds: Option<f64>,
    pub budget_reached: bool,
    pub last_event_type: String,
}

impl Db {
    pub fn open(path: &Path) -> Result<Self> {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    rusqlite::Error::InvalidPath(std::path::PathBuf::from(format!(
                        "{}: {e}",
                        parent.display()
                    )))
                })?;
            }
        }
        let conn = Connection::open(path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    #[cfg(test)]
    pub fn in_memory() -> Result<Self> {
        let conn = Connection::open_in_memory()?;
        conn.execute_batch("PRAGMA foreign_keys=ON;")?;
        conn.execute_batch(SCHEMA)?;
        Ok(Db {
            conn: Arc::new(Mutex::new(conn)),
        })
    }

    fn with<T>(&self, f: impl FnOnce(&Connection) -> rusqlite::Result<T>) -> Result<T> {
        let conn = self.conn.lock().expect("sqlite mutex poisoned");
        Ok(f(&conn)?)
    }

    // ---- environments

    pub fn upsert_environment(
        &self,
        slug: &str,
        kind: &str,
        environment_id: Option<&str>,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO environments (slug, kind, anthropic_id, synced_at)
                 VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(slug) DO UPDATE SET
                   kind = excluded.kind,
                   anthropic_id = COALESCE(excluded.anthropic_id, environments.anthropic_id),
                   synced_at = excluded.synced_at",
                params![slug, kind, environment_id, now()],
            )?;
            Ok(())
        })
    }

    pub fn environment(&self, slug: &str) -> Result<Option<EnvironmentRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT slug, kind, anthropic_id, synced_at FROM environments WHERE slug = ?1",
                [slug],
                |r| {
                    Ok(EnvironmentRow {
                        slug: r.get(0)?,
                        kind: r.get(1)?,
                        environment_id: r.get(2)?,
                        synced_at: r.get(3)?,
                    })
                },
            )
            .optional()
        })
    }

    // ---- agents

    pub fn upsert_agent(&self, row: &AgentRow) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO agents (slug, anthropic_id, anthropic_version, definition_sha256,
                                     max_list_cost_cents, effort, default_environment, synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)
                 ON CONFLICT(slug) DO UPDATE SET
                   anthropic_id = excluded.anthropic_id,
                   anthropic_version = excluded.anthropic_version,
                   definition_sha256 = excluded.definition_sha256,
                   max_list_cost_cents = excluded.max_list_cost_cents,
                   effort = excluded.effort,
                   default_environment = excluded.default_environment,
                   synced_at = excluded.synced_at",
                params![
                    row.slug,
                    row.agent_id,
                    row.agent_version,
                    row.definition_sha256,
                    row.max_list_cost_cents.to_string(),
                    row.effort,
                    row.default_environment,
                    now(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn agent(&self, slug: &str) -> Result<Option<AgentRow>> {
        self.with(|c| {
            c.query_row(
                &format!("{AGENT_SELECT} WHERE slug = ?1"),
                [slug],
                read_agent_row,
            )
            .optional()
        })
    }

    pub fn agents(&self) -> Result<Vec<AgentRow>> {
        self.with(|c| {
            let mut stmt = c.prepare(&format!("{AGENT_SELECT} ORDER BY slug"))?;
            let rows = stmt.query_map([], read_agent_row)?;
            rows.collect()
        })
    }

    // ---- skills

    pub fn upsert_skill(&self, row: &SkillRow) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO skills (name, anthropic_id, version_id, content_sha256, synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(name) DO UPDATE SET
                   anthropic_id = excluded.anthropic_id,
                   version_id = excluded.version_id,
                   content_sha256 = excluded.content_sha256,
                   synced_at = excluded.synced_at",
                params![
                    row.name,
                    row.skill_id,
                    row.version_id,
                    row.content_sha256,
                    now()
                ],
            )?;
            Ok(())
        })
    }

    pub fn skill(&self, name: &str) -> Result<Option<SkillRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT name, anthropic_id, version_id, content_sha256 FROM skills WHERE name = ?1",
                [name],
                |r| {
                    Ok(SkillRow {
                        name: r.get(0)?,
                        skill_id: r.get(1)?,
                        version_id: r.get(2)?,
                        content_sha256: r.get(3)?,
                    })
                },
            )
            .optional()
        })
    }

    // ---- per-agent github access

    pub fn agent_github(&self, slug: &str) -> Result<Option<AgentGithubRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT token_env, mounts FROM agent_github WHERE slug = ?1",
                [slug],
                |r| {
                    let mounts: String = r.get(1)?;
                    Ok(AgentGithubRow {
                        token_env: r.get(0)?,
                        mounts: serde_json::from_str(&mounts).unwrap_or_default(),
                    })
                },
            )
            .optional()
        })
    }

    /// `None` removes the row: an agent whose file dropped its `github` block
    /// stops mounting anything on the next sync.
    pub fn set_agent_github(&self, slug: &str, row: Option<&AgentGithubRow>) -> Result<()> {
        self.with(|c| {
            match row {
                Some(row) => {
                    let mounts =
                        serde_json::to_string(&row.mounts).expect("Vec<String> serializes");
                    c.execute(
                        "INSERT INTO agent_github (slug, token_env, mounts, synced_at)
                         VALUES (?1, ?2, ?3, ?4)
                         ON CONFLICT(slug) DO UPDATE SET
                           token_env = excluded.token_env,
                           mounts = excluded.mounts,
                           synced_at = excluded.synced_at",
                        params![slug, row.token_env, mounts, now()],
                    )?;
                }
                None => {
                    c.execute("DELETE FROM agent_github WHERE slug = ?1", [slug])?;
                }
            }
            Ok(())
        })
    }

    // ---- per-agent vaults

    pub fn agent_vault(&self, slug: &str) -> Result<Option<String>> {
        self.with(|c| {
            c.query_row(
                "SELECT vault_id FROM agent_vaults WHERE slug = ?1",
                [slug],
                |r| r.get(0),
            )
            .optional()
        })
    }

    pub fn set_agent_vault(&self, slug: &str, vault_id: &str) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO agent_vaults (slug, vault_id, synced_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(slug) DO UPDATE SET
                   vault_id = excluded.vault_id,
                   synced_at = excluded.synced_at",
                params![slug, vault_id, now()],
            )?;
            Ok(())
        })
    }

    pub fn agent_credential(
        &self,
        slug: &str,
        secret_name: &str,
    ) -> Result<Option<AgentCredentialRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT credential_id, config_sha256 FROM agent_credentials
                 WHERE slug = ?1 AND secret_name = ?2",
                [slug, secret_name],
                |r| {
                    Ok(AgentCredentialRow {
                        credential_id: r.get(0)?,
                        config_sha256: r.get(1)?,
                    })
                },
            )
            .optional()
        })
    }

    pub fn upsert_agent_credential(
        &self,
        slug: &str,
        secret_name: &str,
        row: &AgentCredentialRow,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO agent_credentials (slug, secret_name, credential_id, config_sha256, synced_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)
                 ON CONFLICT(slug, secret_name) DO UPDATE SET
                   credential_id = excluded.credential_id,
                   config_sha256 = excluded.config_sha256,
                   synced_at = excluded.synced_at",
                params![slug, secret_name, row.credential_id, row.config_sha256, now()],
            )?;
            Ok(())
        })
    }

    // ---- usage rollups

    pub fn upsert_usage(&self, s: &UsageSnapshot) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO session_usage (session_id, agent_slug, environment_slug, list_cost_cents,
                                            input_tokens, output_tokens, active_seconds,
                                            budget_reached, last_event_type, observed_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)
                 ON CONFLICT(session_id) DO UPDATE SET
                   agent_slug = excluded.agent_slug,
                   environment_slug = COALESCE(excluded.environment_slug, session_usage.environment_slug),
                   list_cost_cents = COALESCE(excluded.list_cost_cents, session_usage.list_cost_cents),
                   input_tokens = COALESCE(excluded.input_tokens, session_usage.input_tokens),
                   output_tokens = COALESCE(excluded.output_tokens, session_usage.output_tokens),
                   active_seconds = COALESCE(excluded.active_seconds, session_usage.active_seconds),
                   budget_reached = MAX(excluded.budget_reached, session_usage.budget_reached),
                   last_event_type = excluded.last_event_type,
                   observed_at = excluded.observed_at",
                params![
                    s.session_id,
                    s.agent_slug,
                    s.environment_slug,
                    s.list_cost_cents.map(|c| c.to_string()),
                    s.input_tokens.map(|v| v as i64),
                    s.output_tokens.map(|v| v as i64),
                    s.active_seconds,
                    s.budget_reached as i64,
                    s.last_event_type,
                    now(),
                ],
            )?;
            Ok(())
        })
    }

    /// Most recently observed rollups first — what the Usage tab's session
    /// list shows.
    pub fn usage_rows(&self, limit: u32) -> Result<Vec<UsageRow>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT session_id, agent_slug, environment_slug, list_cost_cents, input_tokens,
                        output_tokens, active_seconds, budget_reached, last_event_type, observed_at
                 FROM session_usage ORDER BY observed_at DESC, rowid DESC LIMIT ?1",
            )?;
            let rows = stmt.query_map([limit], read_usage_row)?;
            rows.collect()
        })
    }

    /// Totals per agent, for the Usage tab's summary. `total_list_cost_cents`
    /// sums whatever's been observed so far — sessions never reported by a
    /// webhook yet (still running, or the webhook hasn't landed) aren't
    /// counted until they are, same as the rest of this table.
    pub fn usage_by_agent(&self) -> Result<Vec<AgentUsageRow>> {
        self.with(|c| {
            let mut stmt = c.prepare(
                "SELECT agent_slug, COUNT(*),
                        COALESCE(SUM(CAST(list_cost_cents AS INTEGER)), 0),
                        SUM(budget_reached)
                 FROM session_usage GROUP BY agent_slug ORDER BY agent_slug",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(AgentUsageRow {
                    agent_slug: r.get(0)?,
                    session_count: r.get::<_, i64>(1)? as u64,
                    total_list_cost_cents: r.get::<_, i64>(2)? as u64,
                    budget_reached_count: r.get::<_, i64>(3)? as u64,
                })
            })?;
            rows.collect()
        })
    }

    // ---- mcp-fleet vault

    /// `credential_id` and `synced_at` are stored (for a human inspecting the
    /// database directly) but not read back here — nothing needs them yet.
    pub fn mcp_fleet_vault(&self) -> Result<Option<McpFleetVaultRow>> {
        self.with(|c| {
            c.query_row(
                "SELECT vault_id, mcp_server_url FROM mcp_fleet_vault WHERE id = 1",
                [],
                |r| {
                    Ok(McpFleetVaultRow {
                        vault_id: r.get(0)?,
                        mcp_server_url: r.get(1)?,
                    })
                },
            )
            .optional()
        })
    }

    pub fn set_mcp_fleet_vault(
        &self,
        vault_id: &str,
        credential_id: &str,
        mcp_server_url: &str,
    ) -> Result<()> {
        self.with(|c| {
            c.execute(
                "INSERT INTO mcp_fleet_vault (id, vault_id, credential_id, mcp_server_url, synced_at)
                 VALUES (1, ?1, ?2, ?3, ?4)
                 ON CONFLICT(id) DO UPDATE SET
                   vault_id = excluded.vault_id,
                   credential_id = excluded.credential_id,
                   mcp_server_url = excluded.mcp_server_url,
                   synced_at = excluded.synced_at",
                params![vault_id, credential_id, mcp_server_url, now()],
            )?;
            Ok(())
        })
    }
}

const AGENT_SELECT: &str = "SELECT slug, anthropic_id, anthropic_version, definition_sha256,
                                   max_list_cost_cents, effort, default_environment, synced_at
                            FROM agents";

fn read_agent_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<AgentRow> {
    let cents: String = r.get(4)?;
    let max_list_cost_cents = cents.parse::<Cents>().map_err(|e| {
        rusqlite::Error::FromSqlConversionFailure(4, rusqlite::types::Type::Text, Box::new(e))
    })?;
    Ok(AgentRow {
        slug: r.get(0)?,
        agent_id: r.get(1)?,
        agent_version: r.get::<_, i64>(2)? as u32,
        definition_sha256: r.get(3)?,
        max_list_cost_cents,
        effort: r.get(5)?,
        default_environment: r.get(6)?,
        synced_at: r.get(7)?,
    })
}

fn read_usage_row(r: &rusqlite::Row<'_>) -> rusqlite::Result<UsageRow> {
    let list_cost_cents = r
        .get::<_, Option<String>>(3)?
        .map(|s| {
            s.parse::<Cents>().map_err(|e| {
                rusqlite::Error::FromSqlConversionFailure(
                    3,
                    rusqlite::types::Type::Text,
                    Box::new(e),
                )
            })
        })
        .transpose()?;
    Ok(UsageRow {
        session_id: r.get(0)?,
        agent_slug: r.get(1)?,
        environment_slug: r.get(2)?,
        list_cost_cents,
        input_tokens: r.get::<_, Option<i64>>(4)?.map(|v| v as u64),
        output_tokens: r.get::<_, Option<i64>>(5)?.map(|v| v as u64),
        active_seconds: r.get(6)?,
        budget_reached: r.get::<_, i64>(7)? != 0,
        last_event_type: r.get(8)?,
        observed_at: r.get(9)?,
    })
}

pub fn now() -> String {
    time::OffsetDateTime::now_utc()
        .format(&time::format_description::well_known::Rfc3339)
        .unwrap_or_else(|_| "1970-01-01T00:00:00Z".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_round_trip() {
        let db = Db::in_memory().unwrap();
        db.upsert_environment("cloud-default", "cloud", Some("env_1"))
            .unwrap();
        db.upsert_environment("rig-gpu", "self_hosted", None)
            .unwrap();
        db.upsert_agent(&AgentRow {
            slug: "jarvis".into(),
            agent_id: "agent_1".into(),
            agent_version: 1,
            definition_sha256: "abc".into(),
            max_list_cost_cents: "50".parse().unwrap(),
            effort: "low".into(),
            default_environment: "cloud-default".into(),
            synced_at: String::new(),
        })
        .unwrap();

        let a = db.agent("jarvis").unwrap().unwrap();
        assert_eq!(a.agent_id, "agent_1");
        assert_eq!(a.max_list_cost_cents.get(), 50);
        assert!(db.agent("nope").unwrap().is_none());
        assert_eq!(
            db.environment("rig-gpu").unwrap().unwrap().environment_id,
            None
        );

        // Re-upsert with a NULL id must not clobber a provisioned one.
        db.upsert_environment("cloud-default", "cloud", None)
            .unwrap();
        assert_eq!(
            db.environment("cloud-default")
                .unwrap()
                .unwrap()
                .environment_id
                .as_deref(),
            Some("env_1")
        );
    }

    #[test]
    fn usage_rollup_is_monotonic_on_budget_reached() {
        let db = Db::in_memory().unwrap();
        let snap = |event: &str, reached: bool| UsageSnapshot {
            session_id: "sesn_1".into(),
            agent_slug: "jarvis".into(),
            environment_slug: Some("cloud-default".into()),
            list_cost_cents: Some("53".parse().unwrap()),
            input_tokens: Some(10),
            output_tokens: Some(20),
            active_seconds: Some(4.5),
            budget_reached: reached,
            last_event_type: event.into(),
        };
        db.upsert_usage(&snap("session.budget_reached", true))
            .unwrap();
        db.upsert_usage(&snap("session.status_idled", false))
            .unwrap();
        let reached: i64 = db
            .with(|c| c.query_row("SELECT budget_reached FROM session_usage", [], |r| r.get(0)))
            .unwrap();
        assert_eq!(reached, 1);
    }

    #[test]
    fn usage_reads_aggregate_and_list_correctly() {
        let db = Db::in_memory().unwrap();
        let snap = |session: &str, agent: &str, cents: &str, reached: bool| UsageSnapshot {
            session_id: session.into(),
            agent_slug: agent.into(),
            environment_slug: Some("cloud-default".into()),
            list_cost_cents: Some(cents.parse().unwrap()),
            input_tokens: Some(10),
            output_tokens: Some(20),
            active_seconds: Some(1.5),
            budget_reached: reached,
            last_event_type: "session.status_idled".into(),
        };
        db.upsert_usage(&snap("sesn_1", "jarvis", "5", false))
            .unwrap();
        db.upsert_usage(&snap("sesn_2", "jarvis", "50", true))
            .unwrap();
        db.upsert_usage(&snap("sesn_3", "blueweb-client", "300", false))
            .unwrap();

        let by_agent = db.usage_by_agent().unwrap();
        assert_eq!(by_agent.len(), 2);
        let jarvis = by_agent.iter().find(|a| a.agent_slug == "jarvis").unwrap();
        assert_eq!(jarvis.session_count, 2);
        assert_eq!(jarvis.total_list_cost_cents, 55);
        assert_eq!(jarvis.budget_reached_count, 1);
        let blueweb = by_agent
            .iter()
            .find(|a| a.agent_slug == "blueweb-client")
            .unwrap();
        assert_eq!(blueweb.total_list_cost_cents, 300);
        assert_eq!(blueweb.budget_reached_count, 0);

        let recent = db.usage_rows(2).unwrap();
        assert_eq!(recent.len(), 2, "limit is respected");
        assert_eq!(
            recent[0].session_id, "sesn_3",
            "most recently observed first"
        );
        assert_eq!(recent[0].list_cost_cents.unwrap().get(), 300);
    }

    #[test]
    fn skill_round_trips_and_upserts() {
        let db = Db::in_memory().unwrap();
        assert!(db.skill("blueweb-customer-site").unwrap().is_none());
        let row = SkillRow {
            name: "blueweb-customer-site".into(),
            skill_id: "skill_1".into(),
            version_id: "skillver_1".into(),
            content_sha256: "aa".into(),
        };
        db.upsert_skill(&row).unwrap();
        let got = db.skill("blueweb-customer-site").unwrap().unwrap();
        assert_eq!(
            (got.skill_id.as_str(), got.version_id.as_str()),
            ("skill_1", "skillver_1")
        );
        db.upsert_skill(&SkillRow {
            version_id: "skillver_2".into(),
            content_sha256: "bb".into(),
            ..row
        })
        .unwrap();
        let got = db.skill("blueweb-customer-site").unwrap().unwrap();
        assert_eq!(
            (got.version_id.as_str(), got.content_sha256.as_str()),
            ("skillver_2", "bb")
        );
    }

    #[test]
    fn agent_vault_and_credentials_round_trip() {
        let db = Db::in_memory().unwrap();
        // agent_vaults references agents(slug); seed the agent first.
        db.upsert_environment("cloud-default", "cloud", Some("env_1"))
            .unwrap();
        db.upsert_agent(&AgentRow {
            slug: "blueweb-client".into(),
            agent_id: "agent_1".into(),
            agent_version: 1,
            definition_sha256: "x".into(),
            max_list_cost_cents: "1000".parse().unwrap(),
            effort: "high".into(),
            default_environment: "cloud-default".into(),
            synced_at: String::new(),
        })
        .unwrap();

        assert!(db.agent_github("blueweb-client").unwrap().is_none());
        let gh = AgentGithubRow {
            token_env: "BLUEWEB_GITHUB_TOKEN".into(),
            mounts: vec!["https://github.com/Opus1247/Iron-Fleet".into()],
        };
        db.set_agent_github("blueweb-client", Some(&gh)).unwrap();
        assert_eq!(
            db.agent_github("blueweb-client").unwrap().as_ref(),
            Some(&gh)
        );
        db.set_agent_github("blueweb-client", None).unwrap();
        assert!(db.agent_github("blueweb-client").unwrap().is_none());

        assert!(db.agent_vault("blueweb-client").unwrap().is_none());
        db.set_agent_vault("blueweb-client", "vlt_1").unwrap();
        assert_eq!(
            db.agent_vault("blueweb-client").unwrap().as_deref(),
            Some("vlt_1")
        );

        assert!(db
            .agent_credential("blueweb-client", "GH_TOKEN")
            .unwrap()
            .is_none());
        let row = AgentCredentialRow {
            credential_id: "vcrd_1".into(),
            config_sha256: "aa".into(),
        };
        db.upsert_agent_credential("blueweb-client", "GH_TOKEN", &row)
            .unwrap();
        db.upsert_agent_credential(
            "blueweb-client",
            "GH_TOKEN",
            &AgentCredentialRow {
                config_sha256: "bb".into(),
                ..row
            },
        )
        .unwrap();
        let got = db
            .agent_credential("blueweb-client", "GH_TOKEN")
            .unwrap()
            .unwrap();
        assert_eq!(
            (got.credential_id.as_str(), got.config_sha256.as_str()),
            ("vcrd_1", "bb")
        );
        assert!(db.agent_credential("jarvis", "GH_TOKEN").unwrap().is_none());
    }

    #[test]
    fn mcp_fleet_vault_round_trips_and_upserts() {
        let db = Db::in_memory().unwrap();
        assert!(db.mcp_fleet_vault().unwrap().is_none());

        db.set_mcp_fleet_vault("vlt_1", "vcrd_1", "https://mcp-fleet.example/mcp")
            .unwrap();
        let row = db.mcp_fleet_vault().unwrap().unwrap();
        assert_eq!(row.vault_id, "vlt_1");
        assert_eq!(row.mcp_server_url, "https://mcp-fleet.example/mcp");

        // A second call (e.g. next boot) overwrites the single row rather than
        // erroring or duplicating it.
        db.set_mcp_fleet_vault("vlt_2", "vcrd_2", "https://mcp-fleet.example/mcp")
            .unwrap();
        assert_eq!(db.mcp_fleet_vault().unwrap().unwrap().vault_id, "vlt_2");
    }
}
