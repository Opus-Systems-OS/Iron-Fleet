//! One error type for startup and the HTTP listener. Tool-call failures are a
//! separate, deliberately simpler path — see `client.rs`'s doc comment.

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("config: {0}")]
    Config(String),
    #[error("http client: {0}")]
    Http(#[from] reqwest::Error),
    #[error("bind {addr}: {source}")]
    Bind {
        addr: std::net::SocketAddr,
        source: std::io::Error,
    },
    #[error("server: {0}")]
    Server(std::io::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;
