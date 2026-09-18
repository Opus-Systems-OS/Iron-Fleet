//! Managed Agents wire types. Typed only for the fields the control plane
//! consumes; unknown fields are ignored on the way in (serde default), and the
//! session read routes pass the raw JSON through rather than re-modelling it,
//! so a beta change on Anthropic's side does not break anything here.

use crate::money::Cents;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::collections::BTreeMap;

// ---------------------------------------------------------------- agents

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Effort {
    Low,
    Medium,
    High,
    Xhigh,
    Max,
}

impl Effort {
    pub fn as_str(self) -> &'static str {
        match self {
            Effort::Low => "low",
            Effort::Medium => "medium",
            Effort::High => "high",
            Effort::Xhigh => "xhigh",
            Effort::Max => "max",
        }
    }
}

/// `model` on an agent. Effort only takes effect here — a per-session override
/// silently drops it — which is why the registry carries it on the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModelConfig {
    pub id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub effort: Option<Effort>,
    #[serde(flatten)]
    pub rest: Map<String, Value>,
}

/// Body of `POST /v1/agents` and (minus nothing — full replace) `POST /v1/agents/{id}`.
///
/// The array fields are always serialized, empty or not. On update, Anthropic
/// *preserves* an omitted field and *clears* an explicit `[]` — so omitting an
/// empty `mcp_servers` after removing a server from the registry would leave
/// the old server on the live agent while `tools` lost its toolset, and the
/// update would 400 (`mcp_servers [x] declared but no mcp_toolset references
/// them`). Sending `[]` keeps the apply declarative.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentDefinition {
    pub name: String,
    pub model: ModelConfig,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default)]
    pub tools: Vec<Value>,
    #[serde(default)]
    pub mcp_servers: Vec<Value>,
    #[serde(default)]
    pub skills: Vec<Value>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Agent {
    pub id: String,
    pub version: u32,
}

// ---------------------------------------------------------------- skills
//
// The Skills API (`/v1/skills`) is GA — no beta header — and takes multipart
// uploads rather than JSON, so these are response types only; the request is
// built from a `registry::SkillDir` in `Client::create_skill*`.

/// `POST /v1/skills` response.
#[derive(Debug, Clone, Deserialize)]
pub struct Skill {
    pub id: String,
    /// What `version: "latest"` resolves to right now; pinned into the agent
    /// definition instead so a skill change rolls a new agent version.
    pub latest_version_id: String,
}

/// `POST /v1/skills/{id}/versions` response.
#[derive(Debug, Clone, Deserialize)]
pub struct SkillVersion {
    pub id: String,
    /// Immutable kebab-case slug from the first upload's frontmatter `name`;
    /// sync checks it still equals the directory name.
    pub name: String,
}

