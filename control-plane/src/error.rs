//! One error type for the whole service, with a single HTTP mapping.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde_json::json;
use std::path::PathBuf;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    // Startup only — never reaches an HTTP response.
    #[error("config: {0}")]
    Config(String),
    #[error("agents dir: {}: {reason}", path.display())]
    Registry { path: PathBuf, reason: String },

    // Caller errors.
    #[error("unknown agent slug `{0}`")]
    UnknownAgent(String),
    #[error("unknown environment `{0}`")]
    UnknownEnvironment(String),
    #[error("environment `{0}` is defined but not provisioned yet")]
    EnvironmentNotProvisioned(String),
    #[error("invalid request: {0}")]
    InvalidRequest(String),
    #[error("unauthorized")]
    Unauthorized,
    #[error("webhook signature rejected: {0}")]
    WebhookSignature(&'static str),

    // Our side / upstream.
    #[error("database: {0}")]
    Db(#[from] rusqlite::Error),
    #[error("anthropic {status}: {kind}: {message}")]
    Upstream {
        status: u16,
        kind: String,
        message: String,
        request_id: Option<String>,
    },
    #[error("anthropic transport: {0}")]
    UpstreamTransport(#[from] reqwest::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl Error {
    /// Stable machine-readable name for the JSON error body.
    pub fn kind(&self) -> &'static str {
        match self {
            Error::Config(_) => "config",
            Error::Registry { .. } => "registry",
            Error::UnknownAgent(_) => "unknown_agent",
            Error::UnknownEnvironment(_) => "unknown_environment",
            Error::EnvironmentNotProvisioned(_) => "environment_not_provisioned",
            Error::InvalidRequest(_) => "invalid_request",
            Error::Unauthorized => "unauthorized",
            Error::WebhookSignature(_) => "webhook_signature",
            Error::Db(_) => "database",
            Error::Upstream { .. } => "upstream",
            Error::UpstreamTransport(_) => "upstream_transport",
        }
    }

    pub fn status(&self) -> StatusCode {
        match self {
            Error::Config(_) | Error::Registry { .. } | Error::Db(_) => {
                StatusCode::INTERNAL_SERVER_ERROR
            }
            Error::UnknownAgent(_) | Error::UnknownEnvironment(_) => StatusCode::NOT_FOUND,
            Error::EnvironmentNotProvisioned(_) => StatusCode::CONFLICT,
            Error::InvalidRequest(_) | Error::WebhookSignature(_) => StatusCode::BAD_REQUEST,
            Error::Unauthorized => StatusCode::UNAUTHORIZED,
            Error::Upstream { status, .. } => match *status {
                404 => StatusCode::NOT_FOUND,
                429 => StatusCode::SERVICE_UNAVAILABLE,
                _ => StatusCode::BAD_GATEWAY,
            },
            Error::UpstreamTransport(_) => StatusCode::BAD_GATEWAY,
        }
    }

    /// Message safe to return to a caller. Internal failures are not echoed.
    fn public_message(&self) -> String {
        match self {
            Error::Db(_) | Error::Config(_) | Error::Registry { .. } => "internal error".to_owned(),
            Error::UpstreamTransport(_) => "could not reach the Managed Agents API".to_owned(),
            Error::Upstream { kind, message, .. } => format!("{kind}: {message}"),
            other => other.to_string(),
        }
    }
}

impl IntoResponse for Error {
    fn into_response(self) -> Response {
        let status = self.status();
        if status.is_server_error() {
            tracing::error!(error = %self, kind = self.kind(), "request failed");
        } else {
            tracing::warn!(error = %self, kind = self.kind(), "request rejected");
        }

        let mut body =
            json!({ "error": { "type": self.kind(), "message": self.public_message() } });
        if let Error::Upstream {
            status: upstream,
            request_id,
            ..
        } = &self
        {
            body["error"]["upstream_status"] = json!(upstream);
            if let Some(id) = request_id {
                body["error"]["request_id"] = json!(id);
            }
        }

        let mut resp = (status, Json(body)).into_response();
        if status == StatusCode::SERVICE_UNAVAILABLE {
            resp.headers_mut().insert(
                http::header::RETRY_AFTER,
                http::HeaderValue::from_static("5"),
            );
        }
        resp
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_mapping() {
        assert_eq!(
            Error::UnknownAgent("x".into()).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            Error::EnvironmentNotProvisioned("rig-gpu".into()).status(),
            StatusCode::CONFLICT
        );
        assert_eq!(
            Error::InvalidRequest("x".into()).status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(
            Error::WebhookSignature("bad").status(),
            StatusCode::BAD_REQUEST
        );
        assert_eq!(Error::Unauthorized.status(), StatusCode::UNAUTHORIZED);
        let up = |status| Error::Upstream {
            status,
            kind: "k".into(),
            message: "m".into(),
            request_id: None,
        };
        assert_eq!(up(404).status(), StatusCode::NOT_FOUND);
        assert_eq!(up(429).status(), StatusCode::SERVICE_UNAVAILABLE);
        assert_eq!(up(400).status(), StatusCode::BAD_GATEWAY);
        assert_eq!(up(500).status(), StatusCode::BAD_GATEWAY);
    }
}
