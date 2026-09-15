//! Live session events for the selected session (build order stage 4½ /
//! centralization Phase 3). The control plane proxies the Managed Agents SSE
//! stream byte-for-byte at `GET /sessions/{id}/stream`; this module tails it
//! and re-emits every event to the webview as a Tauri event, unchanged.
//!
//! The one piece of logic here is the reconnect rule from the Managed Agents
//! docs (events-and-streaming, "Streaming events"): only events emitted after
//! a stream opens are delivered, so on every (re)connect we open the stream
//! *first*, then list `GET /sessions/{id}/events` for history, then tail the
//! stream skipping ids the history already gave us. The `seen` set is the only
//! state, it belongs to one open watch, and it dies with it — nothing here is
//! fleet state (CLAUDE.md: "clients hold no fleet state").

use crate::config::ControlPlaneConfig;
use futures_util::StreamExt;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::time::Duration;
use tauri::{AppHandle, Emitter};

/// Emitted once per session event, history and live alike. `event` is the
/// Anthropic event object as the API sent it.
pub const EVENT: &str = "session-event";
/// Emitted on stream lifecycle changes so the UI can show a live/reconnecting pill.
pub const STATE: &str = "session-stream-state";

#[derive(Serialize, Clone)]
struct EventPayload<'a> {
    session_id: &'a str,
    event: Value,
}

#[derive(Serialize, Clone)]
struct StatePayload<'a> {
    session_id: &'a str,
    state: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    message: Option<String>,
}

const BACKOFF_START: Duration = Duration::from_secs(1);
const BACKOFF_MAX: Duration = Duration::from_secs(15);

/// Runs until aborted. `http` must have no total timeout (see `lib.rs`).
pub async fn watch(
    app: AppHandle,
    http: reqwest::Client,
    cfg: ControlPlaneConfig,
    session_id: String,
) {
    let mut backoff = BACKOFF_START;
    loop {
        match tail_once(&app, &http, &cfg, &session_id).await {
            Ok(()) => {
                // Upstream closed cleanly (e.g. control plane restarted). Reconnect.
                emit_state(&app, &session_id, "reconnecting", None);
                backoff = BACKOFF_START;
            }
            Err(Fatal(message)) => {
                emit_state(&app, &session_id, "error", Some(message));
                return;
            }
            Err(Transient(message)) => {
                emit_state(&app, &session_id, "reconnecting", Some(message));
            }
        }
        tokio::time::sleep(backoff).await;
        backoff = (backoff * 2).min(BACKOFF_MAX);
    }
}

use WatchError::*;

enum WatchError {
    /// The control plane said no (401, 404, 400): retrying won't change that.
    Fatal(String),
    /// Network / upstream hiccup: reconnect with backoff.
    Transient(String),
}

impl From<reqwest::Error> for WatchError {
    fn from(e: reqwest::Error) -> Self {
        Transient(e.to_string())
    }
}

async fn tail_once(
    app: &AppHandle,
    http: &reqwest::Client,
    cfg: &ControlPlaneConfig,
    session_id: &str,
) -> Result<(), WatchError> {
    // 1. Open the stream before anything else so no event falls in the gap.
    let resp = http
        .get(format!("{}/sessions/{}/stream", cfg.url, session_id))
        .bearer_auth(&cfg.token)
        .header("accept", "text/event-stream")
        .send()
        .await?;
    let resp = check(resp).await?;
    let mut body = resp.bytes_stream();

    // 2. History, oldest first, following next_page. Seeds `seen`.
    let mut seen: HashSet<String> = HashSet::new();
    let mut page: Option<String> = None;
    loop {
        let mut req = http
            .get(format!("{}/sessions/{}/events", cfg.url, session_id))
            .bearer_auth(&cfg.token);
        if let Some(p) = &page {
            req = req.query(&[("page", p)]);
        }
        let envelope: Value = check(req.send().await?).await?.json().await?;
        for event in envelope["data"].as_array().cloned().unwrap_or_default() {
            if let Some(id) = event["id"].as_str() {
                seen.insert(id.to_owned());
            }
            emit_event(app, session_id, event);
        }
        match envelope["next_page"].as_str() {
            Some(next) if !next.is_empty() => page = Some(next.to_owned()),
            _ => break,
        }
    }
    emit_state(app, session_id, "open", None);

    // 3. Tail live, skipping what history already delivered. Delta previews
    // (`event_start` / `event_delta`) carry no id of their own and always pass.
    let mut buf: Vec<u8> = Vec::new();
    while let Some(chunk) = body.next().await {
        buf.extend_from_slice(&chunk?);
        for data in parse_sse(&mut buf) {
            let event: Value = match serde_json::from_str(&data) {
                Ok(v) => v,
                Err(_) => continue, // not ours to interpret; skip a malformed frame
            };
            if let Some(id) = event["id"].as_str() {
                if !seen.insert(id.to_owned()) {
                    continue;
                }
            }
            emit_event(app, session_id, event);
        }
    }
    Ok(())
}

