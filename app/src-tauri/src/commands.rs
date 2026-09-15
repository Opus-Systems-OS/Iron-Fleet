//! Tauri commands the frontend calls via `invoke`. Every network request to
//! the control plane happens here, in Rust, not as a `fetch` in the webview —
//! that sidesteps the webview's CORS enforcement entirely (the control plane
//! sets no `Access-Control-Allow-Origin` header) and keeps the bearer token
//! out of the page's JS-visible network layer.
//!
//! Stage 3 added the read-only dashboard (`GET` only). Stage 4 adds session
//! controls — start, follow up, interrupt — and the Usage tab. Phase 3 of
//! the centralization plan adds `watch_session`: the selected session's live
//! event stream, delivered to the webview as Tauri events (see `stream.rs`).
//! There is still no way to raise a session's cap: the control plane's
//! budgets are create-only, so no route exists for it at any layer.

use crate::config::ControlPlaneConfig;
use crate::{stream, AppState};
use serde::Serialize;
use tauri::{AppHandle, State};

/// A running `stream::watch` task. Dropping the handle does not stop the
/// task; `abort` does, which is what reselecting a session relies on.
pub struct Watch {
    pub session_id: String,
    handle: tauri::async_runtime::JoinHandle<()>,
}

#[derive(Debug, Serialize)]
pub struct ConnectionStatus {
    configured: bool,
    url: Option<String>,
}

#[tauri::command]
pub fn connection_status(state: State<AppState>) -> ConnectionStatus {
    let cfg = state.config.lock().expect("config mutex poisoned").clone();
    ConnectionStatus {
        configured: cfg.is_some(),
        url: cfg.map(|c| c.url),
    }
}

/// Saves and switches to a new control plane URL/token. Round-trips through
/// `connection_status` on success so the frontend doesn't need a second call.
#[tauri::command]
pub fn set_connection(
    url: String,
    token: String,
    state: State<AppState>,
) -> Result<ConnectionStatus, String> {
    let cfg = ControlPlaneConfig::new(url, token)
        .ok_or_else(|| "URL and token are both required".to_owned())?;
    cfg.save(&state.config_path)
        .map_err(|e| format!("could not save config: {e}"))?;
    let status = ConnectionStatus {
        configured: true,
        url: Some(cfg.url.clone()),
    };
    *state.config.lock().expect("config mutex poisoned") = Some(cfg);
    Ok(status)
}

#[tauri::command]
pub async fn list_agents(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let cfg = require_config(&state)?;
    get_json(&state.http, &cfg, "/agents").await
}

/// Most recent 50 sessions, newest first. Pagination is out of scope — the
/// dashboard is a live snapshot, not a session browser.
#[tauri::command]
pub async fn list_sessions(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let cfg = require_config(&state)?;
    get_json(&state.http, &cfg, "/sessions?limit=50&order=desc").await
}

#[tauri::command]
pub async fn get_session(
    id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let cfg = require_config(&state)?;
    let id = valid_id(id)?;
    get_json(&state.http, &cfg, &format!("/sessions/{id}")).await
}

/// `repositories`: optional extra GitHub repo URLs to mount for this session,
/// on top of the agent's registry defaults. Validated by the control plane;
/// the token comes from there too — the app never sees it.
#[tauri::command]
pub async fn create_session(
    agent_slug: String,
    task: String,
    repositories: Option<Vec<String>>,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let cfg = require_config(&state)?;
    if agent_slug.trim().is_empty() || task.trim().is_empty() {
        return Err("agent and task are both required".to_owned());
    }
    let repositories: Vec<String> = repositories
        .unwrap_or_default()
        .into_iter()
        .map(|r| r.trim().to_owned())
        .filter(|r| !r.is_empty())
        .collect();
    let body = serde_json::json!({
        "agent_slug": agent_slug,
        "task": task,
        "repositories": repositories,
    });
    post_json(&state.http, &cfg, "/sessions", &body).await
}

