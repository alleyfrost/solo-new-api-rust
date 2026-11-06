use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub code: Option<String>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        Self { status, message: message.into(), code: None }
    }
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let body = json!({ "success": false, "message": self.message, "code": self.code });
        (self.status, axum::Json(body)).into_response()
    }
}

pub fn json_ok<T: serde::Serialize>(data: &T) -> Response {
    (StatusCode::OK, axum::Json(json!({ "success": true, "message": "", "data": data }))).into_response()
}
