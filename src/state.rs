use std::collections::HashMap;
use std::sync::{Arc, RwLock};
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
            "SELECT id, user_id, `key`, status, name, expired_time, remain_quota, unlimited_quota, model_limits_enabled, model_limits, allow_ips, `group`, auto_groups, cross_group_retry FROM tokens WHERE deleted_at IS NULL"
        ).fetch_all(pool).await?;
        let channels: Vec<ChannelRow> = sqlx::query_as(
            "SELECT id, type, `key`, status, name, models, `group`, base_url, priority, weight, channel_info FROM channels"
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
}

impl AppState {
    pub fn new(
        config: Config,
        pool: SqlitePool,
        caches: Caches,
        jwt_secret: Vec<u8>,
        log_tx: Option<tokio::sync::mpsc::Sender<crate::logging::LogEntry>>,
    ) -> Self {
        Self {
            config,
            pool,
            caches: Arc::new(RwLock::new(caches)),
            jwt_secret: Arc::new(jwt_secret),
            client: crate::upstream::build_client(),
            routing: Arc::new(crate::routing::RoutingState::new()),
            log_tx,
        }
    }
}
