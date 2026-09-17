//! HTTP client to `control-plane`'s own API (not the Managed Agents API
//! directly — `control-plane`'s job is to sit between this server and
//! Anthropic). Authenticates with `CONTROL_PLANE_TOKEN`, this server's own
//! credential, distinct from the `MCP_FLEET_TOKEN` a caller presents to us.
//!
//! Every method returns `Result<String, String>` rather than this crate's
//! `error::Error`: the `String` on either side becomes MCP tool content
//! directly (rmcp's `IntoCallToolResult` is implemented for
//! `Result<T: IntoContents, E: IntoContents>`, and both branches here just
//! need to be readable text) — a failed control-plane call should come back
//! to the model as a normal tool error it can react to, not a protocol-level
//! failure.

use serde_json::Value;

#[derive(Clone)]
pub struct ControlPlaneClient {
    http: reqwest::Client,
    base_url: String,
    token: String,
}

impl ControlPlaneClient {
    pub fn new(base_url: impl Into<String>, token: impl Into<String>) -> reqwest::Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!("iron-fleet-mcp-fleet/", env!("CARGO_PKG_VERSION")))
            .timeout(std::time::Duration::from_secs(30))
            .build()?;
        Ok(ControlPlaneClient {
            http,
            base_url: base_url.into(),
            token: token.into(),
        })
    }

    pub async fn get(&self, path: &str) -> Result<String, String> {
        self.get_json(path).await.map(|v| v.to_string())
    }

    /// `get`, but the parsed body — for tools that compose a view out of
    /// more than one control-plane response instead of passing one through.
    pub async fn get_json(&self, path: &str) -> Result<Value, String> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("could not reach the control plane: {e}"))?;
        Self::read_json(resp).await
    }

    pub async fn post(&self, path: &str, body: &Value) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .json(body)
            .send()
            .await
            .map_err(|e| format!("could not reach the control plane: {e}"))?;
        Self::read_body(resp).await
    }

    pub async fn post_empty(&self, path: &str) -> Result<String, String> {
        let url = format!("{}{}", self.base_url, path);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.token)
            .send()
            .await
            .map_err(|e| format!("could not reach the control plane: {e}"))?;
        Self::read_body(resp).await
    }

    async fn read_body(resp: reqwest::Response) -> Result<String, String> {
        Self::read_json(resp).await.map(|v| v.to_string())
    }

    async fn read_json(resp: reqwest::Response) -> Result<Value, String> {
        let status = resp.status();
        let body: Value = resp
            .json()
            .await
            .map_err(|e| format!("{status}: unreadable response body: {e}"))?;
        if !status.is_success() {
            let message = body["error"]["message"]
                .as_str()
                .unwrap_or("no error message in response");
            return Err(format!("{status}: {message}"));
        }
        Ok(body)
    }
}
