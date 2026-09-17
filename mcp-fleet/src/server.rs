//! The MCP surface itself: exactly the five tools CLAUDE.md allows jarvis —
//! `list_agents`, `start_session`, `get_session_status`, `send_event`,
//! `interrupt_session` — and nothing else. This is a closed list on purpose:
//! CLAUDE.md is explicit that agent creation, environment creation, budget
//! mutation, or anything that raises a spending limit must never be exposed
//! here. Jarvis dispatches work; it does not define the fleet or change its
//! own constraints. Adding a sixth tool is a CLAUDE.md decision, not a code
//! change to make casually.

use crate::client::ControlPlaneClient;
use rmcp::handler::server::router::tool::ToolRouter;
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{ServerCapabilities, ServerInfo};
use rmcp::{tool, tool_handler, tool_router, ServerHandler};
use schemars::JsonSchema;
use serde::Deserialize;
use serde_json::{json, Value};

#[derive(Debug, Deserialize, JsonSchema)]
pub struct StartSessionArgs {
    /// Registry slug of the agent to run, e.g. "jarvis" or "blueweb-client".
    /// See `list_agents` for the current fleet.
    pub agent_slug: String,
    /// The task for the agent to do — becomes its first message.
    pub task: String,
    /// Environment slug to run in. Omit to use the agent's default
    /// environment; only set this to deliberately override it.
    #[serde(default)]
    pub environment: Option<String>,
    /// Extra GitHub repositories to clone into the sandbox for this session,
    /// as `https://github.com/<owner>/<repo>` URLs — e.g. a customer's site
    /// repo for a change request. The agent's registry defaults are always
    /// mounted; only agents with GitHub access configured accept this.
    #[serde(default)]
    pub repositories: Vec<String>,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SessionIdArgs {
    /// A session id returned by `start_session`, e.g. "sesn_...".
    pub session_id: String,
}

#[derive(Debug, Deserialize, JsonSchema)]
pub struct SendEventArgs {
    /// The session to send this message to.
    pub session_id: String,
    /// The follow-up message to append to the running session.
    pub task: String,
}

/// What jarvis gets back from `get_session_status`: the handful of fields
/// it can act on, plus the session's latest reply so it can relay results.
/// Deliberately not the raw session object — that embeds the whole agent
/// definition (system prompt included) and the session's `vault_ids`,
/// neither of which jarvis has any use for, and it is ~2 KB per call
/// against a 50 ¢ cap.
///
/// `session` is `GET /sessions/{id}`; `latest` is
/// `GET /sessions/{id}/events?order=desc&types=agent.message&limit=1`.
fn session_view(session: &Value, latest: &Value) -> Value {
    let last_reply = latest["data"]
        .as_array()
        .and_then(|d| d.first())
        .and_then(|ev| ev["content"].as_array())
        .map(|blocks| {
            blocks
                .iter()
                .filter_map(|b| b["text"].as_str())
                .collect::<Vec<_>>()
                .join("")
        })
        .filter(|s| !s.is_empty());
    json!({
        "session_id": session["id"],
        "status": session["status"],
        "title": session["title"],
        "agent_slug": session["metadata"]["iron_fleet_agent"],
        "environment": session["metadata"]["iron_fleet_environment"],
        "cap_cents": session["budget"]["max_list_cost"]["amount"],
        "spent_cents": session["usage"]["list_cost"]["amount"],
        "active_seconds": session["usage"]["active_seconds"],
        "created_at": session["created_at"],
        "updated_at": session["updated_at"],
        "last_reply": last_reply,
        "console_url": session["console_url"],
    })
}

fn valid_session_id(id: &str) -> Result<&str, String> {
    let ok = !id.is_empty()
        && id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-');
    if ok {
        Ok(id)
    } else {
        Err("invalid session_id".to_owned())
    }
}

#[derive(Clone)]
pub struct FleetServer {
    client: ControlPlaneClient,
    // The tool_handler macro reads this field through generated code that
    // rustc's dead-code analysis doesn't see as a use.
    #[allow(dead_code)]
    tool_router: ToolRouter<Self>,
}

impl FleetServer {
    pub fn new(client: ControlPlaneClient) -> Self {
        FleetServer {
            client,
            tool_router: Self::tool_router(),
        }
    }
}

#[tool_router]
impl FleetServer {
    #[tool(
        description = "List every agent in the Iron-Fleet registry: slug, per-session cost cap, effort, and default environment."
    )]
    async fn list_agents(&self) -> Result<String, String> {
        self.client.get("/agents").await
    }

    #[tool(
        description = "Start a new session for an agent with a task. The agent's registered cost cap applies automatically — this cannot be changed here. Returns the new session id and its console URL."
    )]
    async fn start_session(
        &self,
        Parameters(args): Parameters<StartSessionArgs>,
    ) -> Result<String, String> {
        let body = serde_json::json!({
            "agent_slug": args.agent_slug,
            "task": args.task,
            "environment": args.environment,
            "repositories": args.repositories,
        });
        self.client.post("/sessions", &body).await
    }

    #[tool(
        description = "Get a session's status (running/idle/terminated), what it has spent against its cap in cents, and its latest reply (last_reply) — use this to check on a session you started and relay what it said."
    )]
    async fn get_session_status(
        &self,
        Parameters(SessionIdArgs { session_id }): Parameters<SessionIdArgs>,
    ) -> Result<String, String> {
        let id = valid_session_id(&session_id)?;
        let session = self.client.get_json(&format!("/sessions/{id}")).await?;
        let latest = self
            .client
            .get_json(&format!(
                "/sessions/{id}/events?order=desc&types=agent.message&limit=1"
            ))
            .await?;
        Ok(session_view(&session, &latest).to_string())
    }

    #[tool(description = "Send a follow-up message to a running session.")]
    async fn send_event(
        &self,
        Parameters(SendEventArgs { session_id, task }): Parameters<SendEventArgs>,
    ) -> Result<String, String> {
        let id = valid_session_id(&session_id)?;
        let body = serde_json::json!({ "task": task });
        self.client
            .post(&format!("/sessions/{id}/events"), &body)
            .await
    }

    #[tool(
        description = "Stop a session's in-flight work without ending the session. Use this if a session is doing something it shouldn't."
    )]
    async fn interrupt_session(
        &self,
        Parameters(SessionIdArgs { session_id }): Parameters<SessionIdArgs>,
    ) -> Result<String, String> {
        let id = valid_session_id(&session_id)?;
        self.client
            .post_empty(&format!("/sessions/{id}/interrupt"))
            .await
    }
}

