//! Iron-Fleet's MCP server: exposes the control plane's jarvis-scoped
//! surface (`server.rs`) over MCP's Streamable HTTP transport, so it can be
//! attached to the `jarvis` agent as a remote `mcp_servers` entry. Deployed
//! last in the build order, once the surface it wraps (`control-plane`'s
//! session routes, stable since stage 4) has stopped moving.

mod client;
mod config;
mod error;
mod server;

use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::{self, Next};
use axum::response::Response;
use axum::routing::get;
use axum::Router;
use client::ControlPlaneClient;
use config::Config;
use error::{Error, Result};
use rmcp::transport::streamable_http_server::session::local::LocalSessionManager;
use rmcp::transport::streamable_http_server::{StreamableHttpServerConfig, StreamableHttpService};
use server::FleetServer;
use std::sync::Arc;
use subtle::ConstantTimeEq;
use tokio_util::sync::CancellationToken;
use tower_http::trace::TraceLayer;
use tracing_subscriber::EnvFilter;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")),
        )
        .with_target(false)
        .init();

    if let Err(e) = run().await {
        tracing::error!(error = %e, "fatal");
        std::process::exit(1);
    }
}

async fn run() -> Result<()> {
    let cfg = Config::from_env()?;
    tracing::info!(?cfg, "starting");

    let client = ControlPlaneClient::new(cfg.control_plane_url.clone(), cfg.control_plane_token)?;

    let cancel = CancellationToken::new();
    let mcp_service: StreamableHttpService<FleetServer, LocalSessionManager> =
        StreamableHttpService::new(
            move || Ok(FleetServer::new(client.clone())),
            Default::default(),
            StreamableHttpServerConfig::default().with_cancellation_token(cancel.child_token()),
        );

    let mcp_fleet_token = Arc::new(cfg.mcp_fleet_token);
    let protected = Router::new().nest_service("/mcp", mcp_service).route_layer(
        middleware::from_fn_with_state(mcp_fleet_token, require_bearer),
    );

    let app = Router::new()
        .route("/healthz", get(healthz))
        .merge(protected)
        .layer(TraceLayer::new_for_http());

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|source| Error::Bind { addr, source })?;
    tracing::info!(%addr, "mcp-fleet listening");

    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal(cancel))
        .await
        .map_err(Error::Server)?;
    Ok(())
}

async fn healthz() -> &'static str {
    "ok"
}

/// Same constant-time bearer check as `control-plane`'s `require_bearer`.
/// Checks `MCP_FLEET_TOKEN`, not `CONTROL_PLANE_TOKEN` — a caller here is
/// only proven to hold the narrower, tool-scoped credential.
async fn require_bearer(
    State(expected): State<Arc<String>>,
    req: Request,
    next: Next,
) -> std::result::Result<Response, StatusCode> {
    let presented = req
        .headers()
        .get(http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");
    let expected = expected.as_bytes();
    let ok = presented.len() == expected.len() && bool::from(presented.as_bytes().ct_eq(expected));
    if !ok {
        return Err(StatusCode::UNAUTHORIZED);
    }
    Ok(next.run(req).await)
}

async fn shutdown_signal(cancel: CancellationToken) {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
    #[cfg(unix)]
    let term = async {
        use tokio::signal::unix::{signal, SignalKind};
        if let Ok(mut s) = signal(SignalKind::terminate()) {
            s.recv().await;
        }
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutdown signal received");
    cancel.cancel();
}
