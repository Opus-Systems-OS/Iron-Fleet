//! Wire shape for the `rig-gpu` worker <-> Managed Agents self-hosted-environment
//! surface.
//!
//! **Unconfirmed.** Nothing in this repo has exercised these endpoints against
//! the live API yet (contrast `control-plane/src/anthropic`, which has fixtures
//! captured from a real run — see `control-plane/README.md`'s "first live
//! Managed Agents run" fix). This module is a best-effort match to the REST
//! conventions the rest of the Managed Agents surface already uses
//! (`/v1/<resource>`, `{"type": "...", ...}` bodies, `{"error": {"type",
//! "message"}}` on failure). Expect to patch endpoint paths and field names
//! here after the first real claim against a provisioned `rig-gpu`
//! environment, the same way stage 1 needed a follow-up fix commit.

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// A unit of work claimed from the environment's queue: one session's pending
/// tool calls, to run in one execution context (CLAUDE.md: "claims items,
/// spawns an execution context, runs tool calls, posts results back").
#[derive(Debug, Clone, Deserialize)]
pub struct Claim {
    pub claim_id: String,
    pub session_id: String,
    #[serde(default)]
    pub tool_calls: Vec<ToolCall>,
    /// How long this claim is held before Anthropic considers the worker gone
    /// and reassigns it. The worker heartbeats well inside this window.
    #[serde(default)]
    pub lease_seconds: Option<u64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub input: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct ToolResult {
    pub tool_use_id: String,
    pub content: String,
    pub is_error: bool,
}

#[derive(Debug, Serialize)]
pub struct SubmitResults<'a> {
    pub results: &'a [ToolResult],
}
