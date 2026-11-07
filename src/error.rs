use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde_json::json;

/// `message_key` is the English source text the frontend looks up in its locale
/// files; `message` stays rendered English for non-browser API callers.
#[derive(Debug)]
pub struct ApiError {
    pub status: StatusCode,
    pub message: String,
    pub code: Option<String>,
    pub message_key: Option<String>,
}

impl ApiError {
    pub fn new(status: StatusCode, message: impl Into<String>) -> Self {
        let message = message.into();
        Self {
            status,
            code: None,
            message_key: None,
            message,
        }
    }
    pub fn with_code(mut self, code: impl Into<String>) -> Self {
        self.code = Some(code.into());
        self
    }
    pub fn with_message_key(mut self, key: impl Into<String>) -> Self {
        self.message_key = Some(key.into());
        self
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let mut body = json!({ "success": false, "message": self.message, "code": self.code });
        if let Some(key) = self.message_key {
            body["message_key"] = json!(key);
        }
        (self.status, axum::Json(body)).into_response()
    }
}

pub fn json_ok<T: serde::Serialize>(data: &T) -> Response {
    (
        StatusCode::OK,
        axum::Json(json!({ "success": true, "message": "", "data": data })),
    )
        .into_response()
}
