//! `GET /agents` — the registry as synced. Lets `curl` confirm boot sync worked
//! and is the surface `mcp-fleet`'s `list_agents` will wrap later.

use super::AppState;
use crate::db::AgentRow;
use crate::error::Result;
use axum::extract::State;
use axum::Json;

pub async fn list(State(state): State<AppState>) -> Result<Json<Vec<AgentRow>>> {
    Ok(Json(state.db.agents()?))
}
