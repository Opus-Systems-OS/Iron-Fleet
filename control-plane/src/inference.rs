//! Client for the rig's Ollama (Phase 6, local inference). Talks Ollama's
//! native API — `/api/tags`, `/api/chat`, `/api/embed` — over the tailnet;
//! the rig binds Ollama to `127.0.0.1` and publishes it with
//! `tailscale serve`, so `INFERENCE_URL` is a `100.x` address nothing
//! outside the tailnet can reach.
//!
//! This is a proxy, not an agent loop. It never picks a model, never
//! retries against Claude, and the only thing it knows about the rig is
//! whether a TCP connect succeeded: a failed one is [`Error::RigOffline`]
//! and the caller gets a 503, full stop.

use crate::error::{Error, Result};
use reqwest::{Response, StatusCode};
use serde_json::Value;
use std::time::Duration;

#[derive(Clone)]
pub struct Inference {
    base_url: String,
    /// Connect timeout so a rig that is off fails in seconds, a read timeout
    /// long enough for a cold model load (which emits nothing for tens of
    /// seconds), and **no** total timeout — `/api/chat` streams.
    http: reqwest::Client,
}

impl std::fmt::Debug for Inference {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inference")
            .field("base_url", &self.base_url)
            .finish_non_exhaustive()
    }
}

impl Inference {
    pub fn new(base_url: impl Into<String>) -> Result<Self> {
        let http = reqwest::Client::builder()
            .user_agent(concat!(
                "iron-fleet-control-plane/",
                env!("CARGO_PKG_VERSION")
            ))
            .connect_timeout(Duration::from_secs(5))
            .read_timeout(Duration::from_secs(180))
            .build()?;
        Ok(Inference {
            base_url: base_url.into(),
            http,
        })
    }

    /// `GET /api/tags`: the pulled models, Ollama's JSON unchanged. A hard
    /// 10 s cap — this is what the app polls for its online/offline line.
    pub async fn models(&self) -> Result<Value> {
        let resp = self
            .http
            .get(self.url("/api/tags"))
            .timeout(Duration::from_secs(10))
            .send()
            .await
            .map_err(|e| self.classify(e))?;
        self.json(resp).await
    }

    /// `POST /api/chat`, body passed through. Returns the response so the
    /// handler can stream it; Ollama's `stream` default (`true`) applies.
    pub async fn chat(&self, body: &Value) -> Result<Response> {
        let resp = self
            .http
            .post(self.url("/api/chat"))
            .json(body)
            .send()
            .await
            .map_err(|e| self.classify(e))?;
        self.check(resp).await
    }

    /// `POST /api/embed`, buffered.
    pub async fn embeddings(&self, body: &Value) -> Result<Value> {
        let resp = self
            .http
            .post(self.url("/api/embed"))
            .json(body)
            .send()
            .await
            .map_err(|e| self.classify(e))?;
        self.json(resp).await
    }

    fn url(&self, path: &str) -> String {
        format!("{}{}", self.base_url, path)
    }

    /// Anything that stopped a request from getting an HTTP status back —
    /// connect refused, no route, connect/read timeout — means the rig (or
    /// the tailnet) is down. Ollama itself is single-host and stateless from
    /// our side, so there is no other way to read it.
    pub fn classify(&self, e: reqwest::Error) -> Error {
        if e.is_connect() || e.is_timeout() || e.is_request() {
            let host = self
                .base_url
                .split("//")
                .nth(1)
                .unwrap_or(&self.base_url)
                .to_owned();
            // `e` prints its own URL; the chained sources are the useful
            // part — hyper's "client error (Connect)" alone says nothing,
            // the leaf ("connect timed out", "connection refused") does.
            let mut cause = Vec::new();
            let mut src = std::error::Error::source(&e);
            while let Some(s) = src {
                cause.push(s.to_string());
                src = s.source();
            }
            if cause.is_empty() {
                cause.push(e.to_string());
            }
            Error::RigOffline(format!("{host}: {}", cause.join(": ")))
        } else {
            Error::UpstreamTransport(e)
        }
    }

    /// Non-2xx from Ollama → [`Error::Inference`] carrying its status and
    /// the `{"error": "..."}` string (`model 'x' not found`, etc.).
    async fn check(&self, resp: Response) -> Result<Response> {
        let status = resp.status();
        if status.is_success() {
            return Ok(resp);
        }
        let bytes = resp.bytes().await.map_err(|e| self.classify(e))?;
        Err(inference_error(status, &bytes))
    }

    async fn json(&self, resp: Response) -> Result<Value> {
        let resp = self.check(resp).await?;
        let bytes = resp.bytes().await.map_err(|e| self.classify(e))?;
        serde_json::from_slice(&bytes).map_err(|e| Error::Inference {
            status: 500,
            message: format!("unparseable response from ollama: {e}"),
        })
    }
}

