//! Thin Managed Agents client. Every Managed Agents request goes through
//! [`Client::send`], which is the only place the beta header is set — so it
//! cannot be forgotten. The GA Skills API (`/v1/skills`, multipart, no beta)
//! goes through [`Client::send_multipart`]; both share auth and error handling.

pub mod types;

use crate::error::{Error, Result};
use crate::registry::SkillDir;
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

    /// The Managed Agents request path. Sets the API key, API version and beta header.
    async fn send<T: DeserializeOwned>(
        &self,
        method: Method,
        path: &str,
        query: &[(&str, &str)],
        body: Option<&impl Serialize>,
    ) -> Result<T> {
        let mut req = self
            .request(method.clone(), path)
            .header("anthropic-beta", MANAGED_AGENTS_BETA)
            .query(query);
        if let Some(b) = body {
            req = req.json(b);
        }
        self.finish(req, &method, path).await
    }

    /// The Skills API path: same auth, `multipart/form-data`, and **no** beta
    /// header — `/v1/skills` is GA and the Managed Agents beta does not apply.
    async fn send_multipart<T: DeserializeOwned>(
        &self,
        path: &str,
        form: reqwest::multipart::Form,
    ) -> Result<T> {
        let req = self.request(Method::POST, path).multipart(form);
        self.finish(req, &Method::POST, path).await
    }

    fn request(&self, method: Method, path: &str) -> reqwest::RequestBuilder {
        let url = format!("{}{}", self.base_url, path);
        self.http
            .request(method, &url)
            .header("x-api-key", &self.api_key)
            .header("anthropic-version", ANTHROPIC_VERSION)
            .header("accept", "application/json")
    }

    async fn finish<T: DeserializeOwned>(
        &self,
        req: reqwest::RequestBuilder,
        method: &Method,
        path: &str,
    ) -> Result<T> {
        let resp = req.send().await?;
        let status = resp.status();
        let request_id = resp
            .headers()
            .get("request-id")
            .and_then(|v| v.to_str().ok())
            .map(str::to_owned);
        let bytes = resp.bytes().await?;

        if !status.is_success() {
            return Err(upstream_error(status, &bytes, request_id, method, path));
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

    // ---- skills

    /// `POST /v1/skills`: first upload of a skill directory. `display_name`
    /// is left to default from the frontmatter `name`.
    pub async fn create_skill(&self, skill: &SkillDir) -> Result<Skill> {
        self.send_multipart("/v1/skills", skill_form(skill)).await
    }

    /// `POST /v1/skills/{id}/versions`: a full snapshot (never a delta).
    pub async fn create_skill_version(
        &self,
        skill_id: &str,
        skill: &SkillDir,
    ) -> Result<SkillVersion> {
        self.send_multipart(
            &format!("/v1/skills/{skill_id}/versions"),
            skill_form(skill),
        )
        .await
    }

    // ---- environments

    pub async fn create_environment(&self, def: &EnvironmentDefinition) -> Result<Environment> {
        self.send(Method::POST, "/v1/environments", &[], Some(def))
            .await
    }

    // ---- vaults

    pub async fn create_vault(&self, def: &VaultCreate) -> Result<Vault> {
        self.send(Method::POST, "/v1/vaults", &[], Some(def)).await
    }

    pub async fn create_credential(
        &self,
        vault_id: &str,
        def: &CredentialCreate,
    ) -> Result<Credential> {
        self.send(
            Method::POST,
            &format!("/v1/vaults/{vault_id}/credentials"),
            &[],
            Some(def),
        )
        .await
    }

    /// Rotate a credential's secret in place (`secret_name` / `mcp_server_url`
    /// are immutable; anything else goes through here).
    pub async fn update_credential(
        &self,
        vault_id: &str,
        credential_id: &str,
        def: &CredentialUpdate,
    ) -> Result<Credential> {
        self.send(
            Method::POST,
            &format!("/v1/vaults/{vault_id}/credentials/{credential_id}"),
            &[],
            Some(def),
        )
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

    /// Append events to a running session (e.g. a follow-up `user.message`).
    /// **Unconfirmed** — see `SendEvents`'s doc comment; no fixture from a live
    /// run backs this endpoint path yet, unlike the rest of this client.
    pub async fn send_events(
        &self,
        session_id: &str,
        events: Vec<UserMessageEvent>,
    ) -> Result<Value> {
        self.send::<Value>(
            Method::POST,
            &format!("/v1/sessions/{session_id}/events"),
            &[],
            Some(&SendEvents { events }),
        )
        .await
    }

    /// Stop a session's in-flight work without ending the session. Same
    /// unconfirmed-endpoint caveat as `send_events`.
    pub async fn interrupt_session(&self, session_id: &str) -> Result<Value> {
        self.send::<Value>(
            Method::POST,
            &format!("/v1/sessions/{session_id}/interrupt"),
            &[],
            None::<&()>,
        )
        .await
    }
}

/// One `files[]` part per file, named `<skill>/<relative path>` — the Skills
/// API requires every file under a single top-level directory that contains
/// `SKILL.md`, and reads the directory structure from the part file names.
/// reqwest percent-encodes file names by default, which would turn the `/`
/// into `%2F` and flatten the tree; `percent_encode_noop` sends them verbatim.
fn skill_form(skill: &SkillDir) -> reqwest::multipart::Form {
    let mut form = reqwest::multipart::Form::new().percent_encode_noop();
    for (rel, bytes) in &skill.files {
        let part = reqwest::multipart::Part::bytes(bytes.clone())
            .file_name(format!("{}/{}", skill.name, rel));
        form = form.part("files[]", part);
    }
    form
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
