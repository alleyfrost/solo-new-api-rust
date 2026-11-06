use axum::response::Response;
use serde_json::json;
use crate::error::json_ok;

pub async fn get_status() -> Response {
    let data = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "start_time": crate::START_TIME,
        "system_name": "solo-new-api",
        "self_use_mode_enabled": true,
        "password_login_enabled": true,
        "password_login_encryption_enabled": false,
        "quota_display_type": "CNY",
        "display_in_currency": true,
        "quota_per_unit": 500000,
        "register_enabled": false,
        "password_register_enabled": false,
    });
    json_ok(&data)
}

pub async fn get_notice() -> Response {
    json_ok(&"")
}

pub async fn get_group() -> Response {
    json_ok(&serde_json::json!({ "default": { "desc": "", "ratio": 1 }, "vip": { "desc": "", "ratio": 1 } }))
}
