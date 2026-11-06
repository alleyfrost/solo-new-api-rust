# M1: 骨架 + 数据层 + 认证 实施计划

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 一个可运行的 Rust 二进制：打开现有 `one-api.db`、校验 schema、提供 `/api/status` 健康检查、管理员登录/刷新/登出、转发 Token 认证中间件与 `/v1/models` 冒烟。

**Architecture:** axum + tokio + sqlx(SQLite/WAL)；进程内缓存加载 users/tokens/channels/abilities/options；argon2 登录 + JWT access + refresh cookie + `user_sessions`；Token 认证按 new-api 语义解析 `sk-`。

**Tech Stack:** Rust 1.96.0, axum 0.8, sqlx 0.8, argon2 0.5, jsonwebtoken 9, tower-http 0.6, tracing.

**Spec:** `docs/superpowers/specs/2026-11-06-solo-new-api-rust-design.md`

## Global Constraints

- 工具链锁 `rust-toolchain.toml` channel `1.96.0`。
- 直接读写现有 SQLite，不建表、不迁移；启动校验必需表/列，缺失退出码 2。
- 未知列/JSON 字段保留；只读列不解析。
- 中文沟通；验证通过即 git commit；不用 `cargo fmt --all`（仅限定文件）。
- 测试用本地 `tests/fixtures/seed.sql` 构造的临时库，保持 hermetic。
- 不引入 Redis；所有状态进程内。

---

## 文件结构

- `rust-toolchain.toml` — 锁定 1.96.0。
- `Cargo.toml` — 依赖与 crate 元数据。
- `src/main.rs` — 入口：加载配置、初始化 DB/缓存/服务、启动 axum、优雅停机。
- `src/config.rs` — `Config`（环境变量）。
- `src/db.rs` — 连接池、PRAGMA、schema 校验、仓储读取。
- `src/state.rs` — `AppState` 与内存缓存（users/tokens/channels/abilities/options）。
- `src/error.rs` — `ApiError` 与响应包辅助。
- `src/auth/mod.rs` — 公共导出。
- `src/auth/token.rs` — 转发 Token 认证中间件。
- `src/auth/session.rs` — 管理登录/会话/JWT/refresh/logout。
- `src/auth/admin.rs` — 管理中间件。
- `src/routes/mod.rs` — 路由装配。
- `src/routes/status.rs` — `/api/status`、`/api/notice`、`/api/group`。
- `src/routes/user.rs` — `/api/user/self`、`/self/groups`、`/user/models`、login/refresh/logout。
- `src/routes/models.rs` — `/v1/models`、`/v1/models/:model`。
- `tests/fixtures/seed.sql` — 测试库 schema+数据。
- `tests/integration_m1.rs` — 端到端集成测试。

---

## Task 1: Cargo 骨架与 /api/status

**Files:**
- Create: `rust-toolchain.toml`, `Cargo.toml`, `src/lib.rs`, `src/main.rs`, `src/config.rs`, `src/state.rs`, `src/routes/mod.rs`, `src/routes/status.rs`, `src/error.rs`
- Test: `tests/integration_m1.rs`

**Interfaces:**
- Produces: `Config::from_env() -> Config`（字段 `port:u16, db_path:String, web_dist:String, jwt_secret_path:String, trust_proxy_headers:bool`）；`AppState { pool, config }`（后续扩展）；`ApiError` 与 `json_ok(data)`、`json_err(status,msg)`；路由 `build_router(state)`。

- [ ] **Step 1: 写 `rust-toolchain.toml`**

```toml
[toolchain]
channel = "1.96.0"
```

- [ ] **Step 2: 写 `Cargo.toml`**

```toml
[package]
name = "solo-new-api-rust"
version = "0.1.0"
edition = "2024"
rust-version = "1.96.0"

[dependencies]
tokio = { version = "1", features = ["full"] }
axum = { version = "0.8", features = ["macros"] }
tower-http = { version = "0.6", features = ["cors", "fs", "compression-gzip", "decompression-gzip", "trace"] }
serde = { version = "1", features = ["derive"] }
serde_json = { version = "1", features = ["preserve_order"] }
sqlx = { version = "0.8", features = ["sqlite", "runtime-tokio", "macros"] }
argon2 = "0.5"
jsonwebtoken = "9"
uuid = { version = "1", features = ["v4"] }
sha2 = "0.10"
rand = "0.8"
regex = "1"
tracing = "0.1"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }
tower = "0.5"
hyper = "1"

[dev-dependencies]
tower = { version = "0.5", features = ["util"] }
http-body-util = "0.1"
```

- [ ] **Step 3: 写 `src/config.rs`**

```rust
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct Config {
    pub port: u16,
    pub db_path: PathBuf,
    pub web_dist: PathBuf,
    pub jwt_secret_path: PathBuf,
    pub trust_proxy_headers: bool,
    pub max_request_bytes: usize,
}

impl Config {
    pub fn from_env() -> Self {
        Self {
            port: env_or("PORT", "3000").parse().unwrap_or(3000),
            db_path: PathBuf::from(env_or("DB_PATH", "/data/one-api.db")),
            web_dist: PathBuf::from(env_or("WEB_DIST", "./web/dist")),
            jwt_secret_path: PathBuf::from(env_or("JWT_SECRET_PATH", "/data/.rust_jwt_secret")),
            trust_proxy_headers: env_or("TRUST_PROXY_HEADERS", "true") == "true",
            max_request_bytes: env_or("MAX_REQUEST_BYTES", "67108864").parse().unwrap_or(67_108_864),
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
```

- [ ] **Step 4: 写 `src/error.rs`**

```rust
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
```

- [ ] **Step 5: 写 `src/routes/status.rs`**

```rust
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
```

- [ ] **Step 6: 写 `src/routes/mod.rs`**

```rust
use axum::Router;
use crate::state::AppState;

pub mod status;

pub fn public_router() -> Router<AppState> {
    Router::new()
        .route("/api/status", axum::routing::get(status::get_status))
        .route("/api/notice", axum::routing::get(status::get_notice))
        .route("/api/group", axum::routing::get(status::get_group))
}
```

- [ ] **Step 7: 写 `src/lib.rs` 与 `src/main.rs`**

`src/lib.rs`：

```rust
pub mod config;
pub mod error;
pub mod routes;
pub mod state;

pub const START_TIME: i64 = 0; // Task 8 替换为 OnceLock
```

`src/main.rs`：

```rust
use solo_new_api_rust::{config, routes, state};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let config = config::Config::from_env();
    let state = state::AppState {
        config: config.clone(),
        pool: sqlx::SqlitePool::connect(&format!("sqlite://{}?mode=rwc", config.db_path.display())).await?,
    };
    let app = routes::public_router().with_state(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    tracing::info!("listening on :{}", config.port);
    axum::serve(listener, app).await?;
    Ok(())
}
```

注：Task 2 会替换连接逻辑为 `db::connect` 并加入 schema 校验；Task 8 完善 `START_TIME` 与优雅停机。

- [ ] **Step 7b: 写 `src/state.rs` 最小版**

```rust
use crate::config::Config;

#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: sqlx::SqlitePool,
}
```

