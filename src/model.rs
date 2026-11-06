#[derive(Debug, Clone, sqlx::FromRow)]
pub struct UserRow {
    pub id: i64,
    pub username: String,
    pub password: String,
    pub display_name: Option<String>,
    pub role: i64,
    pub status: i64,
    pub group: String,
    pub auth_version: i64,
    pub email: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct TokenRow {
    pub id: i64,
    pub user_id: i64,
    pub key: String,
    pub status: i64,
    pub name: String,
    pub expired_time: i64,
    pub remain_quota: i64,
    pub unlimited_quota: bool,
    pub model_limits_enabled: bool,
    pub model_limits: String,
    pub allow_ips: Option<String>,
    pub group: String,
    pub auto_groups: Option<String>,
    pub cross_group_retry: bool,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct ChannelRow {
    pub id: i64,
    pub r#type: i64,
    pub key: String,
    pub status: i64,
    pub name: String,
    pub models: String,
    pub group: String,
    pub base_url: Option<String>,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
    pub channel_info: Option<String>,
    pub model_mapping: Option<String>,
    pub header_override: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct AbilityRow {
    pub group: String,
    pub model: String,
    pub channel_id: i64,
    pub enabled: bool,
    pub priority: Option<i64>,
    pub weight: Option<i64>,
}
