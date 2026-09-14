//! Execution context: the per-session working directory and tool dispatch that
//! actually runs on the rig. This is the one piece of the agent loop that is
//! supposed to live locally (CLAUDE.md: Anthropic keeps the loop; the worker
//! only runs tool calls) — nothing here decides *what* to do next, it only
//! executes what a claim says to run and reports back.
//!
//! Tool names/inputs are matched against `agent_toolset_20260401` (the toolset
//! `agents/gpu-compute.json` grants) on a best-guess basis, same caveat as
//! `protocol.rs`: `bash` and file read/write are the shapes every Claude
//! toolset has had so far, but the exact input schema for this beta hasn't
//! been confirmed against a live run.

use crate::protocol::{ToolCall, ToolResult};
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;
use tokio::io::AsyncReadExt;
use tokio::process::Command;

/// Cap on how much tool output we ship back in one result. A runaway `bash`
/// call (e.g. an uncapped training log) must not take down the claim.
const MAX_OUTPUT_BYTES: usize = 200_000;

pub struct ExecutionContext {
    dir: PathBuf,
}

impl ExecutionContext {
    /// One directory per session, reused across claims for the same session so
    /// files written by an earlier tool call are still there for a later one.
    pub fn for_session(workdir: &Path, session_id: &str) -> std::io::Result<Self> {
        let safe: String = session_id
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dir = workdir.join(safe);
        std::fs::create_dir_all(&dir)?;
        Ok(ExecutionContext { dir })
    }

    pub async fn run_tool(&self, call: &ToolCall, timeout: Duration) -> ToolResult {
        let result = match call.name.as_str() {
            "bash" | "shell" => self.run_bash(call, timeout).await,
            "read_file" => self.read_file(call),
            "write_file" => self.write_file(call),
            other => Err(format!(
                "unsupported tool on rig-gpu worker: `{other}` (only bash/shell, read_file, write_file \
                 are implemented — see worker/src/exec.rs)"
            )),
        };
        match result {
            Ok(content) => ToolResult {
                tool_use_id: call.id.clone(),
                content: truncate(content),
                is_error: false,
            },
            Err(message) => ToolResult {
                tool_use_id: call.id.clone(),
                content: truncate(message),
                is_error: true,
            },
        }
    }

    async fn run_bash(&self, call: &ToolCall, timeout: Duration) -> Result<String, String> {
        let command = call
            .input
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "bash tool call missing string `input.command`".to_owned())?;

        let mut child = Command::new("bash")
            .arg("-lc")
            .arg(command)
            .current_dir(&self.dir)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("spawn bash: {e}"))?;

        let mut stdout = child.stdout.take().expect("piped");
        let mut stderr = child.stderr.take().expect("piped");
        let read_all = async {
            let mut out = Vec::new();
            let mut err = Vec::new();
            let _ = tokio::join!(stdout.read_to_end(&mut out), stderr.read_to_end(&mut err));
            (out, err)
        };

        let ((out, err), status) = tokio::time::timeout(timeout, async {
            let output = read_all.await;
            let status = child.wait().await;
            (output, status)
        })
        .await
        .map_err(|_| {
            let _ = child.start_kill();
            format!("command timed out after {}s", timeout.as_secs())
        })?;

        let status = status.map_err(|e| format!("wait bash: {e}"))?;
        let mut combined = String::from_utf8_lossy(&out).into_owned();
        if !err.is_empty() {
            combined.push_str("\n--- stderr ---\n");
            combined.push_str(&String::from_utf8_lossy(&err));
        }
        if !status.success() {
            combined.push_str(&format!("\n--- exit status: {} ---", status));
        }
        Ok(combined)
    }

    fn resolve_relative(&self, call: &ToolCall) -> Result<PathBuf, String> {
        let rel = call
            .input
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "tool call missing string `input.path`".to_owned())?;
        let joined = self.dir.join(rel);
        // Cheap traversal guard: reject `..` components rather than canonicalizing,
        // since write_file's target may not exist yet.
        if joined
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        {
            return Err(format!("path escapes the session working directory: {rel}"));
        }
        Ok(joined)
    }

    fn read_file(&self, call: &ToolCall) -> Result<String, String> {
        let path = self.resolve_relative(call)?;
        std::fs::read_to_string(&path).map_err(|e| format!("read {}: {e}", path.display()))
    }

    fn write_file(&self, call: &ToolCall) -> Result<String, String> {
        let path = self.resolve_relative(call)?;
        let content = call
            .input
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "write_file call missing string `input.content`".to_owned())?;
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|e| format!("mkdir {}: {e}", parent.display()))?;
        }
        std::fs::write(&path, content).map_err(|e| format!("write {}: {e}", path.display()))?;
        Ok(format!(
            "wrote {} bytes to {}",
            content.len(),
            path.display()
        ))
    }
}

fn truncate(mut s: String) -> String {
    if s.len() > MAX_OUTPUT_BYTES {
        s.truncate(MAX_OUTPUT_BYTES);
        s.push_str("\n--- truncated at 200000 bytes ---");
    }
    s
}
