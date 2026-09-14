//! Runtime configuration. Same rule as `control-plane` and `worker`: nothing
//! comes from a committed file, only the environment.

use crate::error::{Error, Result};
use std::env;

pub struct Config {
    pub control_plane_url: String,
    pub control_plane_token: String,
    /// The bearer token a caller (jarvis's Managed Agents session) must
    /// present to reach this server's `/mcp` route. A separate credential
    /// from `control_plane_token` on purpose: this one only ever proves
    /// "I'm allowed to use the five jarvis tools," never "I'm allowed to
    /// call the control plane's full API."
    pub mcp_fleet_token: String,
    pub port: u16,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("control_plane_url", &self.control_plane_url)
            .field("control_plane_token", &"<redacted>")
            .field("mcp_fleet_token", &"<redacted>")
            .field("port", &self.port)
            .finish()
    }
}

impl Config {
    pub fn from_env() -> Result<Self> {
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

        let port = match optional("PORT") {
            Some(p) => p
                .parse()
                .map_err(|_| Error::Config(format!("PORT is not a valid port: {p:?}")))?,
            None => 8090,
        };

        Ok(Config {
            control_plane_url: required("CONTROL_PLANE_URL")?
                .trim_end_matches('/')
                .to_owned(),
            control_plane_token: required("CONTROL_PLANE_TOKEN")?,
            mcp_fleet_token: required("MCP_FLEET_TOKEN")?,
            port,
        })
    }
}
