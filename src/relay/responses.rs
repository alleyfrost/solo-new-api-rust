use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use axum::response::Response;

use crate::auth::token::TokenContext;
use crate::relay::forward::{self, Wire};
use crate::state::AppState;

pub async fn responses(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    forward::relay(state, ctx, headers, body, Wire::Responses).await
}
