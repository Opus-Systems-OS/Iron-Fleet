//! Session routes. Create applies the registry's budget policy; reads —
//! including the live event stream — are proxied to the Managed Agents API
//! and passed through unchanged.

use super::AppState;
use crate::anthropic::types::{
    AgentRef, Budget, CustomTool, SessionCreate, SessionEvent, SessionResource,
};
use crate::error::{Error, Result};
use crate::money::Cents;
use crate::registry::is_github_repo_url;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path, Query, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
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
    /// Extra `https://github.com/<owner>/<repo>` URLs to mount for this
    /// session, on top of the agent's registry `github.mount` list. Needs the
    /// agent to have a `github` block (that's where the token comes from).
    #[serde(default)]
    pub repositories: Vec<String>,
    /// Client-executed custom tools for this session only (`type` must be
    /// `custom`). The session gets the agent's own tools plus these; the
    /// agent resource is untouched.
    #[serde(default)]
    pub tools: Vec<CustomTool>,
    /// Appended to the agent's system prompt for this session only, after a
    /// blank line. At most 4 000 characters.
    #[serde(default)]
    pub system_suffix: Option<String>,
}

const SYSTEM_SUFFIX_MAX_CHARS: usize = 4_000;

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

/// The agent's registry `github.mount` list plus the request's extra
/// `repositories`, deduped, each as a `github_repository` resource carrying
/// the token named by `github.token_env`. The token is read from the
/// process environment per request and never stored or logged.
fn github_resources(
    state: &AppState,
    slug: &str,
    extra: &[String],
) -> Result<Vec<SessionResource>> {
    let github = state.db.agent_github(slug)?;
    let Some(github) = github else {
        if extra.is_empty() {
            return Ok(vec![]);
        }
        return Err(Error::InvalidRequest(format!(
            "agent `{slug}` has no github block in its registry file, so it cannot mount repositories"
        )));
    };

    let mut urls: Vec<String> = Vec::new();
    for url in github.mounts.iter().chain(extra) {
        let url = url.trim().trim_end_matches('/');
        if !is_github_repo_url(url) {
            return Err(Error::InvalidRequest(format!(
                "repository `{url}` must be https://github.com/<owner>/<repo>"
            )));
        }
        if !urls.iter().any(|u| u.eq_ignore_ascii_case(url)) {
            urls.push(url.to_owned());
        }
    }
    if urls.is_empty() {
        return Ok(vec![]);
    }

    let token = std::env::var(&github.token_env)
        .ok()
        .map(|t| t.trim().to_owned())
        .filter(|t| !t.is_empty())
        .ok_or_else(|| {
            Error::Config(format!(
                "agent `{slug}` mounts repositories but {} is not set in the control plane's environment",
                github.token_env
            ))
        })?;

    Ok(urls
        .into_iter()
        .map(|url| SessionResource::GithubRepository {
            url,
            authorization_token: token.clone(),
        })
        .collect())
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

    let resources = github_resources(&state, &agent.slug, &req.repositories)?;

    // The fleet-wide mcp-fleet vault (harmless on an agent with no
    // mcp_servers entry: a vault credential only applies to a server the
    // agent's own definition references, by URL, at runtime — see
    // docs.claude.com/managed-agents/vaults) plus this agent's own vault of
    // sandbox secrets, if its registry file declares any.
    let mut vault_ids: Vec<String> = state.mcp_fleet_vault_id.iter().cloned().collect();
    vault_ids.extend(state.db.agent_vault(&agent.slug)?);

    let agent_ref = agent_ref(&state, &agent, &req.tools, req.system_suffix.as_deref()).await?;

    let body = SessionCreate {
        agent: agent_ref,
        environment_id: environment_id.clone(),
        title: Some(title_from(task)),
        budget: Budget::limit(agent.max_list_cost_cents),
        initial_events: vec![SessionEvent::text(task)],
        metadata: BTreeMap::from([
            ("iron_fleet_agent".to_owned(), agent.slug.clone()),
            ("iron_fleet_environment".to_owned(), env_slug.clone()),
        ]),
        vault_ids,
        resources,
    };

    let session = state.api.create_session(&body).await?;
    tracing::info!(
        slug = %agent.slug,
        session = %session.id,
        status = %session.status,
        environment = %env_slug,
        cap_cents = %agent.max_list_cost_cents,
        repositories = body.resources.len(),
        custom_tools = req.tools.len(),
        system_suffix = req.system_suffix.is_some(),
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

/// Pinned, unless the caller added tools or a system suffix — then the
/// `agent_with_overrides` form, built on the agent's *live* definition (a
/// `tools` override replaces the list in full, so the agent's own tools
/// must be restated; `GET /v1/agents/{id}` is the one source that already
/// has `mcp_toolset` names and everything else resolved).
async fn agent_ref(
    state: &AppState,
    agent: &crate::db::AgentRow,
    tools: &[CustomTool],
    system_suffix: Option<&str>,
) -> Result<AgentRef> {
    let suffix = system_suffix.map(str::trim).filter(|s| !s.is_empty());
    if tools.is_empty() && suffix.is_none() {
        return Ok(AgentRef::pinned(&agent.agent_id, agent.agent_version));
    }
    validate_custom_tools(tools)?;
    if let Some(sfx) = suffix {
        if sfx.chars().count() > SYSTEM_SUFFIX_MAX_CHARS {
            return Err(Error::InvalidRequest(format!(
                "system_suffix is longer than {SYSTEM_SUFFIX_MAX_CHARS} characters"
            )));
        }
    }
    let live = state.api.get_agent_raw(&agent.agent_id).await?;
    Ok(build_overrides(agent, &live, tools, suffix))
}

/// Pure assembly, for tests: the live agent's tools + the custom ones; the
/// live system prompt + a blank line + the suffix.
fn build_overrides(
    agent: &crate::db::AgentRow,
    live: &Value,
    tools: &[CustomTool],
    suffix: Option<&str>,
) -> AgentRef {
    let merged_tools = if tools.is_empty() {
        None
    } else {
        let mut list: Vec<Value> = live["tools"].as_array().cloned().unwrap_or_default();
        list.extend(
            tools
                .iter()
                .map(|t| serde_json::to_value(t).expect("serializable")),
        );
        Some(list)
    };
    let system = suffix.map(|sfx| match live["system"].as_str() {
        Some(base) if !base.trim().is_empty() => format!("{base}\n\n{sfx}"),
        _ => sfx.to_owned(),
    });
    AgentRef::with_overrides(&agent.agent_id, agent.agent_version, merged_tools, system)
}

fn validate_custom_tools(tools: &[CustomTool]) -> Result<()> {
    if tools.len() > 32 {
        return Err(Error::InvalidRequest("at most 32 custom tools".into()));
    }
    let mut seen = std::collections::HashSet::new();
    for t in tools {
        if t.kind != "custom" {
            return Err(Error::InvalidRequest(format!(
                "tool `{}`: only type `custom` may be added to a session",
                t.name
            )));
        }
        let name_ok = !t.name.is_empty()
            && t.name.len() <= 64
            && t.name
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_');
        if !name_ok {
            return Err(Error::InvalidRequest(format!(
                "tool name `{}` must be 1-64 chars of [a-z0-9_]",
                t.name
            )));
        }
        if !seen.insert(t.name.as_str()) {
            return Err(Error::InvalidRequest(format!(
                "duplicate tool `{}`",
                t.name
            )));
        }
        if t.description.trim().is_empty() {
            return Err(Error::InvalidRequest(format!(
                "tool `{}` needs a description",
                t.name
            )));
        }
        if !t.input_schema.is_object() {
            return Err(Error::InvalidRequest(format!(
                "tool `{}`: input_schema must be a JSON Schema object",
                t.name
            )));
        }
    }
    Ok(())
}

/// Body of `POST /sessions/{id}/tool-results`: answers to
/// `agent.custom_tool_use` events, by id.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResultsRequest {
    pub results: Vec<ToolResult>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolResult {
    /// The `id` of the `agent.custom_tool_use` event being answered.
    pub custom_tool_use_id: String,
    /// What the tool returned, as text.
    pub content: String,
    #[serde(default)]
    pub is_error: bool,
}

pub async fn tool_results(
    State(state): State<AppState>,
    Path(id): Path<String>,
    body: Result<Json<ToolResultsRequest>, JsonRejection>,
) -> Result<Json<Value>> {
    if !valid_session_id(&id) {
        return Err(Error::InvalidRequest(
            "session id has unexpected characters".into(),
        ));
    }
    let Json(req) = body.map_err(|e| Error::InvalidRequest(e.body_text()))?;
    if req.results.is_empty() {
        return Err(Error::InvalidRequest("results must not be empty".into()));
    }
    let mut events = Vec::with_capacity(req.results.len());
    for r in &req.results {
        if !valid_session_id(&r.custom_tool_use_id) {
            return Err(Error::InvalidRequest(
                "custom_tool_use_id has unexpected characters".into(),
            ));
        }
        events.push(SessionEvent::custom_tool_result(
            &r.custom_tool_use_id,
            &r.content,
            r.is_error,
        ));
    }
    let n = events.len();
    let result = state.api.send_events(&id, events).await?;
    tracing::info!(session = %id, results = n, "custom tool results sent");
    Ok(Json(result))
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
        .send_events(&id, vec![SessionEvent::text(task)])
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

/// Query for `GET /sessions/{id}/events`. `types` is comma-separated here
/// and fanned out to the API's repeated `types[]`; `order` (`asc`, the
/// default, or `desc`) passes through like the sessions list's — `desc`
/// with `types=agent.message&limit=1` is how mcp-fleet fetches a session's
/// latest reply for jarvis.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EventsQuery {
    #[serde(default)]
    pub page: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
    #[serde(default)]
    pub types: Option<String>,
    #[serde(default)]
    pub order: Option<String>,
}

/// Event history, oldest first unless `order=desc`, as the Anthropic
/// envelope (`data`, `next_page`, `prev_page`) unchanged. Paired with
/// `stream` for the documented reconnect pattern: open the stream, list
/// history, dedupe on `id`.
pub async fn list_events(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<EventsQuery>,
) -> Result<Json<Value>> {
    if !valid_session_id(&id) {
        return Err(Error::InvalidRequest(
            "session id has unexpected characters".into(),
        ));
    }
    let mut query: Vec<(&str, String)> = Vec::new();
    if let Some(page) = &q.page {
        query.push(("page", page.clone()));
    }
    if let Some(limit) = q.limit {
        query.push(("limit", limit.to_string()));
    }
    for t in csv(q.types.as_deref()) {
        query.push(("types[]", t));
    }
    if let Some(order) = &q.order {
        query.push(("order", order.clone()));
    }
    let borrowed: Vec<(&str, &str)> = query.iter().map(|(k, v)| (*k, v.as_str())).collect();
    Ok(Json(state.api.list_events_raw(&id, &borrowed).await?))
}

/// The only `event_deltas` values the API accepts; anything else is a 400
/// upstream, so reject it here with a clearer message.
const EVENT_DELTA_TYPES: [&str; 2] = ["agent.message", "agent.thinking"];

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StreamQuery {
    /// Comma-separated subset of `EVENT_DELTA_TYPES`.
    #[serde(default)]
    pub event_deltas: Option<String>,
}

fn parse_event_deltas(raw: Option<&str>) -> Result<Vec<String>> {
    let deltas = csv(raw);
    if let Some(bad) = deltas
        .iter()
        .find(|d| !EVENT_DELTA_TYPES.contains(&d.as_str()))
    {
        return Err(Error::InvalidRequest(format!(
            "event_deltas `{bad}` is not one of {}",
            EVENT_DELTA_TYPES.join(", ")
        )));
    }
    Ok(deltas)
}

fn csv(raw: Option<&str>) -> Vec<String> {
    raw.unwrap_or("")
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// `GET /sessions/{id}/stream`: the session's live event stream, proxied as
/// SSE byte-for-byte. Each frame is `data: {event}` where `{event}` is the
/// Anthropic event object unchanged. Holds the upstream connection open for
/// as long as the caller does; dropping this response cancels it. Only
/// events emitted after the stream opens arrive — list `/events` afterwards
/// to fill in history.
pub async fn stream(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<StreamQuery>,
) -> Result<Response> {
    if !valid_session_id(&id) {
        return Err(Error::InvalidRequest(
            "session id has unexpected characters".into(),
        ));
    }
    let deltas = parse_event_deltas(q.event_deltas.as_deref())?;
    let upstream = state.api.open_event_stream(&id, &deltas).await?;
    tracing::info!(session = %id, deltas = deltas.len(), "event stream opened");
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/event-stream")
        .header(header::CACHE_CONTROL, "no-cache")
        // Tell buffering proxies (nginx-style) to pass frames through as they arrive.
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(upstream.bytes_stream()))
        .map_err(|e| Error::Config(format!("stream response: {e}")))
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

    fn agent_row() -> crate::db::AgentRow {
        crate::db::AgentRow {
            slug: "jarvis".into(),
            agent_id: "agent_1".into(),
            agent_version: 5,
            definition_sha256: String::new(),
            max_list_cost_cents: "50".parse().unwrap(),
            effort: "low".into(),
            default_environment: "cloud-default".into(),
            synced_at: String::new(),
        }
    }

    fn music_tool() -> CustomTool {
        CustomTool {
            kind: "custom".into(),
            name: "play_music".into(),
            description: "Play something in the Music app.".into(),
            input_schema: serde_json::json!({"type": "object", "properties": {}}),
        }
    }

    #[test]
    fn overrides_restate_the_live_tools_and_append_the_suffix() {
        let live = serde_json::json!({
            "id": "agent_1", "version": 5,
            "system": "You are Jarvis.",
            "tools": [{"type": "agent_toolset_20260401"}, {"type": "mcp_toolset", "mcp_server_name": "fleet"}]
        });
        let r = build_overrides(&agent_row(), &live, &[music_tool()], Some("Be brief."));
        assert_eq!(r.kind, "agent_with_overrides");
        assert_eq!((r.id.as_str(), r.version), ("agent_1", 5));
        let tools = r.tools.unwrap();
        assert_eq!(
            tools.len(),
            3,
            "agent's two tools kept, one custom appended"
        );
        assert_eq!(tools[1]["mcp_server_name"], "fleet");
        assert_eq!(tools[2]["type"], "custom");
        assert_eq!(tools[2]["name"], "play_music");
        assert_eq!(r.system.as_deref(), Some("You are Jarvis.\n\nBe brief."));

        // Suffix only: tools untouched (None → inherited), system appended.
        let r = build_overrides(&agent_row(), &live, &[], Some("Be brief."));
        assert!(r.tools.is_none());
        assert!(r.system.is_some());
        // Tools only: system inherited.
        let r = build_overrides(&agent_row(), &live, &[music_tool()], None);
        assert!(r.system.is_none());
        assert_eq!(r.tools.as_ref().unwrap().len(), 3);
        // The wire shape serializes the type tag and omits absent overrides.
        let json = serde_json::to_value(&r).unwrap();
        assert_eq!(json["type"], "agent_with_overrides");
        assert!(json.get("system").is_none());
    }

    #[test]
    fn custom_tools_are_validated() {
        assert!(validate_custom_tools(&[music_tool()]).is_ok());
        let mut bad = music_tool();
        bad.kind = "agent_toolset_20260401".into();
        assert!(validate_custom_tools(&[bad]).is_err(), "only custom");
        let mut bad = music_tool();
        bad.name = "Play Music".into();
        assert!(validate_custom_tools(&[bad]).is_err(), "name charset");
        let mut bad = music_tool();
        bad.input_schema = serde_json::json!("not an object");
        assert!(validate_custom_tools(&[bad]).is_err());
        assert!(
            validate_custom_tools(&[music_tool(), music_tool()]).is_err(),
            "duplicate"
        );
    }

    #[test]
    fn custom_tool_result_event_shape() {
        let ev = SessionEvent::custom_tool_result("sevt_1", "Now playing: Around the World", false);
        let json = serde_json::to_value(&ev).unwrap();
        assert_eq!(json["type"], "user.custom_tool_result");
        assert_eq!(json["custom_tool_use_id"], "sevt_1");
        assert_eq!(json["content"][0]["text"], "Now playing: Around the World");
        assert!(json.get("is_error").is_none(), "false is omitted");
        let err =
            serde_json::to_value(SessionEvent::custom_tool_result("sevt_1", "no match", true))
                .unwrap();
        assert_eq!(err["is_error"], true);
    }

    #[test]
    fn event_deltas_are_validated_against_the_documented_set() {
        assert_eq!(parse_event_deltas(None).unwrap(), Vec::<String>::new());
        assert_eq!(
            parse_event_deltas(Some("agent.message, agent.thinking")).unwrap(),
            vec!["agent.message".to_owned(), "agent.thinking".to_owned()]
        );
        let err = parse_event_deltas(Some("agent.message,span.model_request_start")).unwrap_err();
        assert!(matches!(err, Error::InvalidRequest(_)), "{err}");
    }

    #[test]
    fn csv_trims_and_drops_empties() {
        assert_eq!(csv(Some(" a,,b ,")), vec!["a".to_owned(), "b".to_owned()]);
        assert!(csv(Some("")).is_empty());
    }

    #[test]
    fn create_request_rejects_unknown_fields() {
        let r: std::result::Result<CreateRequest, _> =
            serde_json::from_str(r#"{"agent_slug":"jarvis","task":"t","budget":"9999"}"#);
        assert!(r.is_err(), "callers must not be able to pass a budget");
    }
}
