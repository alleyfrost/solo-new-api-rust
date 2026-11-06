use axum::Router;

use crate::state::AppState;

pub mod token;

pub fn router() -> Router<AppState> {
    Router::new().merge(token::router())
}
