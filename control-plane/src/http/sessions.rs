//! Session routes. Create applies the registry's budget policy; reads are
//! proxied live to the Managed Agents API and passed through unchanged.

use super::AppState;
use crate::anthropic::types::{AgentRef, Budget, SessionCreate, UserMessageEvent};
use crate::error::{Error, Result};
use crate::money::Cents;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

fn console_url(workspace: &str, session_id: &str) -> String {
    format!("https://platform.claude.com/workspaces/{workspace}/sessions/{session_id}")
}

/// Same character rule Anthropic resource ids follow elsewhere in this file.
fn valid_session_id(id: &str) -> bool {
    !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

const TITLE_MAX_CHARS: usize = 80;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CreateRequest {
    pub agent_slug: String,
    pub task: String,
    #[serde(default)]
    pub environment: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct CreateResponse {
    pub session_id: String,
    pub status: String,
    pub agent_slug: String,
    pub agent_id: String,
    pub agent_version: u32,
    pub environment: String,
    pub environment_id: String,
    pub budget: BudgetSummary,
    pub console_url: String,
}

#[derive(Debug, Serialize)]
pub struct BudgetSummary {
    pub max_list_cost_cents: Cents,
}

pub async fn create(
    State(state): State<AppState>,
    body: Result<Json<CreateRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<CreateResponse>)> {
    // Route axum's body rejection through our JSON error shape instead of its plain-text 422.
    let Json(req) = body.map_err(|e| Error::InvalidRequest(e.body_text()))?;
    let task = req.task.trim();
    if task.is_empty() {
        return Err(Error::InvalidRequest("task must not be empty".into()));
    }

    let agent = state
        .db
        .agent(&req.agent_slug)?
        .ok_or_else(|| Error::UnknownAgent(req.agent_slug.clone()))?;

    let env_slug = req
        .environment
        .unwrap_or_else(|| agent.default_environment.clone());
    let env = state
        .db
        .environment(&env_slug)?
        .ok_or_else(|| Error::UnknownEnvironment(env_slug.clone()))?;
    let environment_id = env
        .environment_id
        .ok_or_else(|| Error::EnvironmentNotProvisioned(env_slug.clone()))?;

    let body = SessionCreate {
        agent: AgentRef::pinned(&agent.agent_id, agent.agent_version),
        environment_id: environment_id.clone(),
        title: Some(title_from(task)),
        budget: Budget::limit(agent.max_list_cost_cents),
        initial_events: vec![UserMessageEvent::text(task)],
        metadata: BTreeMap::from([
            ("iron_fleet_agent".to_owned(), agent.slug.clone()),
            ("iron_fleet_environment".to_owned(), env_slug.clone()),
        ]),
    };

    let session = state.api.create_session(&body).await?;
    tracing::info!(
        slug = %agent.slug,
        session = %session.id,
        status = %session.status,
        environment = %env_slug,
        cap_cents = %agent.max_list_cost_cents,
        "session created"
    );

    Ok((
        StatusCode::CREATED,
        Json(CreateResponse {
            console_url: console_url(&state.console_workspace, &session.id),
            session_id: session.id,
            status: session.status,
            agent_slug: agent.slug,
            agent_id: agent.agent_id,
            agent_version: agent.agent_version,
            environment: env_slug,
            environment_id,
            budget: BudgetSummary {
                max_list_cost_cents: agent.max_list_cost_cents,
            },
        }),
    ))
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListQuery {
    #[serde(default)]
    pub agent_slug: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub page: Option<String>,
    #[serde(default)]
    pub order: Option<String>,
}

pub async fn list(
    State(state): State<AppState>,
    Query(q): Query<ListQuery>,
) -> Result<Json<Value>> {
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(slug) = &q.agent_slug {
        let agent = state
            .db
            .agent(slug)?
            .ok_or_else(|| Error::UnknownAgent(slug.clone()))?;
        query.push(("agent_id", agent.agent_id));
    }
    if let Some(limit) = q.limit {
        query.push(("limit", limit.to_string()));
    }
    if let Some(page) = &q.page {
        query.push(("page", page.clone()));
    }
    if let Some(order) = &q.order {
        query.push(("order", order.clone()));
    }
    let borrowed: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
    let mut envelope = state.api.list_sessions_raw(&borrowed).await?;
    if let Some(items) = envelope.get_mut("data").and_then(|d| d.as_array_mut()) {
        for item in items {
            if let Some(id) = item.get("id").and_then(|v| v.as_str()).map(str::to_owned) {
                item["console_url"] = Value::String(console_url(&state.console_workspace, &id));
            }
        }
    }
    Ok(Json(envelope))
}

pub async fn get(State(state): State<AppState>, Path(id): Path<String>) -> Result<Json<Value>> {
    if !valid_session_id(&id) {
        return Err(Error::InvalidRequest(
            "session id has unexpected characters".into(),
        ));
    }
    let mut session = state.api.get_session_raw(&id).await?;
    session["console_url"] = Value::String(console_url(&state.console_workspace, &id));
    Ok(Json(session))
}

/// Body of `POST /sessions/{id}/events`: one follow-up `user.message`.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SendEventRequest {
    pub task: String,
}

pub async fn send_event(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<SendEventRequest>, JsonRejection>,
) -> Result<Json<Value>> {
    if !valid_session_id(&id) {
        return Err(Error::InvalidRequest(
            "session id has unexpected characters".into(),
        ));
    }
    let Json(req) = body.map_err(|e| Error::InvalidRequest(e.body_text()))?;
    let task = req.task.trim();
    if task.is_empty() {
        return Err(Error::InvalidRequest("task must not be empty".into()));
    }
    let result = state
        .api
        .send_events(&id, vec![UserMessageEvent::text(task)])
        .await?;
    tracing::info!(session = %id, "event sent");
    Ok(Json(result))
}

pub async fn interrupt(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Value>> {
    if !valid_session_id(&id) {
        return Err(Error::InvalidRequest(
            "session id has unexpected characters".into(),
        ));
    }
    let result = state.api.interrupt_session(&id).await?;
    tracing::info!(session = %id, "session interrupted");
    Ok(Json(result))
}

fn title_from(task: &str) -> String {
    let first_line = task.lines().next().unwrap_or(task).trim();
    let mut t: String = first_line.chars().take(TITLE_MAX_CHARS).collect();
    if first_line.chars().count() > TITLE_MAX_CHARS {
        t.push('…');
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn console_url_is_the_documented_format() {
        assert_eq!(
            console_url("default", "sesn_1"),
            "https://platform.claude.com/workspaces/default/sessions/sesn_1"
        );
    }

    #[test]
    fn session_id_validation_matches_agent_id_charset() {
        assert!(valid_session_id("sesn_01ABCxyz-_9"));
        assert!(!valid_session_id(""));
        assert!(!valid_session_id("../etc/passwd"));
        assert!(!valid_session_id("sesn 1"));
    }

    #[test]
    fn title_is_first_line_truncated() {
        assert_eq!(title_from("Hello\nworld"), "Hello");
        let long = "x".repeat(100);
        let t = title_from(&long);
        assert_eq!(t.chars().count(), TITLE_MAX_CHARS + 1);
        assert!(t.ends_with('…'));
    }

    #[test]
    fn create_request_rejects_unknown_fields() {
        let r: std::result::Result<CreateRequest, _> =
            serde_json::from_str(r#"{"agent_slug":"jarvis","task":"t","budget":"9999"}"#);
        assert!(r.is_err(), "callers must not be able to pass a budget");
    }
}
