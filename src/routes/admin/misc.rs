use axum::Router;
use axum::extract::{Query, State};
use axum::routing::get;
use serde::Deserialize;
use serde_json::json;

use crate::error::json_ok;
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/perf-metrics/summary", get(perf_summary))
        .route("/api/option/model_pricing/issues", get(pricing_issues))
        .route("/api/task_plugin_options", get(task_plugin_options))
        .route("/api/prefill_group", get(prefill_groups))
        .route("/api/prefill_group/", get(prefill_groups))
}

async fn perf_summary() -> axum::response::Response {
    json_ok(&json!({ "models": [], "summary": {} }))
}

async fn pricing_issues() -> axum::response::Response {
    json_ok(&json!([]))
}

async fn task_plugin_options() -> axum::response::Response {
    json_ok(&json!([]))
}

#[derive(Deserialize)]
struct PrefillQuery {
    #[serde(rename = "type")]
    group_type: Option<String>,
}

#[derive(sqlx::FromRow)]
struct PrefillRow {
    id: i64,
    name: String,
    #[sqlx(rename = "type")]
    group_type: String,
    items: Option<String>,
    description: Option<String>,
    created_time: Option<i64>,
    updated_time: Option<i64>,
}

async fn prefill_groups(
    State(state): State<AppState>,
    Query(query): Query<PrefillQuery>,
) -> axum::response::Response {
    let rows: Vec<PrefillRow> = sqlx::query_as(
        "SELECT id, name, type, items, description, created_time, updated_time \
         FROM prefill_groups WHERE deleted_at IS NULL \
         AND (?1 IS NULL OR type = ?1) ORDER BY updated_time DESC",
    )
    .bind(query.group_type.as_deref())
    .fetch_all(&state.pool)
    .await
    .unwrap_or_default();
    let data: Vec<serde_json::Value> = rows
        .into_iter()
        .map(|row| {
            let items = row
                .items
                .as_deref()
                .and_then(|raw| serde_json::from_str::<serde_json::Value>(raw).ok())
                .unwrap_or_else(|| json!([]));
            json!({
                "id": row.id,
                "name": row.name,
                "type": row.group_type,
                "items": items,
                "description": row.description,
                "created_time": row.created_time.unwrap_or(0),
                "updated_time": row.updated_time.unwrap_or(0),
            })
        })
        .collect();
    json_ok(&data)
}
