//! One error type for the worker. Mirrors `control-plane`'s shape (kind + status
//! where it applies) since both talk to the same Managed Agents surface.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("config: {0}")]
    Config(String),

    #[error("managed agents {status}: {kind}: {message}")]
    Upstream {
        status: u16,
        kind: String,
        message: String,
        request_id: Option<String>,
    },
    #[error("managed agents transport: {0}")]
    UpstreamTransport(#[from] reqwest::Error),

    #[error("execution: {0}")]
    Exec(String),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
