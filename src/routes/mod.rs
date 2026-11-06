use axum::Router;
use crate::state::AppState;

pub mod admin;
pub mod models;
pub mod status;
pub mod user;

pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/api/status", axum::routing::get(status::get_status))
        .route("/api/notice", axum::routing::get(status::get_notice))
        .route("/api/group", axum::routing::get(status::get_group))
}
