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

    #[tool(description = "Get a session's current status, usage, and budget.")]
    async fn get_session_status(
        &self,
        Parameters(SessionIdArgs { session_id }): Parameters<SessionIdArgs>,
    ) -> Result<String, String> {
        let id = valid_session_id(&session_id)?;
        self.client.get(&format!("/sessions/{id}")).await
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
