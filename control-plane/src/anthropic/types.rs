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
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub mcp_servers: Vec<Value>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
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

/// Pinned agent reference for session create.
#[derive(Debug, Clone, Serialize)]
pub struct AgentRef {
    #[serde(rename = "type")]
    pub kind: &'static str, // always "agent"
    pub id: String,
    pub version: u32,
}

impl AgentRef {
    pub fn pinned(id: impl Into<String>, version: u32) -> Self {
        AgentRef {
            kind: "agent",
            id: id.into(),
            version,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TextBlock {
    #[serde(rename = "type")]
    pub kind: &'static str, // "text"
    pub text: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct UserMessageEvent {
    #[serde(rename = "type")]
    pub kind: &'static str, // "user.message"
    pub content: Vec<TextBlock>,
}

impl UserMessageEvent {
    pub fn text(text: impl Into<String>) -> Self {
        UserMessageEvent {
            kind: "user.message",
            content: vec![TextBlock {
                kind: "text",
                text: text.into(),
            }],
        }
    }
}

/// Body of `POST /v1/sessions/{id}/events`. Shape mirrors `SessionCreate`'s
/// `initial_events`. **Unconfirmed** — unlike the rest of this file, nothing
/// has exercised this against the live API yet; see `Client::send_events`.
#[derive(Debug, Clone, Serialize)]
pub struct SendEvents {
    pub events: Vec<UserMessageEvent>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SessionCreate {
    pub agent: AgentRef,
    pub environment_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    pub budget: Budget,
    pub initial_events: Vec<UserMessageEvent>,
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: BTreeMap<String, String>,
    /// Vault ids to authenticate this session's MCP servers with — matched to
    /// an agent's `mcp_servers` entries by URL at runtime, not by name.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub vault_ids: Vec<String>,
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

/// A credential's `auth` object. Only `static_bearer` is implemented — the
/// one shape `mcp-fleet` needs (a fixed bearer token, not OAuth).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum CredentialAuth {
    StaticBearer {
        mcp_server_url: String,
        token: String,
    },
}

/// Body of `POST /v1/vaults/{vault_id}/credentials`.
#[derive(Debug, Clone, Serialize)]
pub struct CredentialCreate {
    pub display_name: String,
    pub auth: CredentialAuth,
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
            initial_events: vec![UserMessageEvent::text("hi")],
            metadata: BTreeMap::from([("iron_fleet_agent".into(), "jarvis".into())]),
            vault_ids: vec![],
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
            initial_events: vec![UserMessageEvent::text("hi")],
            metadata: BTreeMap::new(),
            vault_ids: vec!["vlt_1".into()],
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
    fn effort_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Effort::Xhigh).unwrap(), "\"xhigh\"");
        let m: ModelConfig =
            serde_json::from_str(r#"{"id":"claude-opus-5","effort":"low"}"#).unwrap();
        assert_eq!(m.effort, Some(Effort::Low));
    }
}
