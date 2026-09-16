//! `/inference/*`: the rig's Ollama behind the control plane's bearer token
//! (Phase 6). Registered by `http::router` only when `INFERENCE_URL` is set;
//! otherwise these paths are axum's default 404 and the service is
//! unchanged. Bodies are validated for shape and passed through — `options`,
//! `keep_alive`, `format`, `tools` all reach Ollama untouched.
//!
//! Rig off → `503 rig_offline`. That is the whole failover story: nothing
//! here, or anywhere else, retries against Claude.

use super::AppState;
use crate::error::{Error, Result};
use crate::inference::Inference;
use axum::body::Body;
use axum::extract::{DefaultBodyLimit, State};
use axum::http::{header, StatusCode};
use axum::response::Response;
use axum::routing::{get, post};
use axum::{Json, Router};
use serde_json::Value;

/// The three routes, with a 1 MiB body cap — a chat request is a few KB of
/// messages, an embedding input rarely more; a bigger body is a mistake.
pub fn router() -> Router<AppState> {
    Router::new()
        .route("/inference/models", get(models))
        .route("/inference/chat", post(chat))
        .route("/inference/embeddings", post(embeddings))
        .route_layer(DefaultBodyLimit::max(1 << 20))
}

fn backend(state: &AppState) -> Result<&Inference> {
    state
        .inference
        .as_ref()
        .as_ref()
        .ok_or_else(|| Error::Config("inference routes registered without INFERENCE_URL".into()))
}

/// `GET /inference/models`: Ollama's `/api/tags` JSON unchanged.
pub async fn models(State(state): State<AppState>) -> Result<Json<Value>> {
    let tags = backend(&state)?.models().await?;
    Ok(Json(tags))
}

/// `POST /inference/chat`: `{model, messages, ...}` → `/api/chat`. Streams
/// NDJSON unless the body says `"stream": false`, in which case Ollama's
/// single JSON object is buffered and returned as such.
pub async fn chat(State(state): State<AppState>, Json(body): Json<Value>) -> Result<Response> {
    validate(&body, "messages", Value::is_array, "an array")?;
    let streaming = body["stream"].as_bool().unwrap_or(true);
    let inf = backend(&state)?;
    let upstream = inf.chat(&body).await?;
    tracing::info!(
        model = body["model"].as_str().unwrap_or(""),
        streaming,
        "inference chat"
    );

    if !streaming {
        let bytes = upstream.bytes().await.map_err(|e| inf.classify(e))?;
        return Response::builder()
            .status(StatusCode::OK)
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(bytes))
            .map_err(|e| Error::Config(format!("chat response: {e}")));
    }
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/x-ndjson")
        .header(header::CACHE_CONTROL, "no-cache")
        // Same as `sessions::stream`: buffering proxies pass frames through.
        .header("x-accel-buffering", "no")
        .body(Body::from_stream(upstream.bytes_stream()))
        .map_err(|e| Error::Config(format!("chat stream response: {e}")))
}

/// `POST /inference/embeddings`: `{model, input, ...}` → `/api/embed`, buffered.
pub async fn embeddings(
    State(state): State<AppState>,
    Json(body): Json<Value>,
) -> Result<Json<Value>> {
    validate(
        &body,
        "input",
        |v| v.is_string() || v.is_array(),
        "a string or an array",
    )?;
    let out = backend(&state)?.embeddings(&body).await?;
    Ok(Json(out))
}

/// Shape check only: an object with a non-empty string `model` and a `field`
/// that passes `ok`. Everything else is Ollama's to judge.
fn validate(body: &Value, field: &str, ok: fn(&Value) -> bool, expected: &str) -> Result<()> {
    let obj = body
        .as_object()
        .ok_or_else(|| Error::InvalidRequest("body must be a JSON object".into()))?;
    match obj.get("model").and_then(Value::as_str) {
        Some(m) if !m.trim().is_empty() => {}
        _ => {
            return Err(Error::InvalidRequest(
                "`model` must be a non-empty string".into(),
            ))
        }
    }
    match obj.get(field) {
        Some(v) if ok(v) => Ok(()),
        _ => Err(Error::InvalidRequest(format!(
            "`{field}` must be {expected}"
        ))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn chat_ok(body: &Value) -> Result<()> {
        validate(body, "messages", Value::is_array, "an array")
    }

    #[test]
    fn chat_body_needs_model_and_messages() {
        assert!(chat_ok(&json!({"model": "qwen3:8b", "messages": []})).is_ok());
        assert!(chat_ok(&json!({"model": "qwen3:8b", "messages": [], "stream": false, "options": {"temperature": 0}})).is_ok());
        for bad in [
            json!([]),
            json!("x"),
            json!({"messages": []}),
            json!({"model": "", "messages": []}),
            json!({"model": 3, "messages": []}),
            json!({"model": "qwen3:8b"}),
            json!({"model": "qwen3:8b", "messages": "hi"}),
        ] {
            match chat_ok(&bad) {
                Err(Error::InvalidRequest(_)) => {}
                other => panic!("{bad}: {other:?}"),
            }
        }
    }

    #[test]
    fn embeddings_input_is_string_or_array() {
        let ok = |b: &Value| validate(b, "input", |v| v.is_string() || v.is_array(), "x");
        assert!(ok(&json!({"model": "nomic-embed-text", "input": "hello"})).is_ok());
        assert!(ok(&json!({"model": "nomic-embed-text", "input": ["a", "b"]})).is_ok());
        assert!(matches!(
            ok(&json!({"model": "nomic-embed-text", "input": 1})),
            Err(Error::InvalidRequest(_))
        ));
        assert!(matches!(
            ok(&json!({"model": "nomic-embed-text"})),
            Err(Error::InvalidRequest(_))
        ));
    }
}