- [ ] **Step 8: 写失败集成测试 `tests/integration_m1.rs`**

```rust
use axum::body::Body;
use http_body_util::BodyExt;
use tower::ServiceExt;

#[tokio::test]
async fn status_returns_success() {
    let cfg = solo_new_api_rust::config::Config {
        port: 0,
        db_path: std::path::PathBuf::from(":memory:"),
        web_dist: std::path::PathBuf::from("./web/dist"),
        jwt_secret_path: std::path::PathBuf::from("/tmp/.jwt-test"),
        trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let pool = sqlx::SqlitePool::connect("sqlite::memory:").await.unwrap();
    let state = solo_new_api_rust::state::AppState { config: cfg, pool };
    let app = solo_new_api_rust::routes::public_router().with_state(state);
    let res = app.oneshot(
        axum::http::Request::builder().uri("/api/status").body(Body::empty()).unwrap()
    ).await.unwrap();
    assert_eq!(res.status(), 200);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(v["success"], true);
    assert_eq!(v["data"]["self_use_mode_enabled"], true);
}
```

- [ ] **Step 9: 运行测试**

Run: `cargo test --test integration_m1`
Expected: PASS。

- [ ] **Step 10: 提交**

```bash
git add rust-toolchain.toml Cargo.toml src tests
git commit -m "feat(m1): cargo skeleton with /api/status"
```

---

## Task 2: SQLite 连接与 schema 校验

**Files:**
- Create: `src/db.rs`
- Modify: `src/main.rs`, `src/lib.rs`（新增，供集成测试引用）
- Test: `tests/schema_validation.rs`

**Interfaces:**
- Produces: `db::connect(path: &Path) -> Result<SqlitePool, sqlx::Error>`（WAL/NORMAL/busy_timeout/foreign_keys/temp_store）；`db::validate_schema(&SqlitePool) -> Result<(), SchemaError>`；`SchemaError` 含缺失表/列列表。

- [ ] **Step 1: 写失败测试 `tests/schema_validation.rs`**

```rust
use sqlx::SqlitePool;

#[tokio::test]
async fn rejects_empty_database() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    let err = solo_new_api_rust::db::validate_schema(&pool).await.unwrap_err();
    let msg = err.to_string();
    assert!(msg.contains("users"), "missing tables should be reported: {msg}");
}

#[tokio::test]
async fn accepts_minimum_schema() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    for sql in [
        "CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, display_name TEXT, email TEXT)",
        "CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, expired_time INTEGER, `group` TEXT, unlimited_quota INTEGER, remain_quota INTEGER, used_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, auto_groups TEXT, cross_group_retry INTEGER, deleted_at TEXT)",
        "CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, setting TEXT, param_override TEXT, header_override TEXT, channel_info TEXT, settings TEXT)",
        "CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER, tag TEXT)",
        "CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)",
        "CREATE TABLE user_sessions (sid TEXT PRIMARY KEY, user_id INTEGER, status TEXT, refresh_hash TEXT, expires_at INTEGER, version INTEGER, user_auth_version INTEGER)",
    ] {
        sqlx::query(sql).execute(&pool).await.unwrap();
    }
    solo_new_api_rust::db::validate_schema(&pool).await.unwrap();
}
```

- [ ] **Step 2: 运行确认失败**

Run: `cargo test --test schema_validation`
Expected: 编译失败（`solo_new_api_rust::db` 不存在）。

- [ ] **Step 3: 写 `src/lib.rs`**

```rust
pub mod config;
pub mod db;
pub mod error;
pub mod routes;
pub mod state;
```

`src/main.rs` 改为 `use solo_new_api_rust::{config, routes, state};`，删除本地 `mod` 声明。

- [ ] **Step 4: 写 `src/db.rs`**

```rust
use std::fmt;
use std::path::Path;
use sqlx::sqlite::SqliteConnectOptions;
use sqlx::{ConnectOptions, SqlitePool};

#[derive(Debug)]
pub struct SchemaError {
    pub missing: Vec<String>,
}

impl fmt::Display for SchemaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "database schema mismatch, missing: {}", self.missing.join(", "))
    }
}
impl std::error::Error for SchemaError {}

pub async fn connect(path: &Path) -> Result<SqlitePool, sqlx::Error> {
    let opts = SqliteConnectOptions::new()
        .filename(path)
        .create_if_missing(false)
        .journal_mode(sqlx::sqlite::SqliteJournalMode::Wal)
        .synchronous(sqlx::sqlite::SqliteSynchronous::Normal)
        .busy_timeout(std::time::Duration::from_millis(5000))
        .foreign_keys(true)
        .disable_statement_logging();
    SqlitePool::connect_with(opts).await
}

pub async fn validate_schema(pool: &SqlitePool) -> Result<(), SchemaError> {
    const REQUIRED: &[(&str, &[&str])] = &[
        ("users", &["id", "username", "password", "role", "status", "group", "auth_version"]),
        ("tokens", &["id", "user_id", "key", "status", "expired_time", "group", "unlimited_quota", "deleted_at"]),
        ("channels", &["id", "type", "key", "status", "models", "group", "priority", "weight"]),
        ("abilities", &["group", "model", "channel_id", "enabled", "priority", "weight"]),
        ("options", &["key", "value"]),
        ("user_sessions", &["sid", "user_id", "status", "refresh_hash", "expires_at", "version", "user_auth_version"]),
    ];
    let mut missing = Vec::new();
    for (table, columns) in REQUIRED {
        let exists: Option<i64> = sqlx::query_scalar(
            "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?"
        ).bind(table).fetch_optional(pool).await.unwrap_or(None);
        if exists.is_none() {
            missing.push(format!("table {table}"));
            continue;
        }
        for column in *columns {
            let found: Option<i64> = sqlx::query_scalar(
                "SELECT 1 FROM pragma_table_info(?) WHERE name=?"
            ).bind(table).bind(column).fetch_optional(pool).await.unwrap_or(None);
            if found.is_none() {
                missing.push(format!("{table}.{column}"));
            }
        }
    }
    if missing.is_empty() { Ok(()) } else { Err(SchemaError { missing }) }
}
```

- [ ] **Step 5: 在 `main.rs` 接入**

替换 Task 1 的直接 `SqlitePool::connect`：

```rust
let pool = db::connect(&config.db_path).await?;
db::validate_schema(&pool).await.map_err(|e| {
    eprintln!("{e}");
    std::process::exit(2);
})?;
let state = state::AppState { config: config.clone(), pool };
```

- [ ] **Step 6: 运行测试**

Run: `cargo test --test schema_validation`
Expected: 2 passed。

- [ ] **Step 7: 提交**

```bash
git add src tests
git commit -m "feat(m1): sqlite connection and schema validation"
```

---

## Task 3: 行类型与内存缓存加载

**Files:**
- Create: `src/model.rs`, `src/state.rs`（替换 Task 1 的占位）
- Modify: `src/lib.rs`, `src/main.rs`, `tests/integration_m1.rs`（AppState 新增 caches 字段）
- Test: `tests/state_cache.rs`

