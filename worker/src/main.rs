//! Iron-Fleet `rig-gpu` worker. Claims sessions assigned to a self-hosted
//! environment, runs their tool calls locally against the RTX 5070, posts
//! results back. The agent loop itself stays on Anthropic's side — this binary
//! never decides what to do next, only executes what it's told (see
//! `protocol.rs` and `exec.rs`).

mod client;
mod config;
mod error;
mod exec;
mod gpu;
mod protocol;

use clap::{Parser, Subcommand};
use client::Client;
use config::Config;
use error::{Error, Result};
use exec::ExecutionContext;
use protocol::{Claim, ToolResult};
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "iron-fleet-worker", version, about)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Poll forever (default).
    Run,
    /// Claim at most one unit of work and exit. For exercising the loop
    /// against `worker/dev/mock-rig-gpu.py` without leaving something running.
    ClaimOnce,
}

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
    let cli = Cli::parse();
    let once = matches!(cli.command, Some(Command::ClaimOnce));

    let cfg = Config::from_env()?;
    tracing::info!(?cfg, "starting");
    std::fs::create_dir_all(&cfg.workdir).map_err(|e| {
        Error::Config(format!(
            "create WORKER_WORKDIR {}: {e}",
            cfg.workdir.display()
        ))
    })?;
    gpu::log_status().await;

    let client = Client::new(
        cfg.anthropic_base_url.clone(),
        cfg.environment_id.clone(),
        cfg.environment_key.clone(),
    )?;

    loop {
        tokio::select! {
            _ = shutdown_signal() => {
                tracing::info!("shutdown signal received, exiting");
                return Ok(());
            }
            claimed = client.claim() => {
                match claimed {
                    Ok(Some(claim)) => {
                        let claim_id = claim.claim_id.clone();
                        if let Err(e) = handle_claim(&client, &cfg, claim).await {
                            tracing::error!(error = %e, claim_id, "claim handling failed");
                        }
                        if once {
                            return Ok(());
                        }
                    }
                    Ok(None) => {
                        if once {
                            tracing::info!("no work queued");
                            return Ok(());
                        }
                        tokio::time::sleep(cfg.poll_interval).await;
                    }
                    Err(e) => {
                        tracing::error!(error = %e, "claim request failed, backing off");
                        tokio::time::sleep(cfg.poll_interval).await;
                    }
                }
            }
        }
    }
}

async fn handle_claim(client: &Client, cfg: &Config, claim: Claim) -> Result<()> {
    tracing::info!(
        claim_id = %claim.claim_id,
        session_id = %claim.session_id,
        tool_calls = claim.tool_calls.len(),
        "claimed work"
    );

    if let Some(lease) = claim.lease_seconds {
        if cfg.heartbeat_interval.as_secs() * 2 >= lease {
            tracing::warn!(
                claim_id = %claim.claim_id,
                lease_seconds = lease,
                heartbeat_seconds = cfg.heartbeat_interval.as_secs(),
                "WORKER_HEARTBEAT_SECONDS is not comfortably inside this claim's lease"
            );
        }
    }

    let ctx = match ExecutionContext::for_session(&cfg.workdir, &claim.session_id) {
        Ok(ctx) => ctx,
        Err(e) => {
            // Can't do the work — give the claim back rather than let it sit
            // until the lease expires on its own.
            let _ = client.release(&claim.claim_id).await;
            return Err(Error::Exec(format!("create execution context: {e}")));
        }
    };

    // Heartbeat runs alongside the tool calls so a long CUDA job doesn't outlive
    // the claim's lease and get reassigned out from under it.
    let heartbeat_client = client.clone();
    let claim_id_for_heartbeat = claim.claim_id.clone();
    let heartbeat_interval = cfg.heartbeat_interval;
    let heartbeat = tokio::spawn(async move {
        loop {
            tokio::time::sleep(heartbeat_interval).await;
            if let Err(e) = heartbeat_client.heartbeat(&claim_id_for_heartbeat).await {
                tracing::warn!(error = %e, claim_id = %claim_id_for_heartbeat, "heartbeat failed");
            }
        }
    });

    let mut results: Vec<ToolResult> = Vec::with_capacity(claim.tool_calls.len());
    for call in &claim.tool_calls {
        tracing::info!(claim_id = %claim.claim_id, tool = %call.name, tool_use_id = %call.id, "running tool call");
        let result = ctx.run_tool(call, cfg.tool_timeout).await;
        if result.is_error {
            tracing::warn!(claim_id = %claim.claim_id, tool = %call.name, tool_use_id = %call.id, "tool call failed");
        }
        results.push(result);
    }

    heartbeat.abort();

    client.submit_results(&claim.claim_id, &results).await?;
    tracing::info!(claim_id = %claim.claim_id, results = results.len(), "submitted results");
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
}
