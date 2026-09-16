//! `GET /usage` — the Usage tab — and `GET /usage/export.csv`, the audit
//! trail. Both are built entirely from `session_usage`, the cumulative
//! rollup upserted by the webhook handler; nothing here calls Anthropic. A
//! session with no webhook delivered yet (still running, or the webhook
//! hasn't landed) is simply absent until one arrives — same staleness the
//! table has always had (control-plane/README.md).
//!
//! Both take `?since=` / `?until=` (RFC 3339, or a bare `YYYY-MM-DD` for
//! midnight UTC) as a half-open `[since, until)` window on `observed_at`.

use super::AppState;
use crate::db::{AgentUsageRow, UsageOrder, UsageRow, UsageWindow};
use crate::error::{Error, Result};
use axum::extract::{Query, State};
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;
use time::format_description::well_known::Rfc3339;
use time::{Date, OffsetDateTime, Time};

const RECENT_LIMIT: u32 = 100;

#[derive(Debug, Deserialize)]
pub struct UsageQuery {
    pub since: Option<String>,
    pub until: Option<String>,
}

impl UsageQuery {
    fn window(self) -> Result<UsageWindow> {
        let window = UsageWindow {
            since: parse_bound("since", self.since)?,
            until: parse_bound("until", self.until)?,
        };
        if let (Some(since), Some(until)) = (&window.since, &window.until) {
            if since >= until {
                return Err(Error::InvalidRequest(
                    "since must be earlier than until".into(),
                ));
            }
        }
        Ok(window)
    }
}

/// Normalise a user-supplied bound to the RFC 3339 UTC form `db::now()`
/// writes, so the database can compare it as text.
fn parse_bound(name: &str, raw: Option<String>) -> Result<Option<String>> {
    let Some(raw) = raw else { return Ok(None) };
    let raw = raw.trim();
    let date_only = time::format_description::parse_borrowed::<2>("[year]-[month]-[day]")
        .expect("literal format description");
    let parsed = OffsetDateTime::parse(raw, &Rfc3339)
        .or_else(|_| Date::parse(raw, &date_only).map(|d| d.with_time(Time::MIDNIGHT).assume_utc()))
        .map_err(|_| {
            Error::InvalidRequest(format!(
                "{name}: expected RFC 3339 (2026-09-15T00:00:00Z) or YYYY-MM-DD, got {raw:?}"
            ))
        })?;
    let utc = parsed.to_offset(time::UtcOffset::UTC);
    // `now()` writes no fractional seconds; strip ours so the two forms
    // sort together. `.replace_nanosecond(0)` can't fail for 0.
    let utc = utc.replace_nanosecond(0).unwrap_or(utc);
    utc.format(&Rfc3339)
        .map(Some)
        .map_err(|e| Error::InvalidRequest(format!("{name}: {e}")))
}

#[derive(Debug, Serialize)]
pub struct UsageResponse {
    pub window: UsageWindow,
    pub by_agent: Vec<AgentUsageRow>,
    pub recent: Vec<UsageRow>,
}

pub async fn get(
    State(state): State<AppState>,
    Query(q): Query<UsageQuery>,
) -> Result<Json<UsageResponse>> {
    let window = q.window()?;
    Ok(Json(UsageResponse {
        by_agent: state.db.usage_by_agent(&window)?,
        recent: state
            .db
            .usage_rows(&window, UsageOrder::Newest, Some(RECENT_LIMIT))?,
        window,
    }))
}

/// Every `session_usage` row in the window, oldest first, as CSV. This is
/// the audit trail we own; a session's transcript is behind its Console
/// link, never here (CLAUDE.md: no session store).
pub async fn export_csv(
    State(state): State<AppState>,
    Query(q): Query<UsageQuery>,
) -> Result<Response> {
    let window = q.window()?;
    let rows = state.db.usage_rows(&window, UsageOrder::Oldest, None)?;
    // Bounds go in the filename compacted (`20260915T000000Z`): colons are
    // not filename-safe on macOS or Windows.
    let stamp = |b: &Option<String>, default: &str| {
        b.as_deref()
            .map_or_else(|| default.to_owned(), |s| s.replace([':', '-'], ""))
    };
    let filename = format!(
        "session_usage-{}-{}.csv",
        stamp(&window.since, "start"),
        stamp(&window.until, "now"),
    );
    Ok((
        [
            (header::CONTENT_TYPE, "text/csv; charset=utf-8".to_owned()),
            (
                header::CONTENT_DISPOSITION,
                format!("attachment; filename=\"{filename}\""),
            ),
        ],
        to_csv(&rows),
    )
        .into_response())
}