// ---------------------------------------------------------- environments

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentDefinition {
    pub name: String,
    pub config: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Environment {
    pub id: String,
    /// Only present on the create response for a `self_hosted` environment.
    /// Shown once; the control plane never stores or re-logs it (CLAUDE.md:
    /// "the rig's environment key stays on the rig").
    #[serde(default)]
    pub environment_key: Option<String>,
}

// -------------------------------------------------------------- sessions

/// The only currency the budget API accepts. A unit enum, so nothing else can be sent.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "UPPERCASE")]
pub enum Currency {
    Usd,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MonetaryAmount {
    pub amount: Cents,
    pub currency: Currency,
}

impl MonetaryAmount {
    pub fn usd(amount: Cents) -> Self {
        MonetaryAmount {
            amount,
            currency: Currency::Usd,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BudgetType {
    Limit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Budget {
    #[serde(rename = "type")]
    pub kind: BudgetType,
    pub max_list_cost: MonetaryAmount,
}

impl Budget {
    pub fn limit(cents: Cents) -> Self {
        Budget {
            kind: BudgetType::Limit,
            max_list_cost: MonetaryAmount::usd(cents),
        }
    }
}

/// Agent reference for session create: pinned, or pinned with session-local
/// overrides (docs: managed-agents/sessions, "Override agent configuration
/// for a session"). An override *replaces* the agent's field in full and
/// never touches the agent resource — so `tools` here is always the agent's
/// own list plus whatever the caller added, assembled by `sessions::create`.
#[derive(Debug, Clone, Serialize)]
pub struct AgentRef {
    #[serde(rename = "type")]
    pub kind: &'static str, // "agent" or "agent_with_overrides"
    pub id: String,
    pub version: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools: Option<Vec<Value>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub system: Option<String>,
}

impl AgentRef {
    pub fn pinned(id: impl Into<String>, version: u32) -> Self {
        AgentRef {
            kind: "agent",
            id: id.into(),
            version,
            tools: None,
            system: None,
        }
    }

    pub fn with_overrides(
        id: impl Into<String>,
        version: u32,
        tools: Option<Vec<Value>>,
        system: Option<String>,
    ) -> Self {
        AgentRef {
            kind: "agent_with_overrides",
            id: id.into(),
            version,
            tools,
            system,
        }
    }
}

/// A client-executed tool declared on one session (docs:
/// managed-agents/tools, "Custom tools"). The session emits
/// `agent.custom_tool_use` and waits (`requires_action`) for a
/// `user.custom_tool_result`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CustomTool {
    #[serde(rename = "type")]
    pub kind: String, // must be "custom"
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

#[derive(Debug, Clone, Serialize)]
pub struct TextBlock {
    #[serde(rename = "type")]
    pub kind: &'static str, // "text"
    pub text: String,
}

/// An event we send to a session (`initial_events` on create, or the body of
/// `POST /v1/sessions/{id}/events`). `Interrupt` is how a running turn is
/// stopped — there is no separate interrupt route (docs:
/// managed-agents/events-and-streaming, "Interrupting"). Only `Message` is
/// valid in `initial_events`; `sessions::create` never builds anything else.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type")]
pub enum SessionEvent {
    #[serde(rename = "user.message")]
    Message { content: Vec<TextBlock> },
    #[serde(rename = "user.interrupt")]
    Interrupt,
    /// The result of a client-executed custom tool, answering an
    /// `agent.custom_tool_use` event by its id.
    #[serde(rename = "user.custom_tool_result")]
    CustomToolResult {
        custom_tool_use_id: String,
        content: Vec<TextBlock>,
        #[serde(skip_serializing_if = "std::ops::Not::not")]
        is_error: bool,
    },
}

impl SessionEvent {
    pub fn text(text: impl Into<String>) -> Self {
        SessionEvent::Message {
            content: vec![TextBlock {
                kind: "text",
                text: text.into(),
            }],
        }
    }

    pub fn custom_tool_result(
        custom_tool_use_id: impl Into<String>,
        text: impl Into<String>,
        is_error: bool,
    ) -> Self {
        SessionEvent::CustomToolResult {
            custom_tool_use_id: custom_tool_use_id.into(),
            content: vec![TextBlock {
                kind: "text",
                text: text.into(),
            }],
            is_error,
        }
    }
}

/// Body of `POST /v1/sessions/{id}/events`. Shape mirrors `SessionCreate`'s
/// `initial_events`.
#[derive(Debug, Clone, Serialize)]
pub struct SendEvents {
    pub events: Vec<SessionEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionCreate {
    pub agent: AgentRef,
    pub environment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub budget: Budget,
    pub initial_events: Vec<SessionEvent>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    /// Vault ids to authenticate this session's MCP servers with — matched to
    /// an agent's `mcp_servers` entries by URL at runtime, not by name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub vault_ids: Vec<String>,
    /// Repositories cloned into the sandbox at session start
    /// (docs: managed-agents/github). Cloud environments only.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub resources: Vec<SessionResource>,
}

/// One `resources[]` entry. Only `github_repository` is modelled.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionResource {
    /// `url` is `https://github.com/<owner>/<repo>` (no `.git`); the repo
    /// lands at `/workspace/<repo>` since `mount_path` is left to default.
    GithubRepository {
        url: String,
        authorization_token: String,
    },
}

impl std::fmt::Debug for SessionResource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SessionResource::GithubRepository { url, .. } => f
                .debug_struct("GithubRepository")
                .field("url", url)
                .field("authorization_token", &"<redacted>")
                .finish(),
        }
    }
}

// ---------------------------------------------------------------- vaults

/// Body of `POST /v1/vaults`.
#[derive(Debug, Clone, Serialize)]
pub struct VaultCreate {
    pub display_name: String,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Vault {
    pub id: String,
}

/// A credential's `auth` object. Two of the three documented shapes: the
/// `static_bearer` that `mcp-fleet` needs, and `environment_variable` for
/// CLIs in the sandbox (`gh`, `wrangler`). OAuth is not implemented.
#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialAuth {
    StaticBearer {
        mcp_server_url: String,
        token: String,
    },
    /// The sandbox sees `secret_name` set to an opaque placeholder; the real
    /// value is substituted at egress, only on `allowed_hosts`, and only in
    /// request headers (`injection_location.header`) — the narrowest scope,
    /// and the only one `gh`/`wrangler` need. Body injection is deliberately
    /// not offered here.
    EnvironmentVariable {
        secret_name: String,
        secret_value: String,
        networking: CredentialNetworking,
        injection_location: InjectionLocation,
    },
}

// Secrets pass through this type; keep them out of `{:?}` output, which the
// tracing macros happily render.
impl std::fmt::Debug for CredentialAuth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredentialAuth::StaticBearer { mcp_server_url, .. } => f
                .debug_struct("StaticBearer")
                .field("mcp_server_url", mcp_server_url)
                .field("token", &"<redacted>")
                .finish(),
            CredentialAuth::EnvironmentVariable {
                secret_name,
                networking,
                ..
            } => f
                .debug_struct("EnvironmentVariable")
                .field("secret_name", secret_name)
                .field("secret_value", &"<redacted>")
                .field("networking", networking)
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialNetworking {
    Limited { allowed_hosts: Vec<String> },
}

#[derive(Debug, Clone, Copy, Serialize)]
pub struct InjectionLocation {
    pub header: bool,
}

impl InjectionLocation {
    pub const HEADER_ONLY: Self = Self { header: true };
}

/// Body of `POST /v1/vaults/{vault_id}/credentials`.
#[derive(Debug, Clone, Serialize)]
pub struct CredentialCreate {
    pub display_name: String,
    pub auth: CredentialAuth,
}

/// Body of `POST /v1/vaults/{vault_id}/credentials/{credential_id}` — a
/// rotation. `secret_name` is immutable; `networking` is a full replacement.
#[derive(Debug, Clone, Serialize)]
pub struct CredentialUpdate {
    pub auth: CredentialAuthUpdate,
}

#[derive(Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialAuthUpdate {
    StaticBearer {
        token: String,
    },
    EnvironmentVariable {
        secret_value: String,
        networking: CredentialNetworking,
    },
}

impl std::fmt::Debug for CredentialAuthUpdate {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CredentialAuthUpdate::StaticBearer { .. } => f
                .debug_struct("StaticBearer")
                .field("token", &"<redacted>")
                .finish(),
            CredentialAuthUpdate::EnvironmentVariable { networking, .. } => f
                .debug_struct("EnvironmentVariable")
                .field("secret_value", &"<redacted>")
                .field("networking", networking)
                .finish(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct Credential {
    pub id: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub list_cost: Option<MonetaryAmount>,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    /// Fractional seconds on the wire (e.g. `1.604`).
    #[serde(default)]
    pub active_seconds: Option<f64>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Session {
    pub id: String,
    pub status: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, String>,
    #[serde(default)]
    pub usage: Option<Usage>,
    #[serde(default)]
    pub budget: Option<Budget>,
}

// ---------------------------------------------------------------- errors

#[derive(Debug, Clone, Deserialize)]
pub struct ApiError {
    pub error: ApiErrorBody,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ApiErrorBody {
    #[serde(rename = "type")]
    pub kind: String,
    pub message: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_create_serializes_budget_amount_as_string() {
        let body = SessionCreate {
            agent: AgentRef::pinned("agent_1", 3),
            environment_id: "env_1".into(),
            title: Some("t".into()),
            budget: Budget::limit("50".parse().unwrap()),
            initial_events: vec![SessionEvent::text("hi")],
            metadata: BTreeMap::from([("iron_fleet_agent".into(), "jarvis".into())]),
            vault_ids: vec![],
            resources: vec![],
        };
        let v = serde_json::to_value(&body).unwrap();
        assert!(v.get("vault_ids").is_none(), "empty vault_ids is omitted");
        assert_eq!(
            v["agent"],
            serde_json::json!({"type": "agent", "id": "agent_1", "version": 3})
        );
        assert_eq!(v["budget"]["type"], "limit");
        assert_eq!(
            v["budget"]["max_list_cost"]["amount"],
            serde_json::Value::String("50".into())
        );
        assert_eq!(v["budget"]["max_list_cost"]["currency"], "USD");
        assert_eq!(v["initial_events"][0]["type"], "user.message");
        assert_eq!(v["initial_events"][0]["content"][0]["text"], "hi");
        assert_eq!(v["metadata"]["iron_fleet_agent"], "jarvis");
    }

    #[test]
    fn session_create_includes_nonempty_vault_ids() {
        let body = SessionCreate {
            agent: AgentRef::pinned("agent_1", 3),
            environment_id: "env_1".into(),
            title: None,
            budget: Budget::limit("50".parse().unwrap()),
            initial_events: vec![SessionEvent::text("hi")],
            metadata: BTreeMap::new(),
            vault_ids: vec!["vlt_1".into()],
            resources: vec![],
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["vault_ids"], serde_json::json!(["vlt_1"]));
    }

    /// Matches docs.claude.com/managed-agents/vaults's static_bearer example
    /// verbatim: `{"type": "static_bearer", "mcp_server_url": ..., "token": ...}`.
    #[test]
    fn static_bearer_credential_matches_documented_shape() {
        let body = CredentialCreate {
            display_name: "Iron-Fleet MCP".into(),
            auth: CredentialAuth::StaticBearer {
                mcp_server_url: "https://mcp-fleet.example/mcp".into(),
                token: "secret".into(),
            },
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(
            v["auth"],
            serde_json::json!({
                "type": "static_bearer",
                "mcp_server_url": "https://mcp-fleet.example/mcp",
                "token": "secret",
            })
        );
    }

    #[test]
    fn session_tolerates_unknown_fields() {
        let raw = serde_json::json!({
            "type": "session", "id": "sesn_1", "status": "idle",
            "metadata": {"iron_fleet_agent": "jarvis"},
            "usage": {"list_cost": {"amount": "53", "currency": "USD"}, "input_tokens": 10, "server_tool_use": {}},
            "budget": {"type": "limit", "max_list_cost": {"amount": "50", "currency": "USD"}},
            "environment_id": "env_1", "stats": {"active_seconds": 4}
        });
        let s: Session = serde_json::from_value(raw).unwrap();
        assert_eq!(
            s.usage.as_ref().unwrap().list_cost.unwrap().amount.get(),
            53
        );
        assert_eq!(s.budget.unwrap().max_list_cost.amount.get(), 50);
        assert_eq!(s.status, "idle");
    }

    #[test]
    fn real_session_response_deserializes() {
        // Captured from GET /v1/sessions/{id} on 2026-09-13; fresh sessions
        // report list_cost "0" and active_seconds is fractional.
        let raw = include_str!("fixtures/session_idle.json");
        let s: Session = serde_json::from_str(raw).unwrap();
        assert_eq!(s.status, "idle");
        assert_eq!(s.metadata["iron_fleet_agent"], "jarvis");
        let u = s.usage.unwrap();
        assert_eq!(u.list_cost.unwrap().amount.get(), 5);
        assert_eq!(u.active_seconds, Some(1.604));
        assert_eq!(s.budget.unwrap().max_list_cost.amount.get(), 50);

        let mut fresh: serde_json::Value = serde_json::from_str(raw).unwrap();
        fresh["usage"]["list_cost"]["amount"] = serde_json::json!("0");
        let s: Session = serde_json::from_value(fresh).unwrap();
        assert!(s.usage.unwrap().list_cost.unwrap().amount.is_zero());
    }

    #[test]
    fn agent_update_sends_empty_arrays_so_removals_apply() {
        // Regression: 2026-09-14 the live agent kept an mcp_servers entry the
        // registry had dropped, because the empty array was omitted and
        // Anthropic preserves omitted fields on update.
        let def: AgentDefinition = serde_json::from_str(
            r#"{"name":"A","model":{"id":"claude-opus-5"},"tools":[{"type":"agent_toolset_20260401"}]}"#,
        )
        .unwrap();
        let v = serde_json::to_value(&def).unwrap();
        assert_eq!(v["mcp_servers"], serde_json::json!([]));
        assert_eq!(v["skills"], serde_json::json!([]));
        assert_eq!(
            v["tools"],
            serde_json::json!([{"type":"agent_toolset_20260401"}])
        );
    }

    #[test]
    fn session_events_serialize_to_the_documented_wire_shapes() {
        assert_eq!(
            serde_json::to_value(SessionEvent::Interrupt).unwrap(),
            serde_json::json!({"type":"user.interrupt"})
        );
        assert_eq!(
            serde_json::to_value(SessionEvent::text("hi")).unwrap(),
            serde_json::json!({"type":"user.message","content":[{"type":"text","text":"hi"}]})
        );
        let body = SendEvents {
            events: vec![SessionEvent::Interrupt, SessionEvent::text("now this")],
        };
        let v = serde_json::to_value(&body).unwrap();
        assert_eq!(v["events"][0]["type"], "user.interrupt");
        assert_eq!(v["events"][1]["type"], "user.message");
    }

    #[test]
    fn effort_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Effort::Xhigh).unwrap(), "\"xhigh\"");
        let m: ModelConfig =
            serde_json::from_str(r#"{"id":"claude-opus-5","effort":"low"}"#).unwrap();
        assert_eq!(m.effort, Some(Effort::Low));
    }
}
