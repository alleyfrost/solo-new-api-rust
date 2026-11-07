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

#[derive(Deserialize)]
struct PerfQuery {
    hours: Option<i64>,
}

#[derive(Default)]
struct ModelAgg {
    request_count: i64,
    success_count: i64,
    total_latency_ms: i64,
    output_tokens: i64,
    generation_ms: i64,
}

fn rate(success: i64, total: i64) -> f64 {
    if total <= 0 {
        return 0.0;
    }
    (success as f64 * 10000.0 / total as f64).round() / 100.0
}

fn avg(total: i64, count: i64) -> f64 {
    if count <= 0 {
        return 0.0;
    }
    (total as f64 / count as f64 * 100.0).round() / 100.0
}

fn tps(tokens: i64, generation_ms: i64) -> f64 {
    if generation_ms <= 0 {
        return 0.0;
    }
    (tokens as f64 / (generation_ms as f64 / 1000.0) * 100.0).round() / 100.0
}

/// Relay 指标按小时桶累计，这里只做只读聚合；写入在 logging 的日志批里完成。
async fn perf_summary(
    State(state): State<AppState>,
    Query(query): Query<PerfQuery>,
) -> axum::response::Response {
    let hours = query.hours.unwrap_or(24).clamp(1, 24 * 30);
    let end = now();
    let start = end - hours * 3600;
    let rows = sqlx::query(
        "SELECT model_name, bucket_ts, request_count, success_count, total_latency_ms, output_tokens, generation_ms \
         FROM perf_metrics WHERE bucket_ts >= ? AND bucket_ts <= ? AND model_name <> '' \
         ORDER BY bucket_ts ASC",
    )
    .bind(start)
    .bind(end)
    .fetch_all(&state.pool)
    .await;
    let rows = match rows {
        Ok(rows) => rows,
        Err(_) => {
            return json_ok(&json!({
                "models": [], "summary": {}, "window_start": start, "window_end": end
            }));
        }
    };

    let mut models: std::collections::BTreeMap<String, ModelAgg> =
        std::collections::BTreeMap::new();
    let mut series: std::collections::HashMap<String, Vec<serde_json::Value>> =
        std::collections::HashMap::new();
    for row in &rows {
        use sqlx::Row;
        let name: String = row.get("model_name");
        let requests: i64 = row.get("request_count");
        let successes: i64 = row.get("success_count");
        let agg = models.entry(name.clone()).or_default();
        agg.request_count += requests;
        agg.success_count += successes;
        agg.total_latency_ms += row.get::<i64, _>("total_latency_ms");
        agg.output_tokens += row.get::<i64, _>("output_tokens");
        agg.generation_ms += row.get::<i64, _>("generation_ms");
        if requests > 0 {
            series.entry(name).or_default().push(json!({
                "ts": row.get::<i64, _>("bucket_ts"),
                "success_rate": rate(successes, requests),
            }));
        }
    }

    let mut totals = ModelAgg::default();
    let mut list: Vec<(String, ModelAgg)> = models.into_iter().collect();
    for (_, agg) in &list {
        totals.request_count += agg.request_count;
        totals.success_count += agg.success_count;
        totals.total_latency_ms += agg.total_latency_ms;
        totals.output_tokens += agg.output_tokens;
        totals.generation_ms += agg.generation_ms;
    }
    list.sort_by_key(|(_, agg)| std::cmp::Reverse(agg.request_count));
    let models: Vec<serde_json::Value> = list
        .into_iter()
        .take(12)
        .map(|(name, agg)| {
            json!({
                "model_name": name,
                "request_count": agg.request_count,
                "success_rate": rate(agg.success_count, agg.request_count),
                "avg_latency_ms": avg(agg.total_latency_ms, agg.request_count),
                "avg_tps": tps(agg.output_tokens, agg.generation_ms),
                "recent_success_series": series.remove(&name).unwrap_or_default(),
            })
        })
        .collect();

    json_ok(&json!({
        "models": models,
        "summary": {
            "success_rate": rate(totals.success_count, totals.request_count),
            "avg_latency_ms": avg(totals.total_latency_ms, totals.request_count),
            "avg_tps": tps(totals.output_tokens, totals.generation_ms),
        },
        "window_start": start,
        "window_end": end,
    }))
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
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