/// Maps a non-2xx control-plane response onto fatal vs. transient.
async fn check(resp: reqwest::Response) -> Result<reqwest::Response, WatchError> {
    let status = resp.status();
    if status.is_success() {
        return Ok(resp);
    }
    let body: Value = resp.json().await.unwrap_or(Value::Null);
    let message = body["error"]["message"]
        .as_str()
        .map(|m| format!("{status}: {m}"))
        .unwrap_or_else(|| status.to_string());
    if status.is_client_error() {
        Err(Fatal(message))
    } else {
        Err(Transient(message))
    }
}

fn emit_event(app: &AppHandle, session_id: &str, event: Value) {
    let _ = app.emit(EVENT, EventPayload { session_id, event });
}

fn emit_state(app: &AppHandle, session_id: &str, state: &'static str, message: Option<String>) {
    let _ = app.emit(
        STATE,
        StatePayload {
            session_id,
            state,
            message,
        },
    );
}

/// Pulls every complete SSE frame out of `buf`, leaving any partial trailing
/// frame in place for the next chunk. Returns each frame's joined `data`
/// (multiple `data:` lines join with `\n`, per the spec). Comments (`:`) and
/// other fields (`event:`, `id:`, `retry:`) are ignored — the Managed Agents
/// stream only uses `data:`.
pub fn parse_sse(buf: &mut Vec<u8>) -> Vec<String> {
    let mut frames = Vec::new();
    loop {
        // A frame ends at a blank line: "\n\n" or "\r\n\r\n".
        let Some((end, sep_len)) = find_frame_end(buf) else {
            return frames;
        };
        let frame = String::from_utf8_lossy(&buf[..end]).into_owned();
        buf.drain(..end + sep_len);
        let data: Vec<&str> = frame
            .lines()
            .filter_map(|line| line.strip_prefix("data:"))
            .map(|v| v.strip_prefix(' ').unwrap_or(v))
            .collect();
        if !data.is_empty() {
            frames.push(data.join("\n"));
        }
    }
}

fn find_frame_end(buf: &[u8]) -> Option<(usize, usize)> {
    let lf = buf.windows(2).position(|w| w == b"\n\n").map(|i| (i, 2));
    let crlf = buf
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .map(|i| (i, 4));
    match (lf, crlf) {
        (Some(a), Some(b)) => Some(if a.0 <= b.0 { a } else { b }),
        (a, b) => a.or(b),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_frame_per_blank_line() {
        let mut buf = b"data: {\"a\":1}\n\ndata: {\"b\":2}\n\n".to_vec();
        assert_eq!(parse_sse(&mut buf), vec!["{\"a\":1}", "{\"b\":2}"]);
        assert!(buf.is_empty());
    }

    #[test]
    fn partial_frame_waits_for_the_next_chunk() {
        let mut buf = b"data: {\"a\":1}\n\ndata: {\"b\"".to_vec();
        assert_eq!(parse_sse(&mut buf), vec!["{\"a\":1}"]);
        assert_eq!(buf, b"data: {\"b\"");
        buf.extend_from_slice(b":2}\n\n");
        assert_eq!(parse_sse(&mut buf), vec!["{\"b\":2}"]);
    }

    #[test]
    fn multi_line_data_joins_with_newline_and_other_fields_are_ignored() {
        let mut buf = b": keepalive\nevent: ping\nid: 7\ndata: one\ndata: two\n\n".to_vec();
        assert_eq!(parse_sse(&mut buf), vec!["one\ntwo"]);
    }

    #[test]
    fn crlf_frames_parse_too() {
        let mut buf = b"data: x\r\n\r\ndata: y\r\n\r\n".to_vec();
        assert_eq!(parse_sse(&mut buf), vec!["x", "y"]);
    }

    #[test]
    fn comment_only_frames_yield_nothing() {
        let mut buf = b": hi\n\n".to_vec();
        assert!(parse_sse(&mut buf).is_empty());
        assert!(buf.is_empty());
    }
}
