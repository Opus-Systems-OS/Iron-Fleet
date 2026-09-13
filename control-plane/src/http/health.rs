use axum::Json;
use serde_json::{json, Value};

pub async fn healthz() -> Json<Value> {
    Json(
        json!({ "ok": true, "service": "iron-fleet-control-plane", "version": env!("CARGO_PKG_VERSION") }),
    )
}
