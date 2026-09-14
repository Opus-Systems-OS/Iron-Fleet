//! `GET /usage` — the Usage tab. Built entirely from `session_usage`, the
//! cumulative rollup upserted by the webhook handler; nothing here calls
//! Anthropic. A session with no webhook delivered yet (still running, or
//! the webhook hasn't landed) is simply absent until one arrives — same
//! staleness the table has always had (control-plane/README.md).

use super::AppState;
use crate::db::{AgentUsageRow, UsageRow};
use crate::error::Result;
use axum::extract::State;
use axum::Json;
use serde::Serialize;

const RECENT_LIMIT: u32 = 100;

#[derive(Debug, Serialize)]
pub struct UsageResponse {
    pub by_agent: Vec<AgentUsageRow>,
    pub recent: Vec<UsageRow>,
}

pub async fn get(State(state): State<AppState>) -> Result<Json<UsageResponse>> {
    Ok(Json(UsageResponse {
        by_agent: state.db.usage_by_agent()?,
        recent: state.db.usage_rows(RECENT_LIMIT)?,
    }))
}
