use std::path::PathBuf;

use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

/// Serves the built frontend from `dist`, falling back to `index.html` so that
/// client-side routes (e.g. `/keys`) load the SPA shell.
///
/// Uses `fallback` rather than `not_found_service`: the latter forces the
/// fallback's status to 404, which would break SPA deep links routed by axum's
/// own `fallback_service`.
pub fn spa_service(dist: PathBuf) -> Router {
    let index = dist.join("index.html");
    Router::new().fallback_service(ServeDir::new(dist).fallback(ServeFile::new(index)))
}
