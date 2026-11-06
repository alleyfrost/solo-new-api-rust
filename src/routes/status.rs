use crate::error::json_ok;
use crate::state::AppState;
use axum::extract::State;
use axum::response::Response;
use serde_json::json;

pub async fn get_status() -> Response {
    let data = json!({
        "version": env!("CARGO_PKG_VERSION"),
        "start_time": crate::START_TIME.get().copied().unwrap_or(0),
        "system_name": "solo-new-api",
        "self_use_mode_enabled": true,
        "password_login_enabled": true,
        "password_login_encryption_enabled": false,
        "quota_display_type": "CNY",
        "display_in_currency": true,
        "quota_per_unit": 500000,
        "register_enabled": false,
        "password_register_enabled": false,
        "HeaderNavModules": {
            "home": false,
            "console": true,
            "pricing": {"enabled": false, "requireAuth": false},
            "rankings": {"enabled": false, "requireAuth": false},
            "docs": false,
            "about": false
        }
    });
    json_ok(&data)
}

pub async fn get_setup() -> Response {
    json_ok(&json!({
        "status": true,
        "root_init": true,
        "database_type": "sqlite"
    }))
}

pub async fn get_notice() -> Response {
    json_ok(&"")
}

pub async fn get_group(State(state): State<AppState>) -> Response {
    let groups: Vec<(String,)> =
        sqlx::query_as("SELECT DISTINCT `group` FROM abilities ORDER BY `group`")
            .fetch_all(&state.pool)
            .await
            .unwrap_or_default();
    if groups.is_empty() {
        return json_ok(&vec!["default".to_string()]);
    }
    let names: Vec<String> = groups.into_iter().map(|(g,)| g).collect();
    json_ok(&names)
}