**Interfaces:**
- Produces: `model::{UserRow, TokenRow, ChannelRow, AbilityRow}`；`state::Caches { users, tokens, channels, abilities, options }`；`Caches::load(&SqlitePool) -> Result<Caches, sqlx::Error>`；`Caches::token_by_key(&str) -> Option<TokenRow>`；`AppState { config, pool, caches: Arc<Caches> }`。

- [ ] **Step 1: 写 `src/model.rs`**

```rust
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
```

- [ ] **Step 2: 写 `src/state.rs`**

```rust
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
}
```

- [ ] **Step 3: `src/lib.rs` 增加 `pub mod model;`；`main.rs` 构造缓存**

```rust
let caches = state::Caches::load(&pool).await?;
let state = state::AppState { config: config.clone(), pool, caches: std::sync::Arc::new(std::sync::RwLock::new(caches)) };
```

同步修改 `tests/integration_m1.rs` 的 `AppState` 构造，加入空缓存：

```rust
caches: std::sync::Arc::new(std::sync::RwLock::new(solo_new_api_rust::state::Caches {
    users: Default::default(), tokens: Default::default(), channels: Default::default(),
    abilities: Default::default(), options: Default::default(),
})),
```

- [ ] **Step 4: 写测试 `tests/state_cache.rs`**

```rust
use sqlx::SqlitePool;

#[tokio::test]
async fn loads_users_tokens_channels_and_abilities() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE tokens (id INTEGER PRIMARY KEY, user_id INTEGER, `key` TEXT, status INTEGER, name TEXT, expired_time INTEGER, remain_quota INTEGER, unlimited_quota INTEGER, model_limits_enabled INTEGER, model_limits TEXT, allow_ips TEXT, `group` TEXT, auto_groups TEXT, cross_group_retry INTEGER, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE channels (id INTEGER PRIMARY KEY, type INTEGER, `key` TEXT, status INTEGER, name TEXT, models TEXT, `group` TEXT, base_url TEXT, priority INTEGER, weight INTEGER, channel_info TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE abilities (`group` TEXT, model TEXT, channel_id INTEGER, enabled INTEGER, priority INTEGER, weight INTEGER)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE options (`key` TEXT PRIMARY KEY, value TEXT)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO users VALUES (1,'root','hash','Root',100,1,'default',1,NULL,NULL)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO tokens VALUES (2,1,'abc123',1,'t',-1,0,1,0,'','', 'vip',NULL,0,NULL)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO channels VALUES (3,1,'sk-up',1,'c','m1,m2','vip',NULL,0,0,'{}')").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO abilities VALUES ('vip','m1',3,1,0,0)").execute(&pool).await.unwrap();
    sqlx::query("INSERT INTO options VALUES ('RetryTimes','10')").execute(&pool).await.unwrap();

    let caches = solo_new_api_rust::state::Caches::load(&pool).await.unwrap();
    assert_eq!(caches.users.len(), 1);
    assert_eq!(caches.token_by_key("abc123").unwrap().group, "vip");
    assert_eq!(caches.channels.len(), 1);
    assert_eq!(caches.abilities[&("vip".to_string(), "m1".to_string())].len(), 1);
    assert_eq!(caches.options["RetryTimes"], "10");
}
```

- [ ] **Step 5: 运行测试**

Run: `cargo test --test state_cache`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src tests
git commit -m "feat(m1): row types and in-memory caches"
```

---

## Task 4: 转发 Token 认证中间件

**Files:**
- Create: `src/auth/mod.rs`, `src/auth/token.rs`
- Modify: `src/lib.rs`
- Test: `tests/token_auth.rs`

**Interfaces:**
- Produces: `auth::token::TokenContext { user_id:i64, token_id:i64, token_group:String }`（作为 request extension）；`auth::token::token_auth(State, Request, Next) -> Result<Response, ApiError>`；`auth::token::extract_key(&HeaderMap, &Uri) -> Option<String>`；`auth::token::validate_token(&Caches, &str) -> Result<TokenContext, ApiError>`。

- [ ] **Step 1: 写 `src/auth/token.rs`**

```rust
use axum::extract::{Request, State};
use axum::http::{HeaderMap, StatusCode, Uri};
use axum::middleware::Next;
use axum::response::Response;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct TokenContext {
    pub user_id: i64,
    pub token_id: i64,
    pub token_group: String,
}

pub fn extract_key(headers: &HeaderMap, uri: &Uri) -> Option<String> {
    let raw = headers.get("authorization").and_then(|v| v.to_str().ok())
        .map(|v| v.trim())
        .or_else(|| headers.get("x-api-key").and_then(|v| v.to_str().ok()).map(|v| v.trim()))
        .or_else(|| uri.query().and_then(|q| q.split('&')
            .find_map(|p| p.strip_prefix("key="))))
        .filter(|v| !v.is_empty())?;
    let raw = raw.strip_prefix("Bearer ").or_else(|| raw.strip_prefix("bearer ")).unwrap_or(raw);
    let raw = raw.strip_prefix("sk-").unwrap_or(raw);
    Some(raw.split('-').next().unwrap_or(raw).to_string())
}

pub fn validate_token(state: &AppState, key: &str) -> Result<TokenContext, ApiError> {
    let caches = state.caches.read().expect("cache lock");
    let token = caches.token_by_key(key).ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Invalid token"))?;
    let user = caches.users.get(&token.user_id).ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Invalid token"))?;
    if user.status != 1 || token.status != 1 {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Invalid token"));
    }
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
    if token.expired_time != -1 && token.expired_time < now {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Token expired"));
    }
    Ok(TokenContext { user_id: token.user_id, token_id: token.id, token_group: token.group.clone() })
}

pub async fn token_auth(State(state): State<AppState>, mut req: Request, next: Next) -> Result<Response, ApiError> {
    let key = extract_key(req.headers(), req.uri())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Token not provided"))?;
    let ctx = validate_token(&state, &key)?;
    req.extensions_mut().insert(ctx);
    Ok(next.run(req).await)
}
```

- [ ] **Step 2: 写 `src/auth/mod.rs`**

```rust
pub mod token;
```

- [ ] **Step 3: `src/lib.rs` 增加 `pub mod auth;`**

- [ ] **Step 4: 写测试 `tests/token_auth.rs`**

```rust
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use axum::body::Body;
use axum::http::Request;
use axum::routing::get;
use http_body_util::BodyExt;
use tower::ServiceExt;