fn inference_error(status: StatusCode, body: &[u8]) -> Error {
    let message = serde_json::from_slice::<Value>(body)
        .ok()
        .and_then(|v| v["error"].as_str().map(str::to_owned))
        .unwrap_or_else(|| {
            let text = String::from_utf8_lossy(body);
            let text = text.trim();
            if text.is_empty() {
                status.canonical_reason().unwrap_or("error").to_owned()
            } else {
                text.chars().take(200).collect()
            }
        });
    Error::Inference {
        status: status.as_u16(),
        message,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::routing::{get, post};
    use axum::{Json, Router};
    use futures_util::StreamExt;
    use serde_json::json;

    /// A port that was just listening and isn't any more: connect refused.
    async fn closed_port() -> u16 {
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = l.local_addr().unwrap().port();
        drop(l);
        port
    }

    #[tokio::test]
    async fn connect_refused_is_rig_offline() {
        let port = closed_port().await;
        let inf = Inference::new(format!("http://127.0.0.1:{port}")).unwrap();
        match inf.models().await {
            Err(Error::RigOffline(msg)) => {
                assert!(msg.starts_with(&format!("127.0.0.1:{port}: ")), "{msg}");
                // The leaf of the source chain, not hyper's opaque
                // "client error (Connect)": both OSes say "refused".
                assert!(msg.to_lowercase().contains("refused"), "{msg}");
            }
            other => panic!("expected RigOffline, got {other:?}"),
        }
        match inf.chat(&json!({"model": "x", "messages": []})).await {
            Err(Error::RigOffline(_)) => {}
            other => panic!("expected RigOffline, got {other:?}"),
        }
    }

    /// Tiny stand-in for Ollama: `/api/tags`, a two-frame NDJSON `/api/chat`,
    /// `/api/embed`, and Ollama's `{"error": …}` shape for an unknown model.
    async fn stub() -> String {
        async fn chat(Json(body): Json<Value>) -> axum::response::Response {
            if body["model"] != "qwen3:8b" {
                return (
                    StatusCode::NOT_FOUND,
                    Json(json!({"error": format!("model '{}' not found", body["model"].as_str().unwrap_or("?"))})),
                )
                    .into_response();
            }
            let frames = futures_util::stream::iter(vec![
                Ok::<_, std::io::Error>(
                    "{\"message\":{\"role\":\"assistant\",\"content\":\"hi\"},\"done\":false}\n",
                ),
                Ok("{\"message\":{\"role\":\"assistant\",\"content\":\"\"},\"done\":true}\n"),
            ]);
            axum::response::Response::builder()
                .header("content-type", "application/x-ndjson")
                .body(Body::from_stream(frames))
                .unwrap()
        }
        use axum::response::IntoResponse;
        let app = Router::new()
            .route(
                "/api/tags",
                get(|| async {
                    Json(json!({"models": [{"name": "qwen3:8b"}, {"name": "nomic-embed-text"}]}))
                }),
            )
            .route("/api/chat", post(chat))
            .route(
                "/api/embed",
                post(|Json(body): Json<Value>| async move {
                    Json(json!({"model": body["model"], "embeddings": [[0.1, 0.2]]}))
                }),
            );
        let l = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = l.local_addr().unwrap();
        tokio::spawn(async move { axum::serve(l, app).await.unwrap() });
        format!("http://{addr}")
    }

    #[tokio::test]
    async fn proxies_tags_chat_stream_and_embed() {
        let inf = Inference::new(stub().await).unwrap();

        let tags = inf.models().await.unwrap();
        assert_eq!(tags["models"][0]["name"], "qwen3:8b");

        let resp = inf
            .chat(&json!({"model": "qwen3:8b", "messages": [{"role": "user", "content": "hi"}]}))
            .await
            .unwrap();
        let mut frames = Vec::new();
        let mut stream = resp.bytes_stream();
        while let Some(chunk) = stream.next().await {
            frames.push(String::from_utf8(chunk.unwrap().to_vec()).unwrap());
        }
        let text = frames.concat();
        assert_eq!(text.lines().count(), 2, "{text}");
        assert!(text.lines().last().unwrap().contains("\"done\":true"));

        let emb = inf
            .embeddings(&json!({"model": "nomic-embed-text", "input": "x"}))
            .await
            .unwrap();
        assert_eq!(emb["embeddings"][0][1], 0.2);
    }

    #[tokio::test]
    async fn ollama_error_passes_through_with_status_and_message() {
        let inf = Inference::new(stub().await).unwrap();
        match inf.chat(&json!({"model": "nope", "messages": []})).await {
            Err(Error::Inference { status, message }) => {
                assert_eq!(status, 404);
                assert_eq!(message, "model 'nope' not found");
            }
            other => panic!("expected Inference, got {other:?}"),
        }
    }

    #[test]
    fn inference_error_falls_back_to_body_text_then_reason() {
        match inference_error(StatusCode::BAD_GATEWAY, b"<html>upstream</html>") {
            Error::Inference { status, message } => {
                assert_eq!(status, 502);
                assert_eq!(message, "<html>upstream</html>");
            }
            other => panic!("{other:?}"),
        }
        match inference_error(StatusCode::INTERNAL_SERVER_ERROR, b"") {
            Error::Inference { message, .. } => assert_eq!(message, "Internal Server Error"),
            other => panic!("{other:?}"),
        }
    }
}
