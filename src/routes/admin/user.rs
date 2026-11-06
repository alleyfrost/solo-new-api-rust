use axum::Router;
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::Response;
use axum::routing::get;
use serde_json::json;

use crate::error::{ApiError, json_ok};
use crate::state::AppState;

pub fn router() -> Router<AppState> {
    Router::new().route("/api/user/{id}", get(get_user))
}

#[derive(sqlx::FromRow)]
struct UserDetail {
    id: i64,
    username: String,
    display_name: Option<String>,
    quota: Option<i64>,
    used_quota: Option<i64>,
    request_count: Option<i64>,
    #[sqlx(rename = "group")]
    group_name: Option<String>,
    aff_code: Option<String>,
    aff_count: Option<i64>,
    aff_quota: Option<i64>,
    remark: Option<String>,
}

async fn get_user(
    State(state): State<AppState>,
    Path(id): Path<i64>,
) -> Result<Response, ApiError> {
    let user: Option<UserDetail> = sqlx::query_as(
        "SELECT id, username, display_name, quota, used_quota, request_count, `group`, \
         aff_code, aff_count, aff_quota, remark FROM users WHERE id=? AND deleted_at IS NULL",
    )
    .bind(id)
    .fetch_optional(&state.pool)
    .await
    .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let Some(user) = user else {
        return Err(ApiError::new(StatusCode::NOT_FOUND, "user not found"));
    };
    Ok(json_ok(&json!({
        "id": user.id,
        "username": user.username,
        "display_name": user.display_name,
        "quota": user.quota.unwrap_or(0),
        "used_quota": user.used_quota.unwrap_or(0),
        "request_count": user.request_count.unwrap_or(0),
        "group": user.group_name,
        "aff_code": user.aff_code,
        "aff_count": user.aff_count.unwrap_or(0),
        "aff_quota": user.aff_quota.unwrap_or(0),
        "remark": user.remark,
    })))
}
