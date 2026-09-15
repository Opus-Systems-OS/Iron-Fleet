//! Runtime configuration. Everything comes from the environment; nothing is read
//! from files, so a secret can never end up committed by accident.

use crate::error::{Error, Result};
use std::env;
use std::path::PathBuf;

#[derive(Debug, Clone)]
pub struct Config {
    pub anthropic_api_key: String,
    pub anthropic_base_url: String,
    pub anthropic_workspace: String,
    pub webhook_signing_key: String,
    pub control_plane_token: String,
    pub port: u16,
    pub database_path: PathBuf,
    pub agents_dir: PathBuf,
    pub sync_on_boot: bool,
    /// Both or neither: if set, control-plane provisions (once) a vault +
    /// `static_bearer` credential for `mcp-fleet` and attaches it to every
    /// session. Unset by default — jarvis's `mcp_servers` entry works
    /// unauthenticated (or isn't wired up at all) until these are set.
    pub mcp_fleet_url: Option<String>,
    pub mcp_fleet_token: Option<String>,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // Values are trimmed: a trailing newline from a paste would otherwise
        // become an invalid HTTP header and surface as an opaque client error.
        let required = |name: &str| -> Result<String> {
            let v = env::var(name).unwrap_or_default().trim().to_owned();
            if v.is_empty() {
                return Err(Error::Config(format!("{name} is not set")));
            }
            if v.chars().any(|c| c.is_whitespace() || c.is_control()) {
                return Err(Error::Config(format!(
                    "{name} contains whitespace or control characters"
                )));
            }
            Ok(v)
        };
        let optional = |name: &str| {
            env::var(name)
                .ok()
                .map(|v| v.trim().to_owned())
                .filter(|v| !v.is_empty())
        };

        let webhook_signing_key = required("ANTHROPIC_WEBHOOK_SIGNING_KEY")?;
        if !webhook_signing_key.starts_with("whsec_") {
            return Err(Error::Config(
                "ANTHROPIC_WEBHOOK_SIGNING_KEY must be the whsec_-prefixed secret from the Console"
                    .into(),
            ));
        }

        let port = match optional("PORT") {
            Some(p) => p
                .parse()
                .map_err(|_| Error::Config(format!("PORT is not a valid port: {p:?}")))?,
            None => 8080,
        };

        // The droplet compose file sets DATABASE_PATH to the mounted volume.
        let database_path = optional("DATABASE_PATH")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("control-plane.db"));

        let sync_on_boot = match optional("SYNC_ON_BOOT").as_deref() {
            None | Some("true") | Some("1") => true,
            Some("false") | Some("0") => false,
            Some(other) => {
                return Err(Error::Config(format!(
                    "SYNC_ON_BOOT must be true or false, got {other:?}"
                )))
            }
        };

        Ok(Config {
            anthropic_api_key: required("ANTHROPIC_API_KEY")?,
            anthropic_base_url: optional("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| "https://api.anthropic.com".into())
                .trim_end_matches('/')
                .to_owned(),
            anthropic_workspace: optional("ANTHROPIC_WORKSPACE")
                .unwrap_or_else(|| "default".into()),
            webhook_signing_key,
            control_plane_token: required("CONTROL_PLANE_TOKEN")?,
            port,
            database_path,
            agents_dir: optional("AGENTS_DIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("agents")),
            sync_on_boot,
            mcp_fleet_url: optional("MCP_FLEET_URL"),
            mcp_fleet_token: optional("MCP_FLEET_TOKEN"),
        })
    }
}
