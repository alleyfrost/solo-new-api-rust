use axum::Router;

use crate::state::AppState;

pub mod channel;
pub mod channel_ops;
pub mod data;
pub mod log;
pub mod misc;
pub mod models;
pub mod option;
pub mod token;
pub mod user;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(token::router())
        .merge(channel::router())
        .merge(channel_ops::router())
        .merge(log::router())
        .merge(data::router())
        .merge(models::router())
        .merge(option::router())
        .merge(misc::router())
        .merge(user::router())
}
