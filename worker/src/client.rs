//! Thin client for the `rig-gpu` worker's slice of the Managed Agents API.
//!
//! Authenticates with the environment key, not an account `ANTHROPIC_API_KEY`
//! — CLAUDE.md: "the rig's environment key stays on the rig." Every request
//! still needs the same beta header as the control plane
//! (`managed-agents-2026-04-01`); see `protocol.rs` for why the endpoint
//! shapes below are marked unconfirmed.

use crate::error::{Error, Result};
use crate::protocol::*;
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;

pub const MANAGED_AGENTS_BETA: &str = "managed-agents-2026-04-01";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    environment_id: String,
    environment_key: String,
}

impl Client {
    pub fn new(
        base_url: impl Into<String>,
        environment_id: impl Into<String>,
        environment_key: impl Into<String>,
    ) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("iron-fleet-worker/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Client {
            http,
            base_url: base_url.into(),
            environment_id: environment_id.into(),
            environment_key: environment_key.into(),
        })
    }

    async fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        body: Option<&impl Serialize>,
    ) -> Result<Option<T>> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self
            .http
            .request(method.clone(), &url)
            .header("x-environment-key", &self.environment_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", MANAGED_AGENTS_BETA)
            .header("accept", "application/json");
        if let Some(b) = body {
            req = req.json(b);
        }

        let resp = req.send().await?;
        let status = resp.status();
        if status == StatusCode::NO_CONTENT {
            return Ok(None);
        }
        let request_id = resp
            .headers()
            .get("request-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = resp.bytes().await?;

        if !status.is_success() {
            return Err(upstream_error(status, &bytes, request_id, &method, path));
        }
        if bytes.is_empty() {
            return Ok(None);
        }
        serde_json::from_slice(&bytes)
            .map(Some)
            .map_err(|e| Error::Upstream {
                status: status.as_u16(),
                kind: "unparseable_response".into(),
                message: format!("{method} {path}: {e}"),
                request_id,
            })
    }

    /// Long-poll the next queued unit of work for this environment.
    /// `None` means nothing is queued right now — the caller sleeps and retries.
    pub async fn claim(&self) -> Result<Option<Claim>> {
        self.send(
            Method::POST,
            &format!("/v1/environments/{}/claims", self.environment_id),
            None::<&()>,
        )
        .await
    }

    /// Keep a claim alive past its lease while a long tool call is still running.
    pub async fn heartbeat(&self, claim_id: &str) -> Result<()> {
        self.send::<Value>(
            Method::POST,
            &format!(
                "/v1/environments/{}/claims/{claim_id}/heartbeat",
                self.environment_id
            ),
            None::<&()>,
        )
        .await?;
        Ok(())
    }

    pub async fn submit_results(&self, claim_id: &str, results: &[ToolResult]) -> Result<()> {
        self.send::<Value>(
            Method::POST,
            &format!(
                "/v1/environments/{}/claims/{claim_id}/results",
                self.environment_id
            ),
            Some(&SubmitResults { results }),
        )
        .await?;
        Ok(())
    }

    /// Give the claim back unfinished, e.g. on shutdown, so it can be picked up
    /// again rather than sitting until the lease times out.
    pub async fn release(&self, claim_id: &str) -> Result<()> {
        self.send::<Value>(
            Method::POST,
            &format!(
                "/v1/environments/{}/claims/{claim_id}/release",
                self.environment_id
            ),
            None::<&()>,
        )
        .await?;
        Ok(())
    }
}

fn upstream_error(
    status: StatusCode,
    body: &[u8],
    request_id: Option<String>,
    method: &Method,
    path: &str,
) -> Error {
    #[derive(serde::Deserialize)]
    struct ApiError {
        error: ApiErrorBody,
    }
    #[derive(serde::Deserialize)]
    struct ApiErrorBody {
        #[serde(rename = "type")]
        kind: String,
        message: String,
    }
    let (kind, message) = match serde_json::from_slice::<ApiError>(body) {
        Ok(e) => (e.error.kind, e.error.message),
        Err(_) => (
            format!("http_{}", status.as_u16()),
            format!("{method} {path} returned {status} with a non-JSON body"),
        ),
    };
    Error::Upstream {
        status: status.as_u16(),
        kind,
        message,
        request_id,
    }
}
