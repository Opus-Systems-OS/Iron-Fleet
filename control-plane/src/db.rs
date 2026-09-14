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
}
