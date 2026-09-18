//! Where the dashboard points: a control-plane base URL and its bearer token.
//!
//! This is app *configuration*, not fleet state — CLAUDE.md is explicit that
//! clients hold no fleet state and session state lives on Anthropic's side.
//! Which control plane to talk to is closer to a bookmarked URL than session
//! data, so it's fine for it to live in a small local file. `CONTROL_PLANE_URL`
//! / `CONTROL_PLANE_TOKEN` env vars win when set (handy for `npm run tauri
//! dev`); otherwise it's read from `<app config dir>/control-plane.json`,
//! written by the in-app connection form. Since the Opus Systems OS API
//! (2026-09-18) `url` is the API base including `/v1`
//! (`https://api.opustower.dev/v1`) and `token` is a per-device API key
//! (`osk_…`); the file name is historical.

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControlPlaneConfig {
    pub url: String,
    pub token: String,
}

impl ControlPlaneConfig {
    pub fn new(url: impl Into<String>, token: impl Into<String>) -> Option<Self> {
        let url = url.into().trim().trim_end_matches('/').to_owned();
        let token = token.into().trim().to_owned();
        if url.is_empty() || token.is_empty() {
            return None;
        }
        Some(ControlPlaneConfig { url, token })
    }

    pub fn from_env() -> Option<Self> {
        let url = std::env::var("CONTROL_PLANE_URL").ok()?;
        let token = std::env::var("CONTROL_PLANE_TOKEN").ok()?;
        Self::new(url, token)
    }

    pub fn load(path: &Path) -> Option<Self> {
        let text = std::fs::read_to_string(path).ok()?;
        serde_json::from_str(&text).ok()
    }

    pub fn save(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self).expect("Serialize is infallible here");
        std::fs::write(path, text)
    }
}
