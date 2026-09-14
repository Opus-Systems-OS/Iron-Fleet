//! Tauri commands the frontend calls via `invoke`. Every network request to
//! the control plane happens here, in Rust, not as a `fetch` in the webview —
//! that sidesteps the webview's CORS enforcement entirely (the control plane
//! sets no `Access-Control-Allow-Origin` header) and keeps the bearer token
//! out of the page's JS-visible network layer.
//!
//! Stage 3 is the read-only Fleet Dashboard: these commands only ever `GET`.
//! Starting or steering a session is stage 4.

use crate::config::ControlPlaneConfig;
use crate::AppState;
use serde::Serialize;
use tauri::State;

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

/// Most recent 50 sessions, newest first. Pagination is a stage 4 concern —
/// the dashboard is a live snapshot, not a session browser, in stage 3.
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
    if id.is_empty()
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    {
        return Err("invalid session id".to_owned());
    }
    get_json(&state.http, &cfg, &format!("/sessions/{id}")).await
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
