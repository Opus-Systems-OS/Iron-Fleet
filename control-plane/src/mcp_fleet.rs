//! Provisions the one vault + `static_bearer` credential that authenticates
//! sessions to `mcp-fleet` (docs.claude.com/managed-agents/vaults). Separate
//! from `registry::sync` on purpose: this isn't driven by a committed file,
//! only by whether `MCP_FLEET_URL`/`MCP_FLEET_TOKEN` are set, and there's
//! exactly one of it — a vault per fleet, not per agent.

use crate::anthropic::types::{CredentialAuth, CredentialCreate, VaultCreate};
use crate::anthropic::Client;
use crate::db::Db;
use crate::error::Result;
use std::collections::BTreeMap;

/// Idempotent: does nothing (beyond a log line) if a vault for this exact
/// `mcp_server_url` was already provisioned in a prior boot. Returns the
/// vault id to attach to every session via `vault_ids`.
pub async fn ensure_vault(
    api: &Client,
    db: &Db,
    mcp_server_url: &str,
    token: &str,
) -> Result<String> {
    if let Some(existing) = db.mcp_fleet_vault()? {
        if existing.mcp_server_url == mcp_server_url {
            tracing::debug!(vault_id = %existing.vault_id, "mcp-fleet vault already provisioned");
            return Ok(existing.vault_id);
        }
        // The credential key (mcp_server_url) is immutable once created — the
        // old one has to be archived by hand before a new one can replace it,
        // so this needs an operator, not an auto-heal.
        tracing::warn!(
            stored_url = %existing.mcp_server_url,
            configured_url = %mcp_server_url,
            "MCP_FLEET_URL no longer matches the provisioned vault credential; archive \
             the old credential and delete the mcp_fleet_vault row to reprovision"
        );
        return Ok(existing.vault_id);
    }

    let vault = api
        .create_vault(&VaultCreate {
            display_name: "Iron-Fleet".into(),
            metadata: BTreeMap::new(),
        })
        .await?;
    let credential = api
        .create_credential(
            &vault.id,
            &CredentialCreate {
                display_name: "mcp-fleet".into(),
                auth: CredentialAuth::StaticBearer {
                    mcp_server_url: mcp_server_url.to_owned(),
                    token: token.to_owned(),
                },
            },
        )
        .await?;
    tracing::info!(vault_id = %vault.id, credential_id = %credential.id, "mcp-fleet vault provisioned");
    db.set_mcp_fleet_vault(&vault.id, &credential.id, mcp_server_url)?;
    Ok(vault.id)
}
