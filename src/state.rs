use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, RwLock};
use sqlx::SqlitePool;
use crate::config::Config;
use crate::model::{AbilityRow, ChannelRow, TokenRow, UserRow};

pub struct Caches {
    pub users: HashMap<i64, UserRow>,
    pub tokens: HashMap<String, TokenRow>,
    pub channels: HashMap<i64, ChannelRow>,
    pub abilities: HashMap<(String, String), Vec<AbilityRow>>,
    pub options: HashMap<String, String>,
}

impl Caches {
    pub async fn load(pool: &SqlitePool) -> Result<Self, sqlx::Error> {
        let users: Vec<UserRow> = sqlx::query_as(
            "SELECT id, username, password, display_name, role, status, `group`, auth_version, email FROM users WHERE deleted_at IS NULL"
        ).fetch_all(pool).await?;
        let tokens: Vec<TokenRow> = sqlx::query_as(
            "SELECT id, user_id, `key`, status, name, created_time, accessed_time, expired_time, remain_quota, unlimited_quota, model_limits_enabled, model_limits, allow_ips, `group`, auto_groups, cross_group_retry, used_quota, allow_user_quota_query FROM tokens WHERE deleted_at IS NULL"
        ).fetch_all(pool).await?;
        let channels: Vec<ChannelRow> = sqlx::query_as(
            "SELECT id, type, `key`, status, name, models, `group`, base_url, priority, weight, channel_info, model_mapping, header_override, created_time, test_time, response_time, open_ai_organization, test_model, other, balance, balance_updated_time, used_quota, status_code_mapping, auto_ban, other_info, tag, setting, param_override, remark, settings FROM channels"
        ).fetch_all(pool).await?;
        let abilities: Vec<AbilityRow> = sqlx::query_as(
            "SELECT `group`, model, channel_id, enabled, priority, weight FROM abilities"
        ).fetch_all(pool).await?;
        let options: Vec<(String, String)> = sqlx::query_as("SELECT `key`, value FROM options")
            .fetch_all(pool).await?;

        let mut abilities_map: HashMap<(String, String), Vec<AbilityRow>> = HashMap::new();
        for a in abilities {
            abilities_map.entry((a.group.clone(), a.model.clone())).or_default().push(a);
        }
        Ok(Self {
            users: users.into_iter().map(|u| (u.id, u)).collect(),
            tokens: tokens.into_iter().map(|t| (t.key.clone(), t)).collect(),
            channels: channels.into_iter().map(|c| (c.id, c)).collect(),
            abilities: abilities_map,
            options: options.into_iter().collect(),
        })
    }

    pub fn token_by_key(&self, key: &str) -> Option<TokenRow> {
        self.tokens.get(key).cloned()
    }
}

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: SqlitePool,
    pub caches: Arc<RwLock<Caches>>,
    pub jwt_secret: Arc<Vec<u8>>,
    pub client: reqwest::Client,
    pub routing: Arc<crate::routing::RoutingState>,
    pub log_tx: Option<tokio::sync::mpsc::Sender<crate::logging::LogEntry>>,
    /// 正在导出日志的用户；同一用户同时只允许一个导出。
    pub export_slots: Arc<Mutex<HashSet<i64>>>,
    pub body_gate: Arc<crate::mem::BodyGate>,
    pub rate_limits: Arc<crate::ratelimit::RateLimitRegistry>,
}

impl AppState {
    pub fn new(
        config: Config,
        pool: SqlitePool,
        caches: Caches,
        jwt_secret: Vec<u8>,
        log_tx: Option<tokio::sync::mpsc::Sender<crate::logging::LogEntry>>,
    ) -> Self {
        let settings = crate::settings::Settings::from_options(&caches.options);
        Self {
            config,
            pool,
            caches: Arc::new(RwLock::new(caches)),
            jwt_secret: Arc::new(jwt_secret),
            client: crate::upstream::build_client(),
            routing: Arc::new(crate::routing::RoutingState::new()),
            log_tx,
            export_slots: Arc::new(Mutex::new(HashSet::new())),
            body_gate: Arc::new(crate::mem::BodyGate::new(
                settings.body_gate_big,
                settings.body_gate_huge,
            )),
            rate_limits: Arc::new(crate::ratelimit::RateLimitRegistry::default()),
        }
    }

    pub async fn reload_caches(&self) -> Result<(), sqlx::Error> {
        let fresh = Caches::load(&self.pool).await?;
        let settings = crate::settings::Settings::from_options(&fresh.options);
        *self.caches.write().expect("cache lock") = fresh;
        self.body_gate
            .set_limits(settings.body_gate_big, settings.body_gate_huge);
        Ok(())
    }
}
