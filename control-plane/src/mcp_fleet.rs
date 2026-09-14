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

/// Idempotent: does nothing (beyond a log line) if a credential for this
/// exact `mcp_server_url` was already provisioned in a prior boot. Returns
/// the vault id to attach to every session via `vault_ids`.
///
/// A credential's `mcp_server_url` is immutable once created (the old one
/// can't just be edited in place when `MCP_FLEET_URL` changes — e.g. moving
/// from a placeholder to `mcp-fleet`'s real deployed URL), but a vault holds
/// up to 20 credentials and the key only has to be unique *within* the
/// vault, so the fix is a new credential in the same vault, not a new vault.
/// The stale credential for the old URL is left in place rather than
/// archived — it matches nothing any agent references anymore, so it's
/// inert, and archiving requires the credential id we deliberately don't
/// keep around (see `db::McpFleetVaultRow`'s doc comment).
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
        tracing::info!(
            vault_id = %existing.vault_id,
            old_url = %existing.mcp_server_url,
            new_url = %mcp_server_url,
            "MCP_FLEET_URL changed; adding a new credential to the existing vault"
        );
        let credential = create_credential(api, &existing.vault_id, mcp_server_url, token).await?;
        db.set_mcp_fleet_vault(&existing.vault_id, &credential.id, mcp_server_url)?;
        return Ok(existing.vault_id);
    }

    let vault = api
        .create_vault(&VaultCreate {
            display_name: "Iron-Fleet".into(),
            metadata: BTreeMap::new(),
        })
        .await?;
    let credential = create_credential(api, &vault.id, mcp_server_url, token).await?;
    tracing::info!(vault_id = %vault.id, credential_id = %credential.id, "mcp-fleet vault provisioned");
    db.set_mcp_fleet_vault(&vault.id, &credential.id, mcp_server_url)?;
    Ok(vault.id)
}

async fn create_credential(
    api: &Client,
    vault_id: &str,
    mcp_server_url: &str,
    token: &str,
) -> Result<crate::anthropic::types::Credential> {
    api.create_credential(
        vault_id,
        &CredentialCreate {
            display_name: "mcp-fleet".into(),
            auth: CredentialAuth::StaticBearer {
                mcp_server_url: mcp_server_url.to_owned(),
                token: token.to_owned(),
            },
        },
    )
    .await
}
