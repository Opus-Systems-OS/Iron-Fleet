//! Thin Managed Agents client. Every request goes through [`Client::send`], which
//! is the only place headers are set — so the beta header cannot be forgotten.

pub mod types;

use crate::error::{Error, Result};
use reqwest::{Method, StatusCode};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::Value;
use types::*;

/// Beta header required on every Managed Agents request (CLAUDE.md conventions).
/// Memory-store endpoints use `agent-memory-2026-07-22` instead; the control plane
/// does not call those in stage 1.
pub const MANAGED_AGENTS_BETA: &str = "managed-agents-2026-04-01";
pub const ANTHROPIC_VERSION: &str = "2023-06-01";

#[derive(Clone)]
pub struct Client {
    http: reqwest::Client,
    base_url: String,
    api_key: String,
}

impl std::fmt::Debug for Client {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Client")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl Client {
    pub fn new(base_url: impl Into<String>, api_key: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "iron-fleet-control-plane/",
                env!("CARGO_PKG_VERSION")
            ))
            .timeout(std::time::Duration::from_secs(60))
            .build()?;
        Ok(Client {
            http,
            base_url: base_url.into(),
            api_key: api_key.into(),
        })
    }

    /// The single request path. Sets the API key, API version and beta header.
    async fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&impl Serialize>,
    ) -> Result<T> {
        let url = format!("{}{}", self.base_url, path);
        let mut req = self
            .http
            .request(method.clone(), &url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("anthropic-beta", MANAGED_AGENTS_BETA)
            .header("accept", "application/json")
            .query(query);
        if let Some(b) = body {
            req = req.json(b);
        }

        let resp = req.send().await?;
        let status = resp.status();
        let request_id = resp
            .headers()
            .get("request-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = resp.bytes().await?;

        if !status.is_success() {
            return Err(upstream_error(status, &bytes, request_id, &method, path));
        }

        serde_json::from_slice(&bytes).map_err(|e| Error::Upstream {
            status: status.as_u16(),
            kind: "unparseable_response".into(),
            message: format!("{method} {path}: {e}"),
            request_id,
        })
    }

    // ---- agents

    pub async fn create_agent(&self, def: &AgentDefinition) -> Result<Agent> {
        self.send(Method::POST, "/v1/agents", &[], Some(def)).await
    }

    /// Full-body update with no `version`: declarative apply, last write wins.
    pub async fn update_agent(&self, id: &str, def: &AgentDefinition) -> Result<Agent> {
        self.send(Method::POST, &format!("/v1/agents/{id}"), &[], Some(def))
            .await
    }

    // ---- environments

    pub async fn create_environment(&self, def: &EnvironmentDefinition) -> Result<Environment> {
        self.send(Method::POST, "/v1/environments", &[], Some(def))
            .await
    }

    // ---- sessions

    pub async fn create_session(&self, body: &SessionCreate) -> Result<Session> {
        self.send(Method::POST, "/v1/sessions", &[], Some(body))
            .await
    }

    /// Raw session object, passed through to callers unchanged.
    pub async fn get_session_raw(&self, id: &str) -> Result<Value> {
        self.send::<Value>(Method::GET, &format!("/v1/sessions/{id}"), &[], None::<&()>)
            .await
    }

    pub async fn get_session(&self, id: &str) -> Result<Session> {
        self.send::<Session>(Method::GET, &format!("/v1/sessions/{id}"), &[], None::<&()>)
            .await
    }

    /// Raw list envelope (`data`, `next_page`, `prev_page`), passed through unchanged.
    pub async fn list_sessions_raw(&self, query: &[(&str, &str)]) -> Result<Value> {
        self.send::<Value>(Method::GET, "/v1/sessions", query, None::<&()>)
            .await
    }
}

fn upstream_error(
    status: StatusCode,
    body: &[u8],
    request_id: Option<String>,
    method: &Method,
    path: &str,
) -> Error {
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
