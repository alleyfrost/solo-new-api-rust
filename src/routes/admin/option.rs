use axum::Json;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use serde::Deserialize;
use serde_json::{Value, json};

use crate::error::{ApiError, json_ok};
use crate::settings::{self, Settings};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new()
        .route("/api/option", get(list).put(update))
        .route("/api/option/", get(list).put(update))
}

#[derive(Deserialize)]
struct UpdateBody {
    key: String,
    #[serde(default)]
    value: Value,
}

async fn list(State(state): State<AppState>) -> Result<axum::response::Response, ApiError> {
    let entries = Settings::load(&state).await.entries();
    let data: Vec<Value> = entries
        .into_iter()
        .map(|(key, value)| json!({ "key": key, "value": value }))
        .collect();
    Ok(json_ok(&data))
}

async fn update(
    State(state): State<AppState>,
    Json(body): Json<UpdateBody>,
) -> Result<axum::response::Response, ApiError> {
    let value = settings::normalize(&body.key, &body.value)
        .map_err(|message| ApiError::new(StatusCode::OK, message))?;
    sqlx::query(
        "INSERT INTO options (`key`, value) VALUES (?, ?) \
         ON CONFLICT(`key`) DO UPDATE SET value = excluded.value",
    )
    .bind(&body.key)
    .bind(&value)
    .execute(&state.pool)
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    state
        .reload_caches()
        .await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    Ok(json_ok(&json!({ "key": body.key, "value": value })))
}
