//! Iron-Fleet control plane. `serve` (default) runs the HTTP service;
//! `sync` reconciles `agents/` with Anthropic and exits.

mod anthropic;
mod config;
mod db;
mod error;
mod http;
mod mcp_fleet;
mod money;
mod registry;
mod webhook;

use clap::{Parser, Subcommand};
use config::Config;
use std::sync::Arc;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "control-plane", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Run the HTTP service (default).
    Serve,
    /// Reconcile agents/ with the Managed Agents API and exit.
    Sync,
    /// Dev helper: sign a webhook body from stdin with ANTHROPIC_WEBHOOK_SIGNING_KEY
    /// and print the three headers as curl `-H` flags.
    SignWebhook {
        /// Value for the webhook-id header (also the event id to dedupe on).
        #[arg(long, default_value = "whe_local_test")]
        id: String,
    },
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| EnvFilter::new("info,tower_http=info")),
        )
        .with_target(false)
        .init();

    if let Err(e) = run().await {
        tracing::error!(error = %e, "fatal");
        std::process::exit(1);
    }
}

async fn run() -> error::Result<()> {
    let cli = Cli::parse();
    if let Some(Command::SignWebhook { id }) = &cli.command {
        return sign_webhook(id);
    }
    let cfg = Config::from_env()?;
    let db = db::Db::open(&cfg.database_path)?;
    let api = anthropic::Client::new(&cfg.anthropic_base_url, &cfg.anthropic_api_key)?;

    let do_sync = matches!(cli.command, Some(Command::Sync)) || cfg.sync_on_boot;
    if do_sync {
        let reg = registry::load_dir(&cfg.agents_dir)?;
        tracing::info!(
            agents = reg.agents.len(),
            environments = reg.environments.len(),
            dir = %cfg.agents_dir.display(),
            "registry loaded"
        );
        let report = registry::sync::sync(&reg, &api, &db).await?;
        print_new_environment_keys(&report.new_environment_keys);
    }

    let mcp_fleet_vault_id = match (&cfg.mcp_fleet_url, &cfg.mcp_fleet_token) {
        (Some(url), Some(token)) => Some(mcp_fleet::ensure_vault(&api, &db, url, token).await?),
        _ => None,
    };

    if matches!(cli.command, Some(Command::Sync)) {
        return Ok(());
    }

    let signing_key = webhook::signature::SigningKey::parse(&cfg.webhook_signing_key)
        .map_err(|e| error::Error::Config(format!("ANTHROPIC_WEBHOOK_SIGNING_KEY: {e}")))?;

    let state = http::AppState {
        api,
        db,
        signing_key,
        seen_events: Arc::new(webhook::SeenEvents::new(10_000)),
        control_plane_token: Arc::new(cfg.control_plane_token.clone()),
        console_workspace: Arc::new(cfg.anthropic_workspace.clone()),
        mcp_fleet_vault_id: Arc::new(mcp_fleet_vault_id),
    };

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], cfg.port));
    let listener = tokio::net::TcpListener::bind(addr)
        .await
        .map_err(|e| error::Error::Config(format!("bind {addr}: {e}")))?;
    tracing::info!(%addr, db = %cfg.database_path.display(), "control plane listening");

    axum::serve(listener, http::router(state))
        .with_graceful_shutdown(shutdown_signal())
        .await
        .map_err(|e| error::Error::Config(format!("server: {e}")))?;
    Ok(())
}

/// Printed with `eprintln!`, never `tracing`, so a self_hosted environment's
/// key can't end up in an aggregated log sink. Shown exactly once, at the sync
/// that provisions it — the control plane does not store it (CLAUDE.md: "the
/// rig's environment key stays on the rig").
fn print_new_environment_keys(keys: &[(String, String, String)]) {
    for (slug, environment_id, key) in keys {
        eprintln!(
            "\n=== new self_hosted environment: {slug} ({environment_id}) ===\n\
             This key is shown once and is not stored anywhere. Copy it onto the rig now:\n\
             \n  RIG_ENVIRONMENT_ID={environment_id}\n  RIG_ENVIRONMENT_KEY={key}\n\n\
             It will not be printed again; if it's lost, provisioning must be redone.\n"
        );
    }
}

fn sign_webhook(id: &str) -> error::Result<()> {
    use std::io::Read;
    let secret = std::env::var("ANTHROPIC_WEBHOOK_SIGNING_KEY")
        .map_err(|_| error::Error::Config("ANTHROPIC_WEBHOOK_SIGNING_KEY is not set".into()))?;
    let key = webhook::signature::SigningKey::parse(&secret)
        .map_err(|e| error::Error::Config(format!("ANTHROPIC_WEBHOOK_SIGNING_KEY: {e}")))?;
    let mut body = Vec::new();
    std::io::stdin()
        .read_to_end(&mut body)
        .map_err(|e| error::Error::Config(format!("stdin: {e}")))?;
    let ts = webhook::unix_now().to_string();
    let sig = webhook::signature::signature_header(&key, id, &ts, &body);
    println!("-H 'webhook-id: {id}' -H 'webhook-timestamp: {ts}' -H 'webhook-signature: {sig}'");
    Ok(())
}

async fn shutdown_signal() {
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
}