/// Append a follow-up message to a running session.
#[tauri::command]
pub async fn send_session_event(
    id: String,
    task: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let cfg = require_config(&state)?;
    let id = valid_id(id)?;
    if task.trim().is_empty() {
        return Err("message must not be empty".to_owned());
    }
    let body = serde_json::json!({ "task": task });
    post_json(&state.http, &cfg, &format!("/sessions/{id}/events"), &body).await
}

#[tauri::command]
pub async fn interrupt_session(
    id: String,
    state: State<'_, AppState>,
) -> Result<serde_json::Value, String> {
    let cfg = require_config(&state)?;
    let id = valid_id(id)?;
    post_json(
        &state.http,
        &cfg,
        &format!("/sessions/{id}/interrupt"),
        &serde_json::Value::Null,
    )
    .await
}

#[tauri::command]
pub async fn get_usage(state: State<'_, AppState>) -> Result<serde_json::Value, String> {
    let cfg = require_config(&state)?;
    get_json(&state.http, &cfg, "/usage").await
}

/// Start streaming `id`'s events to the webview as `session-event` /
/// `session-stream-state` Tauri events, replacing any watch already running.
/// One watch at a time on purpose: it follows the selected row, and the
/// Fleet tab has exactly one of those.
#[tauri::command]
pub async fn watch_session(
    id: String,
    app: AppHandle,
    state: State<'_, AppState>,
) -> Result<(), String> {
    let cfg = require_config(&state)?;
    let id = valid_id(id)?;
    let handle = tauri::async_runtime::spawn(stream::watch(
        app,
        state.stream_http.clone(),
        cfg,
        id.clone(),
    ));
    let previous = state
        .watch
        .lock()
        .expect("watch mutex poisoned")
        .replace(Watch {
            session_id: id,
            handle,
        });
    if let Some(prev) = previous {
        prev.handle.abort();
    }
    Ok(())
}

#[tauri::command]
pub fn unwatch_session(state: State<'_, AppState>) {
    if let Some(prev) = state.watch.lock().expect("watch mutex poisoned").take() {
        prev.handle.abort();
    }
}

fn valid_id(id: String) -> Result<String, String> {
    let ok = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok {
        Ok(id)
    } else {
        Err("invalid session id".to_owned())
    }
}

fn require_config(state: &AppState) -> Result<ControlPlaneConfig, String> {
    state
        .config
        .lock()
        .expect("config mutex poisoned")
        .clone()
        .ok_or_else(|| "not connected — set the control plane URL and token first".to_owned())
}

async fn get_json(
    http: &reqwest::Client,
    cfg: &ControlPlaneConfig,
    path: &str,
) -> Result<serde_json::Value, String> {
    let url = format!("{}{}", cfg.url, path);
    let resp = http
        .get(&url)
        .bearer_auth(&cfg.token)
        .send()
        .await
        .map_err(|e| format!("could not reach {}: {e}", cfg.url))?;
    read_response(resp).await
}

/// `body: &Value::Null` sends no request body at all (for endpoints like
/// `/interrupt` that take none) rather than a literal JSON `null`.
async fn post_json(
    http: &reqwest::Client,
    cfg: &ControlPlaneConfig,
    path: &str,
    body: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let url = format!("{}{}", cfg.url, path);
    let mut req = http.post(&url).bearer_auth(&cfg.token);
    if !body.is_null() {
        req = req.json(body);
    }
    let resp = req
        .send()
        .await
        .map_err(|e| format!("could not reach {}: {e}", cfg.url))?;
    read_response(resp).await
}

async fn read_response(resp: reqwest::Response) -> Result<serde_json::Value, String> {
    let status = resp.status();
    let body: serde_json::Value = resp
        .json()
        .await
        .map_err(|e| format!("{status}: unreadable response body: {e}"))?;
    if !status.is_success() {
        let message = body["error"]["message"]
            .as_str()
            .unwrap_or("no error message in response");
        return Err(format!("{status}: {message}"));
    }
    Ok(body)
}