fn state() -> solo_new_api_rust::state::AppState {
    let mut caches = solo_new_api_rust::state::Caches {
        users: HashMap::new(), tokens: HashMap::new(), channels: HashMap::new(),
        abilities: HashMap::new(), options: HashMap::new(),
    };
    caches.users.insert(1, solo_new_api_rust::model::UserRow {
        id: 1, username: "root".into(), password: "x".into(), display_name: None,
        role: 100, status: 1, group: "default".into(), auth_version: 1, email: None,
    });
    caches.tokens.insert("abc123".into(), solo_new_api_rust::model::TokenRow {
        id: 2, user_id: 1, key: "abc123".into(), status: 1, name: "t".into(),
        expired_time: -1, remain_quota: 0, unlimited_quota: true,
        model_limits_enabled: false, model_limits: String::new(), allow_ips: None,
        group: "vip".into(), auto_groups: None, cross_group_retry: false,
    });
    let cfg = solo_new_api_rust::config::Config {
        port: 0, db_path: ":memory:".into(), web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(), trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    solo_new_api_rust::state::AppState {
        config: cfg,
        pool: sqlx::SqlitePool::connect_lazy("sqlite::memory:").unwrap(),
        caches: Arc::new(RwLock::new(caches)),
    }
}

async fn whoami(ctx: axum::extract::Extension<solo_new_api_rust::auth::token::TokenContext>) -> String {
    ctx.0.token_group
}

#[tokio::test]
async fn rejects_missing_and_accepts_sk_key() {
    let app = axum::Router::new()
        .route("/protected", get(whoami))
        .layer(axum::middleware::from_fn_with_state(state(), solo_new_api_rust::auth::token::token_auth));

    let res = app.clone().oneshot(Request::builder().uri("/protected").body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 401);

    let res = app.oneshot(Request::builder().uri("/protected")
        .header("authorization", "Bearer sk-abc123")
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    assert_eq!(&body[..], b"vip");
}
```

- [ ] **Step 5: 运行测试**

Run: `cargo test --test token_auth`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src tests
git commit -m "feat(m1): relay token auth middleware"
```

---

## Task 5a: 管理登录、JWT 与 /api/user/self

**Files:**
- Create: `src/auth/session.rs`, `src/auth/admin.rs`, `src/routes/user.rs`
- Modify: `Cargo.toml`（加 `cookie`, `hex`）、`src/state.rs`（加 `jwt_secret`）、`src/auth/mod.rs`、`src/routes/mod.rs`、`src/main.rs`、`tests/token_auth.rs`（构造新字段）
- Test: `tests/admin_login.rs`

**Interfaces:**
- Produces: `session::load_or_create_secret(&Path) -> std::io::Result<Vec<u8>>`；`session::sha256_hex(&[u8]) -> String`；`session::Claims`；`session::create_session(&SqlitePool, &UserRow, ip, ua) -> (AuthBundle, String)`；`session::issue_access(&Claims, secret) -> String`；`admin::AdminContext { user_id, sid }`；`admin::admin_auth(State, Request, Next) -> Result<Response, ApiError>`；`routes::user::{login, self_info}`。

- [ ] **Step 1: 加依赖**

`Cargo.toml` 的 `[dependencies]` 追加：

```toml
cookie = "0.18"
hex = "0.4"
```

- [ ] **Step 2: `src/state.rs` 增加字段**

```rust
#[derive(Clone)]
pub struct AppState {
    pub config: Config,
    pub pool: SqlitePool,
    pub caches: Arc<RwLock<Caches>>,
    pub jwt_secret: Arc<Vec<u8>>,
}
```

同步修改 `tests/token_auth.rs` 的构造：

```rust
jwt_secret: Arc::new(b"test-secret".to_vec()),
```

- [ ] **Step 3: 写 `src/auth/session.rs`**

```rust
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use jsonwebtoken::{decode, encode, DecodingKey, EncodingKey, Header, Validation};
use serde::{Deserialize, Serialize};
use uuid::Uuid;
use crate::error::ApiError;
use crate::model::UserRow;

#[derive(Debug, Serialize, Deserialize)]
pub struct Claims {
    pub sub: i64,
    pub sid: String,
    pub ver: i64,
    pub uver: i64,
    pub iat: i64,
    pub exp: i64,
}

#[derive(Debug, Serialize)]
pub struct SessionView {
    pub sid: String,
    pub current: bool,
    pub login_method: String,
    pub ip: String,
    pub user_agent: String,
    pub created_at: i64,
    pub last_active_at: i64,
    pub expires_at: i64,
}

#[derive(Debug, Serialize)]
pub struct AuthBundle {
    pub access_token: String,
    pub token_type: &'static str,
    pub access_expires_at: i64,
    pub user: serde_json::Value,
    pub session: SessionView,
}

pub fn now() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    hex::encode(hasher.finalize())
}

pub fn load_or_create_secret(path: &Path) -> std::io::Result<Vec<u8>> {
    if let Ok(bytes) = std::fs::read(path) {
        if bytes.len() >= 32 {
            return Ok(bytes);
        }
    }
    use rand::RngCore;
    let mut secret = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut secret);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, &secret)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(secret)
}

pub fn verify_password(hash: &str, password: &str) -> bool {
    PasswordHash::new(hash)
        .ok()
        .and_then(|parsed| Argon2::default().verify_password(password.as_bytes(), &parsed).ok())
        .is_some()
}

pub fn issue_access(claims: &Claims, secret: &[u8]) -> Result<String, ApiError> {
    encode(&Header::default(), claims, &EncodingKey::from_secret(secret))
        .map_err(|e| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))
}

pub fn decode_access(token: &str, secret: &[u8]) -> Result<Claims, ApiError> {
    let mut validation = Validation::default();
    validation.validate_exp = true;
    decode::<Claims>(token, &DecodingKey::from_secret(secret), &validation)
        .map(|data| data.claims)
        .map_err(|_| ApiError::new(axum::http::StatusCode::UNAUTHORIZED, "Session expired!"))
}

pub fn user_json(user: &UserRow) -> serde_json::Value {
    serde_json::json!({
        "id": user.id, "username": user.username, "display_name": user.display_name,
        "role": user.role, "status": user.status, "group": user.group,
        "email": user.email, "quota": 0, "used_quota": 0, "request_count": 0,
    })
}

pub async fn create_session(
    pool: &sqlx::SqlitePool,
    user: &UserRow,
    ip: &str,
    user_agent: &str,
    secret: &[u8],
) -> Result<(AuthBundle, String), ApiError> {
    use rand::RngCore;
    let mut refresh_secret = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut refresh_secret);
    let refresh_plain = hex::encode(&refresh_secret);
    let sid = Uuid::new_v4().to_string();
    let created = now();
    let expires = created + 7 * 24 * 3600;
    sqlx::query(
        "INSERT INTO user_sessions (sid,user_id,version,user_auth_version,status,refresh_hash,login_method,ip,user_agent,created_at,last_active_at,expires_at,revoked_at) \
         VALUES (?,?,1,?,'active',?,'password',?,?,?,?,?,0)"
    )
    .bind(&sid).bind(user.id).bind(user.auth_version)
    .bind(sha256_hex(refresh_plain.as_bytes()))
    .bind(ip).bind(user_agent).bind(created).bind(created).bind(expires)
    .execute(pool).await
    .map_err(|e| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let access_exp = created + 15 * 60;
    let claims = Claims { sub: user.id, sid: sid.clone(), ver: 1, uver: user.auth_version, iat: created, exp: access_exp };
    let access_token = issue_access(&claims, secret)?;
    Ok((AuthBundle {
        access_token,
        token_type: "Bearer",
        access_expires_at: access_exp,
        user: user_json(user),
        session: SessionView {
            sid, current: true, login_method: "password".into(),
            ip: ip.into(), user_agent: user_agent.into(),
            created_at: created, last_active_at: created, expires_at: expires,
        },
    }, refresh_plain))
}
```

- [ ] **Step 4: 写 `src/auth/admin.rs`**

```rust
use axum::extract::{Request, State};
use axum::http::StatusCode;
use axum::middleware::Next;
use axum::response::Response;
use crate::auth::session;
use crate::error::ApiError;
use crate::state::AppState;

#[derive(Debug, Clone)]
pub struct AdminContext {
    pub user_id: i64,
    pub sid: String,
}

pub async fn admin_auth(State(state): State<AppState>, mut req: Request, next: Next) -> Result<Response, ApiError> {
    let header = req.headers().get("authorization").and_then(|v| v.to_str().ok())
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"))?;
    let token = header.strip_prefix("Bearer ").unwrap_or(header);
    let claims = session::decode_access(token, &state.jwt_secret)?;

    let row: Option<(String, i64, i64, i64)> = sqlx::query_as(
        "SELECT s.status, s.version, s.user_auth_version, s.expires_at FROM user_sessions s WHERE s.sid=?"
    ).bind(&claims.sid).fetch_optional(&state.pool).await
        .map_err(|e| ApiError::new(StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let Some((status, version, user_auth_version, expires_at)) = row else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
    };
    if status != "active" || version != claims.ver || user_auth_version != claims.uver
        || expires_at <= session::now()
    {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
    }
    {
        let caches = state.caches.read().expect("cache lock");
        let user = caches.users.get(&claims.sub)
            .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"))?;
        if user.status != 1 || user.auth_version != claims.uver {
            return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"));
        }
    }
    req.extensions_mut().insert(AdminContext { user_id: claims.sub, sid: claims.sid });
    Ok(next.run(req).await)
}
```

- [ ] **Step 5: 写 `src/routes/user.rs`**

```rust
use axum::extract::{Extension, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde::Deserialize;
use crate::auth::admin::AdminContext;
use crate::auth::session::{self, AuthBundle};
use crate::error::{json_ok, ApiError};
use crate::state::AppState;

#[derive(Deserialize)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}

fn client_ip(headers: &HeaderMap) -> String {
    headers.get("x-forwarded-for").and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next()).map(|v| v.trim().to_string())
        .unwrap_or_else(|| "unknown".to_string())
}

fn set_cookie(sid: &str, refresh_plain: &str) -> String {
    format!(
        "new_api_refresh={sid}.{refresh_plain}; Path=/; HttpOnly; SameSite=Lax; Max-Age=604800"
    )
}

pub async fn login(State(state): State<AppState>, headers: HeaderMap, axum::Json(req): axum::Json<LoginRequest>) -> Result<Response, ApiError> {
    let user = {
        let caches = state.caches.read().expect("cache lock");
        caches.users.values().find(|u| u.username == req.username).cloned()
    };
    let Some(user) = user else {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Invalid username or password"));
    };
    if user.status != 1 || !session::verify_password(&user.password, &req.password) {
        return Err(ApiError::new(StatusCode::UNAUTHORIZED, "Invalid username or password"));
    }
    let ip = client_ip(&headers);
    let ua = headers.get("user-agent").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
    let (bundle, refresh_plain) = session::create_session(&state.pool, &user, &ip, &ua, &state.jwt_secret).await?;
    let mut response = json_ok(&bundle);
    response.headers_mut().insert("set-cookie", set_cookie(&bundle.session.sid, &refresh_plain).parse().unwrap());
    Ok(response)
}

pub async fn self_info(State(state): State<AppState>, Extension(ctx): Extension<AdminContext>) -> Response {
    let caches = state.caches.read().expect("cache lock");
    match caches.users.get(&ctx.user_id) {
        Some(user) => json_ok(&session::user_json(user)),
        None => ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!").into_response(),
    }
}
```

- [ ] **Step 6: 装配路由**

`src/auth/mod.rs`：

```rust
pub mod admin;
pub mod session;
pub mod token;
```

`src/routes/mod.rs` 增加 `pub mod user;`。在 `main.rs` 装配：

```rust
let state = /* 上文构造的 AppState */;
let login_route = Router::new()
    .route("/api/user/login", axum::routing::post(routes::user::login));
let self_route = Router::new()
    .route("/api/user/self", axum::routing::get(routes::user::self_info))
    .route_layer(axum::middleware::from_fn_with_state(state.clone(), auth::admin::admin_auth));
let app = Router::new()
    .merge(routes::public_router())
    .merge(login_route)
    .merge(self_route)
    .with_state(state);
```

- [ ] **Step 7: 写测试 `tests/admin_login.rs`**

```rust
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use sqlx::SqlitePool;
use std::collections::HashMap;
use std::sync::{Arc, RwLock};
use tower::ServiceExt;

#[tokio::test]
async fn login_then_self() {
    let pool = SqlitePool::connect("sqlite::memory:").await.unwrap();
    sqlx::query("CREATE TABLE users (id INTEGER PRIMARY KEY, username TEXT, password TEXT, display_name TEXT, role INTEGER, status INTEGER, `group` TEXT, auth_version INTEGER, email TEXT, deleted_at TEXT)").execute(&pool).await.unwrap();
    sqlx::query("CREATE TABLE user_sessions (sid TEXT PRIMARY KEY, user_id INTEGER, version INTEGER, user_auth_version INTEGER, status TEXT, refresh_hash TEXT, login_method TEXT, ip TEXT, user_agent TEXT, created_at INTEGER, last_active_at INTEGER, expires_at INTEGER, revoked_at INTEGER DEFAULT 0)").execute(&pool).await.unwrap();
    use argon2::password_hash::{rand_core::OsRng, PasswordHasher, SaltString};
    let salt = SaltString::generate(&mut OsRng);
    let hash = argon2::Argon2::default()
        .hash_password(b"secret123", &salt).unwrap().to_string();
    sqlx::query("INSERT INTO users (id,username,password,display_name,role,status,`group`,auth_version,email) VALUES (1,'alley',?,'Root',100,1,'default',1,NULL)")
        .bind(&hash).execute(&pool).await.unwrap();

    let user = solo_new_api_rust::model::UserRow {
        id: 1, username: "alley".into(), password: hash, display_name: Some("Root".into()),
        role: 100, status: 1, group: "default".into(), auth_version: 1, email: None,
    };
    let mut caches = solo_new_api_rust::state::Caches {
        users: HashMap::new(), tokens: HashMap::new(), channels: HashMap::new(),
        abilities: HashMap::new(), options: HashMap::new(),
    };
    caches.users.insert(1, user);
    let cfg = solo_new_api_rust::config::Config {
        port: 0, db_path: ":memory:".into(), web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(), trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = solo_new_api_rust::state::AppState {
        config: cfg, pool, caches: Arc::new(RwLock::new(caches)),
        jwt_secret: Arc::new(b"test-secret".to_vec()),
    };
    let login = axum::Router::new()
        .route("/api/user/login", axum::routing::post(solo_new_api_rust::routes::user::login));
    let guarded = axum::Router::new()
        .route("/api/user/self", axum::routing::get(solo_new_api_rust::routes::user::self_info))
        .route_layer(axum::middleware::from_fn_with_state(state.clone(), solo_new_api_rust::auth::admin::admin_auth));
    let app = login.merge(guarded).with_state(state);

    let res = app.clone().oneshot(Request::builder().method("POST").uri("/api/user/login")
        .header("content-type", "application/json")
        .body(Body::from(r#"{"username":"alley","password":"secret123"}"#)).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let token = v["data"]["access_token"].as_str().unwrap().to_string();
    assert!(v["data"]["session"]["sid"].as_str().is_some());

    let res = app.oneshot(Request::builder().uri("/api/user/self")
        .header("authorization", format!("Bearer {token}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
}
```

**测试清理**：测试只保留按列名插入一次；不要先插 `'HASH'` 再删除。

- [ ] **Step 8: 运行测试**

Run: `cargo test --test admin_login`
Expected: PASS。

- [ ] **Step 9: 提交**

```bash
git add Cargo.toml src tests
git commit -m "feat(m1): admin login with JWT and session"
```

---

## Task 5b: 会话刷新与登出

**Files:**
- Modify: `src/auth/session.rs`, `src/routes/user.rs`, `src/main.rs`
- Test: `tests/admin_refresh.rs`

**Interfaces:**
- Produces: `session::parse_refresh_cookie(&HeaderMap) -> Option<(String, String)>`（sid, secret）；`session::refresh_session(&SqlitePool, users: &HashMap<i64, UserRow>, sid, secret, jwt_secret) -> Result<(AuthBundle, String), ApiError>`；`routes::user::{refresh, logout}`。

- [ ] **Step 1: `session.rs` 增加 cookie 解析与刷新**

```rust
use axum::http::HeaderMap;

pub fn parse_refresh_cookie(headers: &HeaderMap) -> Option<(String, String)> {
    let cookie = headers.get("cookie")?.to_str().ok()?;
    let value = cookie.split(';')
        .find_map(|part| part.trim().strip_prefix("new_api_refresh="))?;
    let (sid, secret) = value.split_once('.')?;
    Some((sid.to_string(), secret.to_string()))
}

pub async fn refresh_session(
    pool: &sqlx::SqlitePool,
    users: &std::collections::HashMap<i64, crate::model::UserRow>,
    sid: &str,
    secret: &str,
    jwt_secret: &[u8],
) -> Result<(AuthBundle, String), ApiError> {
    let row: Option<(i64, i64, i64, String, String, Option<String>, i64, String, String, String, i64, i64, i64)> =
        sqlx::query_as(
            "SELECT user_id, version, user_auth_version, status, refresh_hash, previous_refresh_hash, \
             previous_valid_until, login_method, ip, user_agent, created_at, last_active_at, expires_at \
             FROM user_sessions WHERE sid=?"
        ).bind(sid).fetch_optional(pool).await
        .map_err(|e| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;
    let Some((user_id, version, uver, status, refresh_hash, prev_hash, prev_until, method, ip, ua, created, _last, expires)) = row else {
        return Err(ApiError::new(axum::http::StatusCode::UNAUTHORIZED, "Session expired!"));
    };
    if status != "active" || expires <= now() {
        return Err(ApiError::new(axum::http::StatusCode::UNAUTHORIZED, "Session expired!"));
    }
    let presented = sha256_hex(secret.as_bytes());
    let valid = presented == refresh_hash
        || (prev_until > now() && prev_hash.as_deref() == Some(presented.as_str()));
    if !valid {
        return Err(ApiError::new(axum::http::StatusCode::UNAUTHORIZED, "Session expired!"));
    }
    use rand::RngCore;
    let mut next = vec![0u8; 32];
    rand::thread_rng().fill_bytes(&mut next);
    let next_plain = hex::encode(&next);
    let active = now();
    sqlx::query(
        "UPDATE user_sessions SET refresh_hash=?, previous_refresh_hash=?, previous_valid_until=?, last_active_at=? WHERE sid=?"
    )
    .bind(sha256_hex(next_plain.as_bytes())).bind(&refresh_hash)
    .bind(active + 60).bind(active).bind(sid)
    .execute(pool).await
    .map_err(|e| ApiError::new(axum::http::StatusCode::INTERNAL_SERVER_ERROR, e.to_string()))?;

    let user = users.get(&user_id)
        .ok_or_else(|| ApiError::new(axum::http::StatusCode::UNAUTHORIZED, "Session expired!"))?;
    let access_exp = active + 15 * 60;
    let claims = Claims { sub: user_id, sid: sid.to_string(), ver: version, uver, iat: active, exp: access_exp };
    let access_token = issue_access(&claims, jwt_secret)?;
    let bundle = AuthBundle {
        access_token,
        token_type: "Bearer",
        access_expires_at: access_exp,
        user: user_json(user),
        session: SessionView {
            sid: sid.to_string(), current: true, login_method: method,
            ip, user_agent: ua, created_at: created, last_active_at: active, expires_at: expires,
        },
    };
    Ok((bundle, next_plain))
}
```

- [ ] **Step 2: `routes/user.rs` 增加刷新与登出**

```rust
pub async fn refresh(State(state): State<AppState>, headers: HeaderMap) -> Result<Response, ApiError> {
    let (sid, secret) = session::parse_refresh_cookie(&headers)
        .ok_or_else(|| ApiError::new(StatusCode::UNAUTHORIZED, "Session expired!"))?;
    let (bundle, next_plain) = {
        let users = state.caches.read().expect("cache lock").users.clone();
        session::refresh_session(&state.pool, &users, &sid, &secret, &state.jwt_secret).await?
    };
    let mut response = json_ok(&bundle);
    response.headers_mut().insert("set-cookie", set_cookie(&sid, &next_plain).parse().unwrap());
    Ok(response)
}

pub async fn logout(State(state): State<AppState>, headers: HeaderMap) -> Response {
    let sid = session::parse_refresh_cookie(&headers).map(|(sid, _)| sid)
        .or_else(|| headers.get("x-auth-session").and_then(|v| v.to_str().ok()).map(|v| v.to_string()));
    if let Some(sid) = sid {
        let _ = sqlx::query("UPDATE user_sessions SET status='revoked', revoked_at=?, revoked_reason='logout' WHERE sid=?")
            .bind(session::now()).bind(&sid).execute(&state.pool).await;
    }
    let mut response = json_ok(&serde_json::json!({}));
    response.headers_mut().insert("set-cookie",
        "new_api_refresh=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0".parse().unwrap());
    response
}
```

- [ ] **Step 3: 注册路由**

在 `main.rs` 的 `login_route` 上追加：

```rust
.route("/api/user/auth/refresh", axum::routing::post(routes::user::refresh))
.route("/api/user/auth/logout", axum::routing::post(routes::user::logout))
```

- [ ] **Step 4: 写测试 `tests/admin_refresh.rs`**

复用 Task 5a 的测试夹具构造，登录后：

```rust
let cookie = res.headers().get("set-cookie").unwrap().to_str().unwrap()
    .split(';').next().unwrap().to_string();

let res = app.clone().oneshot(Request::builder().method("POST").uri("/api/user/auth/refresh")
    .header("cookie", cookie.clone()).body(Body::empty()).unwrap()).await.unwrap();
assert_eq!(res.status(), 200);
let body = res.into_body().collect().await.unwrap().to_bytes();
let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
let token2 = v["data"]["access_token"].as_str().unwrap().to_string();
assert_ne!(token2, token);

let res = app.clone().oneshot(Request::builder().method("POST").uri("/api/user/auth/logout")
    .header("cookie", cookie).body(Body::empty()).unwrap()).await.unwrap();
assert_eq!(res.status(), 200);

let res = app.oneshot(Request::builder().uri("/api/user/self")
    .header("authorization", format!("Bearer {token2}"))
    .body(Body::empty()).unwrap()).await.unwrap();
assert_eq!(res.status(), 401);
```

- [ ] **Step 5: 运行测试**

Run: `cargo test --test admin_refresh`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src tests
git commit -m "feat(m1): session refresh and logout"
```

---

## Task 6: /v1/models 与模型聚合

**Files:**
- Create: `src/routes/models.rs`
- Modify: `src/auth/token.rs`（`TokenContext` 增加模型限制字段）、`src/routes/mod.rs`、`src/main.rs`
- Test: `tests/models_endpoint.rs`

**Interfaces:**
- Produces: `routes::models::{list_models, retrieve_model}`；`TokenContext { user_id, token_id, token_group, model_limits_enabled: bool, model_limits: String }`；`models::available_models(&Caches, &TokenContext) -> Vec<String>`。

- [ ] **Step 1: 扩展 `TokenContext`**

`src/auth/token.rs` 结构体与构造：

```rust
pub struct TokenContext {
    pub user_id: i64,
    pub token_id: i64,
    pub token_group: String,
    pub model_limits_enabled: bool,
    pub model_limits: String,
}
// validate_token 末尾：
Ok(TokenContext {
    user_id: token.user_id, token_id: token.id, token_group: token.group.clone(),
    model_limits_enabled: token.model_limits_enabled,
    model_limits: token.model_limits.clone(),
})
```

- [ ] **Step 2: 写 `src/routes/models.rs`**

```rust
use axum::extract::{Extension, Path, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::Response;
use serde_json::json;
use crate::auth::token::TokenContext;
use crate::error::{json_ok, ApiError};
use crate::state::{AppState, Caches};

pub fn available_models(caches: &Caches, ctx: &TokenContext) -> Vec<String> {
    let mut models: Vec<String> = caches.abilities.iter()
        .filter(|((group, _), _)| group == &ctx.token_group)
        .filter(|((_, _), rows)| rows.iter().any(|a| a.enabled))
        .filter(|((_, _), rows)| rows.iter().any(|a| {
            caches.channels.get(&a.channel_id).map(|c| c.status == 1).unwrap_or(false)
        }))
        .map(|((_, model), _)| model.clone())
        .collect();
    models.sort();
    models.dedup();
    if ctx.model_limits_enabled {
        let allowed: std::collections::HashSet<&str> = ctx.model_limits.split(',').map(str::trim).collect();
        models.retain(|m| allowed.contains(m.as_str()));
    }
    models
}

fn wants_anthropic(headers: &HeaderMap) -> bool {
    headers.contains_key("anthropic-version") || headers.contains_key("x-api-key")
}

pub async fn list_models(State(state): State<AppState>, Extension(ctx): Extension<TokenContext>, headers: HeaderMap) -> Response {
    let caches = state.caches.read().expect("cache lock");
    let models = available_models(&caches, &ctx);
    if wants_anthropic(&headers) {
        let data: Vec<serde_json::Value> = models.iter().map(|m| json!({
            "type": "model", "id": m, "display_name": m, "created_at": "1970-01-01T00:00:00Z"
        })).collect();
        return json_ok(&json!({
            "data": data, "has_more": false,
            "first_id": models.first(), "last_id": models.last()
        }));
    }
    let data: Vec<serde_json::Value> = models.iter().map(|m| json!({
        "id": m, "object": "model", "created": 0, "owned_by": "system"
    })).collect();
    json_ok(&json!({ "object": "list", "data": data }))
}

pub async fn retrieve_model(State(state): State<AppState>, Extension(ctx): Extension<TokenContext>, Path(model): Path<String>) -> Result<Response, ApiError> {
    let caches = state.caches.read().expect("cache lock");
    let models = available_models(&caches, &ctx);
    if models.iter().any(|m| m == &model) {
        Ok(json_ok(&json!({ "id": model, "object": "model", "created": 0, "owned_by": "system" })))
    } else {
        Err(ApiError::new(StatusCode::NOT_FOUND, "model not found"))
    }
}
```

- [ ] **Step 3: 装配路由**

`src/routes/mod.rs` 增加 `pub mod models;`。在 `main.rs`：

```rust
let relay_routes = Router::new()
    .route("/v1/models", axum::routing::get(routes::models::list_models))
    .route("/v1/models/{model}", axum::routing::get(routes::models::retrieve_model))
    .route_layer(axum::middleware::from_fn_with_state(state.clone(), auth::token::token_auth));
let app = Router::new()
    .merge(routes::public_router())
    .merge(login_route)
    .merge(self_route)
    .merge(relay_routes)
    .with_state(state);
```

- [ ] **Step 4: 写测试 `tests/models_endpoint.rs`**

构造 caches：token `vip`，channel 3 status=1，abilities `vip:m1`、`vip:m2`、`default:m3`。断言：

```rust
// GET /v1/models  Bearer sk-abc123
let v: serde_json::Value = ...;
let ids: Vec<&str> = v["data"].iter().map(|m| m["id"].as_str().unwrap()).collect();
assert_eq!(ids, vec!["m1", "m2"]);

// Anthropic 头
// header("anthropic-version","2023-06-01").header("x-api-key","sk-abc123")
assert_eq!(v["data"][0]["type"], "model");
assert_eq!(v["has_more"], false);
```

再增加一个 `model_limits_enabled=true, model_limits="m2"` 的 token，断言只返回 `m2`。

- [ ] **Step 5: 运行测试**

Run: `cargo test --test models_endpoint`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src tests
git commit -m "feat(m1): /v1/models with group and token limits"
```

---

## Task 7: 静态前端与用户信息接口

**Files:**
- Create: `src/web.rs`
- Modify: `src/routes/user.rs`、`src/routes/mod.rs`、`src/main.rs`
- Test: `tests/static_web.rs`

**Interfaces:**
- Produces: `web::spa_service(dist: PathBuf) -> Router`（未匹配路径回退 `index.html`）；`routes::user::self_groups`、`routes::user::user_models`。

- [ ] **Step 1: 写 `src/web.rs`**

```rust
use std::path::PathBuf;
use axum::Router;
use tower_http::services::{ServeDir, ServeFile};

pub fn spa_service(dist: PathBuf) -> Router {
    let index = dist.join("index.html");
    Router::new().fallback_service(
        ServeDir::new(&dist).not_found_service(ServeFile::new(index))
    )
}
```

- [ ] **Step 2: 增加用户信息接口**

`src/routes/user.rs`：

```rust
pub async fn self_groups() -> Response {
    json_ok(&serde_json::json!({
        "default": { "desc": "默认分组", "ratio": 1 },
        "vip": { "desc": "VIP 分组", "ratio": 1 }
    }))
}

pub async fn user_models(State(state): State<AppState>) -> Response {
    let caches = state.caches.read().expect("cache lock");
    let mut models: Vec<String> = caches.abilities.iter()
        .filter(|(_, rows)| rows.iter().any(|a| a.enabled))
        .filter(|(_, rows)| rows.iter().any(|a| {
            caches.channels.get(&a.channel_id).map(|c| c.status == 1).unwrap_or(false)
        }))
        .map(|((_, model), _)| model.clone())
        .collect();
    models.sort();
    models.dedup();
    json_ok(&models)
}
```

- [ ] **Step 3: 注册**

`main.rs` 的 `self_route` 追加（仍在 admin 中间件下）：

```rust
.route("/api/user/self/groups", axum::routing::get(routes::user::self_groups))
.route("/api/user/models", axum::routing::get(routes::user::user_models))
```

最后作为兜底服务（放在所有 API 路由之后）：

```rust
let app = app.fallback_service(web::spa_service(config.web_dist.clone()));
```

并在 `main.rs` 增加 `mod web;`（或使用 `solo_new_api_rust::web`）。

- [ ] **Step 4: 写测试 `tests/static_web.rs`**

```rust
#[tokio::test]
async fn serves_index_for_spa_routes() {
    let dir = std::env::temp_dir().join(format!("spa-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("index.html"), "<html>ok</html>").unwrap();
    let app = solo_new_api_rust::web::spa_service(dir.clone());
    let res = app.oneshot(axum::http::Request::builder().uri("/keys")
        .body(axum::body::Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
    let body = http_body_util::BodyExt::collect(res.into_body()).await.unwrap().to_bytes();
    assert!(String::from_utf8_lossy(&body).contains("ok"));
    std::fs::remove_dir_all(dir).ok();
}
```

（需要 `use tower::ServiceExt;`）

- [ ] **Step 5: 运行测试**

Run: `cargo test --test static_web`
Expected: PASS。

- [ ] **Step 6: 提交**

```bash
git add src tests
git commit -m "feat(m1): spa static serving and user info endpoints"
```

---

## Task 8: 优雅停机与真实库兼容验收

**Files:**
- Modify: `src/main.rs`、`src/routes/status.rs`
- Create: `tests/real_db.rs`
- Test: `tests/real_db.rs`（默认忽略，需 `REAL_DB` 环境变量）

**Interfaces:**
- Produces: `START_TIME: OnceLock<i64>`；`main` 支持 SIGINT/SIGTERM 优雅停机；真实库冒烟测试可手动执行。

- [ ] **Step 1: 启动时间改为 OnceLock**

`src/lib.rs` 或 `src/main.rs`：

```rust
pub static START_TIME: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
```

`main` 开头：

```rust
let start = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_secs() as i64;
START_TIME.set(start).ok();
```

`status.rs` 中 `"start_time": crate::START_TIME` 改为：

```rust
"start_time": crate::START_TIME.get().copied().unwrap_or(0),
```

- [ ] **Step 2: 优雅停机**

```rust
async fn shutdown_signal() {
    let ctrl_c = async { tokio::signal::ctrl_c().await.ok(); };
    #[cfg(unix)]
    let term = async {
        use tokio::signal::unix::{signal, SignalKind};
        signal(SignalKind::terminate()).unwrap().recv().await;
    };
    #[cfg(not(unix))]
    let term = std::future::pending::<()>();
    tokio::select! { _ = ctrl_c => {}, _ = term => {} }
    tracing::info!("shutdown signal received");
}
```

`main` 启动：

```rust
axum::serve(listener, app)
    .with_graceful_shutdown(shutdown_signal())
    .await?;
```

- [ ] **Step 3: 写真实库测试 `tests/real_db.rs`**

```rust
use axum::body::Body;
use axum::http::Request;
use http_body_util::BodyExt;
use std::sync::{Arc, RwLock};
use tower::ServiceExt;

#[tokio::test]
#[ignore = "requires REAL_DB pointing at a copy of one-api.db"]
async fn real_db_schema_caches_and_token_models() {
    let path = std::env::var("REAL_DB").expect("REAL_DB");
    let pool = solo_new_api_rust::db::connect(std::path::Path::new(&path)).await.unwrap();
    solo_new_api_rust::db::validate_schema(&pool).await.unwrap();

    let caches = solo_new_api_rust::state::Caches::load(&pool).await.unwrap();
    assert!(!caches.users.is_empty());
    assert!(!caches.tokens.is_empty());
    assert!(!caches.channels.is_empty());

    let (key, user) = {
        let token = caches.tokens.values().find(|t| t.status == 1).expect("enabled token");
        let user = caches.users.get(&token.user_id).expect("token user");
        (token.key.clone(), user.clone())
    };
    assert_eq!(user.status, 1);

    let cfg = solo_new_api_rust::config::Config {
        port: 0, db_path: path.into(), web_dist: "./web/dist".into(),
        jwt_secret_path: "/tmp/.jwt-test".into(), trust_proxy_headers: true,
        max_request_bytes: 64 * 1024 * 1024,
    };
    let state = solo_new_api_rust::state::AppState {
        config: cfg, pool: pool.clone(), caches: Arc::new(RwLock::new(caches)),
        jwt_secret: Arc::new(b"test-secret".to_vec()),
    };
    let app = axum::Router::new()
        .route("/v1/models", axum::routing::get(solo_new_api_rust::routes::models::list_models))
        .layer(axum::middleware::from_fn_with_state(state.clone(), solo_new_api_rust::auth::token::token_auth))
        .with_state(state);

    let res = app.oneshot(Request::builder().uri("/v1/models")
        .header("authorization", format!("Bearer sk-{key}"))
        .body(Body::empty()).unwrap()).await.unwrap();
    assert_eq!(res.status(), 200);
    let body = res.into_body().collect().await.unwrap().to_bytes();
    let v: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert!(!v["data"].as_array().unwrap().is_empty());

    let integrity: String = sqlx::query_scalar("PRAGMA integrity_check").fetch_one(&pool).await.unwrap();
    assert_eq!(integrity, "ok");
}
```

- [ ] **Step 4: 运行默认测试与真实库测试**

Run: `cargo test`
Expected: 默认测试全绿，`real_db` ignored。

Run: `REAL_DB=data/one-api.db cargo test --test real_db -- --ignored`
Expected: PASS（要求 `data/one-api.db` 存在且可读；测试只读）。

- [ ] **Step 5: 提交**

```bash
git add src tests
git commit -m "feat(m1): graceful shutdown and real-db compatibility test"
```

---

## M1 完成标准

- `cargo test` 全绿；`REAL_DB=data/one-api.db cargo test --test real_db -- --ignored` 通过。
- 启动真实库副本：`DB_PATH=data/one-api.db cargo run`，`curl /api/status` 返回 `success:true`。
- 用真实启用 Token 调 `/v1/models` 返回渠道模型，不修改数据库结构。
- 管理登录/刷新/登出可用（需知道 root 密码）。