const CSV_HEADER: &str = "session_id,agent_slug,environment_slug,list_cost_cents,input_tokens,\
                          output_tokens,active_seconds,budget_reached,last_event_type,observed_at";

/// RFC 4180 with `\n` line endings. Ten columns of ids and numbers don't
/// justify a crate; `quote` covers the slugs and event types anyway.
fn to_csv(rows: &[UsageRow]) -> String {
    let mut out = String::with_capacity(rows.len() * 160);
    out.push_str(CSV_HEADER);
    out.push('\n');
    for r in rows {
        let opt = |s: Option<String>| s.unwrap_or_default();
        let _ = writeln!(
            out,
            "{},{},{},{},{},{},{},{},{},{}",
            quote(&r.session_id),
            quote(&r.agent_slug),
            quote(&opt(r.environment_slug.clone())),
            opt(r.list_cost_cents.map(|c| c.to_string())),
            opt(r.input_tokens.map(|v| v.to_string())),
            opt(r.output_tokens.map(|v| v.to_string())),
            opt(r.active_seconds.map(|v| v.to_string())),
            u8::from(r.budget_reached),
            quote(&r.last_event_type),
            quote(&r.observed_at),
        );
    }
    out
}

fn quote(field: &str) -> String {
    if field.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", field.replace('"', "\"\""))
    } else {
        field.to_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn bounds_accept_rfc3339_and_bare_dates_and_normalise_to_utc() {
        assert_eq!(
            parse_bound("since", Some("2026-09-15".into()))
                .unwrap()
                .as_deref(),
            Some("2026-09-15T00:00:00Z")
        );
        assert_eq!(
            parse_bound("since", Some("2026-09-15T01:30:00.5-04:00".into()))
                .unwrap()
                .as_deref(),
            Some("2026-09-15T05:30:00Z")
        );
        assert_eq!(parse_bound("since", None).unwrap(), None);
        let err = parse_bound("until", Some("yesterday".into())).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(m) if m.starts_with("until:")));
    }

    #[test]
    fn window_rejects_inverted_bounds() {
        let q = UsageQuery {
            since: Some("2026-09-16".into()),
            until: Some("2026-09-15".into()),
        };
        assert!(matches!(q.window(), Err(Error::InvalidRequest(_))));
        let q = UsageQuery {
            since: Some("2026-09-15".into()),
            until: Some("2026-09-15".into()),
        };
        assert!(
            matches!(q.window(), Err(Error::InvalidRequest(_))),
            "empty window"
        );
    }

    fn row(session: &str, event: &str) -> UsageRow {
        UsageRow {
            session_id: session.into(),
            agent_slug: "jarvis".into(),
            environment_slug: None,
            list_cost_cents: Some("5".parse().unwrap()),
            input_tokens: Some(10),
            output_tokens: None,
            active_seconds: Some(1.5),
            budget_reached: true,
            last_event_type: event.into(),
            observed_at: "2026-09-15T00:00:00Z".into(),
        }
    }

    #[test]
    fn csv_has_header_and_empty_fields_for_nulls() {
        let csv = to_csv(&[row("sesn_1", "session.status_idled")]);
        let mut lines = csv.lines();
        assert_eq!(lines.next().unwrap(), CSV_HEADER);
        assert_eq!(
            lines.next().unwrap(),
            "sesn_1,jarvis,,5,10,,1.5,1,session.status_idled,2026-09-15T00:00:00Z"
        );
        assert!(lines.next().is_none());
    }

    #[test]
    fn csv_quotes_fields_that_need_it() {
        let csv = to_csv(&[row("sesn_1", "odd,\"event\"")]);
        assert!(csv.ends_with(",\"odd,\"\"event\"\"\",2026-09-15T00:00:00Z\n"));
        assert_eq!(quote("plain"), "plain");
    }
}
