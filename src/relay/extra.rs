use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use axum::response::Response;

use crate::auth::token::TokenContext;
use crate::relay::passthrough;
use crate::state::AppState;

pub async fn embeddings(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    passthrough::relay(state, ctx, headers, body, "/v1/embeddings").await
}

pub async fn image_generations(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    passthrough::relay(state, ctx, headers, body, "/v1/images/generations").await
}

pub async fn image_edits(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    passthrough::relay(state, ctx, headers, body, "/v1/images/edits").await
}

pub async fn audio_transcriptions(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    passthrough::relay(state, ctx, headers, body, "/v1/audio/transcriptions").await
}

pub async fn audio_translations(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    passthrough::relay(state, ctx, headers, body, "/v1/audio/translations").await
}

pub async fn audio_speech(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    passthrough::relay(state, ctx, headers, body, "/v1/audio/speech").await
}

pub async fn rerank(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    passthrough::relay(state, ctx, headers, body, "/v1/rerank").await
}

pub async fn completions(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    passthrough::relay(state, ctx, headers, body, "/v1/completions").await
}

pub async fn moderations(
    State(state): State<AppState>,
    Extension(ctx): Extension<TokenContext>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    passthrough::relay(state, ctx, headers, body, "/v1/moderations").await
}
