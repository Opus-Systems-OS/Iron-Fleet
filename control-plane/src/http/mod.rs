//! Router, shared state, and bearer auth for the control plane's own API.

pub mod agents;
pub mod health;
pub mod sessions;
pub mod usage;

use crate::anthropic::Client;
use crate::db::Db;
use crate::error::Error;
use crate::webhook::{self, signature::SigningKey, SeenEvents};
use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::{get, post};
use axum::Router;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tower_http::trace::TraceLayer;

#[derive(Clone)]
pub struct AppState {
    pub api: Client,
    pub db: Db,
    pub signing_key: SigningKey,
    pub seen_events: Arc<SeenEvents>,
    pub control_plane_token: Arc<String>,
    pub console_workspace: Arc<String>,
    /// `Some` once `mcp_fleet::ensure_vault` has provisioned a vault —
    /// attached to every session's `vault_ids` so an agent whose own
    /// `mcp_servers` references that URL authenticates automatically.
    pub mcp_fleet_vault_id: Arc<Option<String>>,
}

pub fn router(state: AppState) -> Router {
    // Bearer-protected: anything that can spend money or read fleet state.
    let protected = Router::new()
        .route("/agents", get(agents::list))
        .route("/sessions", post(sessions::create).get(sessions::list))
        .route("/sessions/{id}", get(sessions::get))
        .route(
            "/sessions/{id}/events",
            post(sessions::send_event).get(sessions::list_events),
        )
        .route("/sessions/{id}/stream", get(sessions::stream))
        .route("/sessions/{id}/interrupt", post(sessions::interrupt))
        .route("/usage", get(usage::get))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_bearer,
        ));

    // Public: Anthropic can't send our bearer token; the webhook is HMAC-verified instead.
    let public = Router::new()
        .route("/healthz", get(health::healthz))
        .route("/webhooks/managed-agents", post(webhook::handle));

    Router::new()
        .merge(protected)
        .merge(public)
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn require_bearer(
    State(state): State<AppState>,
    req: Request,
    next: Next,
) -> Result<Response, Error> {
    let presented = req
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    let expected = state.control_plane_token.as_bytes();
    let ok = presented.len() == expected.len() && bool::from(presented.as_bytes().ct_eq(expected));
    if !ok {
        return Err(Error::Unauthorized);
    }
    Ok(next.run(req).await)
}
