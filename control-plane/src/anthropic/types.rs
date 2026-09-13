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

// ---------------------------------------------------------- environments

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EnvironmentDefinition {
    pub name: String,
    pub config: Value,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Environment {
    pub id: String,
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
}

#[derive(Debug, Clone, Deserialize)]
pub struct Usage {
    #[serde(default)]
    pub list_cost: Option<MonetaryAmount>,
    #[serde(default)]
    pub input_tokens: Option<u64>,
    #[serde(default)]
    pub output_tokens: Option<u64>,
    #[serde(default)]
    pub active_seconds: Option<u64>,
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
        };
        let v = serde_json::to_value(&body).unwrap();
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
    fn effort_serializes_lowercase() {
        assert_eq!(serde_json::to_string(&Effort::Xhigh).unwrap(), "\"xhigh\"");
        let m: ModelConfig =
            serde_json::from_str(r#"{"id":"claude-opus-5","effort":"low"}"#).unwrap();
        assert_eq!(m.effort, Some(Effort::Low));
    }
}
