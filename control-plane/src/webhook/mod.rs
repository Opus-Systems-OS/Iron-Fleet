//! `POST /webhooks/managed-agents`: verify, dedupe, switch on `data.type`,
//! fetch the session live, log, and record a usage rollup. No notification
//! delivery in stage 1 — the log lines are the deliverable.

pub mod signature;

use crate::db::UsageSnapshot;
use crate::error::{Error, Result};
use crate::http::AppState;
use crate::registry::SLUG_METADATA_KEY;
use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use serde::Deserialize;
use signature::Headers;
use std::collections::{HashSet, VecDeque};
use std::sync::Mutex;

/// Webhook `data.type` values this endpoint acts on. These are the *webhook*
/// names — the SSE stream uses `session.status_idle` (no `d`); don't mix them.
pub const SESSION_STATUS_IDLED: &str = "session.status_idled";
pub const SESSION_BUDGET_REACHED: &str = "session.budget_reached";

#[derive(Debug, Deserialize)]
pub struct Envelope {
    pub id: String,
    pub created_at: String,
    pub data: EventData,
}

#[derive(Debug, Deserialize)]
pub struct EventData {
    #[serde(rename = "type")]
    pub kind: String,
    pub id: String,
    #[serde(default)]
    pub organization_id: Option<String>,
    #[serde(default)]
    pub workspace_id: Option<String>,
}

/// Bounded in-memory set of seen `event.id`s. Retries are at most three
/// attempts within a few minutes, so losing this on restart is harmless, and it
/// keeps delivery bookkeeping out of SQLite.
pub struct SeenEvents {
    inner: Mutex<(HashSet<String>, VecDeque<String>)>,
    capacity: usize,
}

impl SeenEvents {
    pub fn new(capacity: usize) -> Self {
        SeenEvents {
            inner: Mutex::new((HashSet::new(), VecDeque::new())),
            capacity,
        }
    }

    /// Returns `true` if this id was already seen.
    pub fn check_and_insert(&self, id: &str) -> bool {
        let mut g = self.inner.lock().expect("seen-events mutex poisoned");
        let (set, order) = &mut *g;
        if set.contains(id) {
            return true;
        }
        set.insert(id.to_owned());
        order.push_back(id.to_owned());
        while order.len() > self.capacity {
            if let Some(old) = order.pop_front() {
                set.remove(&old);
            }
        }
        false
    }
}

pub async fn handle(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode> {
    let hdr = |name: &str| {
        headers
            .get(name)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
    };
    let webhook_id = hdr("webhook-id");
    signature::verify(
        &state.signing_key,
        Headers {
            id: webhook_id,
            timestamp: hdr("webhook-timestamp"),
            signature: hdr("webhook-signature"),
        },
        &body,
        unix_now(),
    )
    .map_err(Error::WebhookSignature)?;

    let event: Envelope = serde_json::from_slice(&body)
        .map_err(|e| Error::InvalidRequest(format!("webhook body: {e}")))?;
    tracing::debug!(
        event_id = %event.id,
        kind = %event.data.kind,
        resource = %event.data.id,
        organization_id = event.data.organization_id.as_deref().unwrap_or("-"),
        workspace_id = event.data.workspace_id.as_deref().unwrap_or("-"),
        "webhook delivery verified"
    );

    if state.seen_events.check_and_insert(&event.id) {
        tracing::info!(event_id = %event.id, kind = %event.data.kind, "duplicate webhook delivery ignored");
        return Ok(StatusCode::NO_CONTENT);
    }

    match event.data.kind.as_str() {
        SESSION_STATUS_IDLED => on_session_event(&state, &event, false).await?,
        SESSION_BUDGET_REACHED => on_session_event(&state, &event, true).await?,
        other => {
            tracing::debug!(event_id = %event.id, kind = other, resource = %event.data.id, "unhandled webhook type");
        }
    }
    Ok(StatusCode::NO_CONTENT)
}

/// Shared path for the two session events: fetch live, log, roll up usage.
/// An upstream failure propagates as 5xx so Anthropic retries the delivery.
async fn on_session_event(state: &AppState, event: &Envelope, budget_reached: bool) -> Result<()> {
    let session = state.api.get_session(&event.data.id).await?;

    let slug = session
        .metadata
        .get("iron_fleet_agent")
        .or_else(|| session.metadata.get(SLUG_METADATA_KEY))
        .cloned()
        .unwrap_or_else(|| "<not started by control plane>".to_owned());
    let environment = session.metadata.get("iron_fleet_environment").cloned();
    let usage = session.usage.as_ref();
    let list_cost = usage.and_then(|u| u.list_cost).map(|m| m.amount);
    let cap = session.budget.map(|b| b.max_list_cost.amount);
    let fmt_cents =
        |c: Option<crate::money::Cents>| c.map(|c| c.to_string()).unwrap_or_else(|| "-".into());

    if budget_reached {
        tracing::warn!(
            event = SESSION_BUDGET_REACHED,
            slug = %slug,
            session = %session.id,
            status = %session.status,
            list_cost_cents = %fmt_cents(list_cost),
            cap_cents = %fmt_cents(cap),
            occurred_at = %event.created_at,
            "BUDGET REACHED — session paused; only a budget change or removal resumes it"
        );
    } else {
        tracing::info!(
            event = SESSION_STATUS_IDLED,
            slug = %slug,
            session = %session.id,
            status = %session.status,
            list_cost_cents = %fmt_cents(list_cost),
            cap_cents = %fmt_cents(cap),
            occurred_at = %event.created_at,
            "session idled — awaiting input"
        );
    }

    state.db.upsert_usage(&UsageSnapshot {
        session_id: session.id.clone(),
        agent_slug: slug,
        environment_slug: environment,
        list_cost_cents: list_cost,
        input_tokens: usage.and_then(|u| u.input_tokens),
        output_tokens: usage.and_then(|u| u.output_tokens),
        active_seconds: usage.and_then(|u| u.active_seconds),
        budget_reached,
        last_event_type: event.data.kind.clone(),
    })?;
    Ok(())
}

pub fn unix_now() -> i64 {
    time::OffsetDateTime::now_utc().unix_timestamp()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seen_events_dedupes_and_evicts() {
        let seen = SeenEvents::new(2);
        assert!(!seen.check_and_insert("a"));
        assert!(seen.check_and_insert("a"));
        assert!(!seen.check_and_insert("b"));
        assert!(!seen.check_and_insert("c")); // evicts "a"
        assert!(!seen.check_and_insert("a"));
    }

    #[test]
    fn envelope_parses_documented_shape() {
        let e: Envelope = serde_json::from_str(
            r#"{"type":"event","id":"whe_1","created_at":"2026-03-18T14:05:22Z",
                "data":{"type":"session.status_idled","id":"sesn_1","organization_id":"o","workspace_id":"w"}}"#,
        )
        .unwrap();
        assert_eq!(e.data.kind, SESSION_STATUS_IDLED);
        assert_eq!(e.data.id, "sesn_1");
    }
}
