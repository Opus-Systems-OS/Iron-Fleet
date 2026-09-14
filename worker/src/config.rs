//! Runtime configuration. Everything comes from the environment, same rule as
//! `control-plane`: nothing sensitive is ever read from a committed file.
//!
//! `RIG_ENVIRONMENT_KEY` is the one secret that is only ever supposed to exist
//! here — CLAUDE.md: "the rig's environment key stays on the rig." It is never
//! logged; `Config`'s `Debug` impl is hand-written to guarantee that.

use crate::error::{Error, Result};
use std::env;
use std::path::PathBuf;
use std::time::Duration;

pub struct Config {
    pub anthropic_base_url: String,
    pub environment_id: String,
    pub environment_key: String,
    pub poll_interval: Duration,
    pub heartbeat_interval: Duration,
    pub tool_timeout: Duration,
    pub workdir: PathBuf,
}

impl std::fmt::Debug for Config {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Config")
            .field("anthropic_base_url", &self.anthropic_base_url)
            .field("environment_id", &self.environment_id)
            .field("environment_key", &"<redacted>")
            .field("poll_interval", &self.poll_interval)
            .field("heartbeat_interval", &self.heartbeat_interval)
            .field("tool_timeout", &self.tool_timeout)
            .field("workdir", &self.workdir)
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
        let optional_secs = |name: &str, default: u64| -> Result<Duration> {
            match optional(name) {
                Some(v) => {
                    let secs: u64 = v.parse().map_err(|_| {
                        Error::Config(format!("{name} is not a whole number of seconds: {v:?}"))
                    })?;
                    if secs == 0 {
                        return Err(Error::Config(format!("{name} must be greater than zero")));
                    }
                    Ok(Duration::from_secs(secs))
                }
                None => Ok(Duration::from_secs(default)),
            }
        };

        Ok(Config {
            anthropic_base_url: optional("ANTHROPIC_BASE_URL")
                .unwrap_or_else(|| "https://api.anthropic.com".into())
                .trim_end_matches('/')
                .to_owned(),
            environment_id: required("RIG_ENVIRONMENT_ID")?,
            environment_key: required("RIG_ENVIRONMENT_KEY")?,
            poll_interval: optional_secs("WORKER_POLL_SECONDS", 5)?,
            heartbeat_interval: optional_secs("WORKER_HEARTBEAT_SECONDS", 20)?,
            tool_timeout: optional_secs("WORKER_TOOL_TIMEOUT_SECONDS", 900)?,
            workdir: optional("WORKER_WORKDIR")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("./work")),
        })
    }
}
