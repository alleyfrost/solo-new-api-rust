use axum::Router;

use crate::state::AppState;

pub mod channel;
pub mod data;
pub mod log;
pub mod misc;
pub mod models;
pub mod token;
pub mod user;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(token::router())
        .merge(channel::router())
        .merge(log::router())
        .merge(data::router())
        .merge(models::router())
        .merge(misc::router())
        .merge(user::router())
}