#[tool_handler]
impl ServerHandler for FleetServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build()).with_instructions(
            "Iron-Fleet's dispatch surface: list agents, start a session for one, check on it, \
             send it a follow-up, or interrupt it. There is no tool to create agents, create \
             environments, or change a cap — the fleet's shape is defined in the agents/ registry, \
             not from here.",
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// CLAUDE.md's closed list. If this test has to change, that is a
    /// CLAUDE.md change first.
    #[test]
    fn exactly_the_five_jarvis_tools() {
        let mut names: Vec<String> = FleetServer::tool_router()
            .list_all()
            .into_iter()
            .map(|t| t.name.to_string())
            .collect();
        names.sort();
        assert_eq!(
            names,
            [
                "get_session_status",
                "interrupt_session",
                "list_agents",
                "send_event",
                "start_session",
            ]
        );
    }

    #[test]
    fn session_ids_are_path_safe_only() {
        assert!(valid_session_id("sesn_014mScwJSHMwi4xXjX26RGEs").is_ok());
        assert!(valid_session_id("").is_err());
        assert!(valid_session_id("sesn_1/../agents").is_err());
        assert!(valid_session_id("sesn 1").is_err());
        assert!(valid_session_id("sesn_1?x=1").is_err());
    }

    #[test]
    fn session_view_is_compact_and_carries_the_last_reply() {
        let session = json!({
            "id": "sesn_1",
            "status": "idle",
            "title": "Run nvidia-smi",
            "agent": {"system": "SECRET PROMPT", "id": "agent_1"},
            "vault_ids": ["vlt_1"],
            "metadata": {"iron_fleet_agent": "gpu-compute", "iron_fleet_environment": "rig-gpu"},
            "budget": {"max_list_cost": {"amount": "500", "currency": "USD"}, "type": "limit"},
            "usage": {"active_seconds": 11.9, "list_cost": {"amount": "6", "currency": "USD"}},
            "created_at": "2026-09-17T14:23:02Z",
            "updated_at": "2026-09-17T14:42:03Z",
            "console_url": "https://platform.claude.com/x/sesn_1"
        });
        let latest = json!({"data": [{"type": "agent.message", "content": [
            {"type": "text", "text": "The driver "}, {"type": "text", "text": "is 616.92."}
        ]}]});
        let v = session_view(&session, &latest);
        assert_eq!(v["agent_slug"], "gpu-compute");
        assert_eq!(v["environment"], "rig-gpu");
        assert_eq!(v["cap_cents"], "500");
        assert_eq!(v["spent_cents"], "6");
        assert_eq!(v["last_reply"], "The driver is 616.92.");
        let text = v.to_string();
        assert!(!text.contains("SECRET PROMPT"));
        assert!(!text.contains("vlt_1"));
    }

    #[test]
    fn session_view_without_a_reply_yet() {
        let v = session_view(
            &json!({"id": "sesn_1", "status": "running"}),
            &json!({"data": []}),
        );
        assert_eq!(v["status"], "running");
        assert!(v["last_reply"].is_null());
        assert!(v["spent_cents"].is_null());
    }
}
