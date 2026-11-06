use axum::Router;

use crate::state::AppState;

pub mod channel;
pub mod data;
pub mod log;
pub mod token;

pub fn router() -> Router<AppState> {
    Router::new()
        .merge(token::router())
        .merge(channel::router())
        .merge(log::router())
        .merge(data::router())
}
