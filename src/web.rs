use std::path::PathBuf;

use axum::body::Body;
use axum::extract::Request;
use axum::Router;
use axum::http::header::{CACHE_CONTROL, HeaderValue};
use axum::http::{Method, StatusCode};
use axum::response::{IntoResponse, Response};
use serde_json::json;
use tower::ServiceExt;
use tower_http::services::{ServeDir, ServeFile};

/// Router whose fallback serves the frontend; merge or mount it at the root.
pub fn spa_service(dist: PathBuf) -> Router {
    Router::new().fallback(move |req| {
        let dist = dist.clone();
        async move { serve(dist, req).await }
    })
}

/// Serves the built frontend from `dist`, falling back to `index.html` for SPA
/// client routes. Unknown `/api`, `/v1`, `/mj` and `/pg` paths return JSON 404
/// instead of the SPA shell, otherwise clients would parse HTML as JSON.
pub async fn serve(dist: PathBuf, req: Request) -> Response {
    let path = req.uri().path().to_string();
    if ["/api", "/v1", "/mj", "/pg"]
        .iter()
        .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
    {
        return (
            StatusCode::NOT_FOUND,
            axum::Json(json!({ "success": false, "message": "not found" })),
        )
            .into_response();
    }
    if req.method() != Method::GET && req.method() != Method::HEAD {
        return StatusCode::METHOD_NOT_ALLOWED.into_response();
    }
    let index = dist.join("index.html");
    let service = ServeDir::new(dist).fallback(ServeFile::new(index));
    match service.oneshot(req).await {
        Ok(res) => {
            let mut response = res.map(Body::new).into_response();
            // 内容哈希的构建产物可长期缓存；入口、图标等固定路径必须每次回源校验。
            let cache = if path.starts_with("/static/") {
                "public, max-age=31536000, immutable"
            } else {
                "no-cache"
            };
            response
                .headers_mut()
                .insert(CACHE_CONTROL, HeaderValue::from_static(cache));
            response
        }
        Err(_) => StatusCode::INTERNAL_SERVER_ERROR.into_response(),
    }
}
