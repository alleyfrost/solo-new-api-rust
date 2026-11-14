use std::error::Error;
use std::future::Future;

use tokio::net::TcpListener;

use crate::{auth, config::Config, db, logging, routes, state};

#[derive(Debug)]
pub enum ServeError {
    Schema(db::SchemaError),
    Other(Box<dyn Error + Send + Sync>),
}

impl std::fmt::Display for ServeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ServeError::Schema(e) => write!(f, "{e}"),
            ServeError::Other(e) => write!(f, "{e}"),
        }
    }
}

impl Error for ServeError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            ServeError::Schema(e) => Some(e),
            ServeError::Other(e) => Some(&**e),
        }
    }
}

fn other(e: impl Error + Send + Sync + 'static) -> ServeError {
    ServeError::Other(Box::new(e))
}

/// 启动完整的 API 与静态站点服务。监听器由调用方绑定，便于 CLI 与桌面端使用
/// 不同的绑定地址与端口策略；schema 不匹配时不修改数据库，交由调用方决定提示
/// 方式与退出码。
pub async fn serve(
    config: Config,
    listener: TcpListener,
    shutdown: impl Future<Output = ()> + Send + 'static,
) -> Result<(), ServeError> {
    let pool = db::connect(&config.db_path).await.map_err(other)?;
    db::validate_schema(&pool)
        .await
        .map_err(ServeError::Schema)?;
    // 每-token 限流配置独立成表，不动任何现有表结构；Go new-api 会无视未知表，
    // 回滚/混用安全。无该配置时行数恒为 0，查询空表不影响任何路径。
    sqlx::query(
        "CREATE TABLE IF NOT EXISTS token_rate_limits (\
         token_id INTEGER PRIMARY KEY, \
         rpm INTEGER NOT NULL DEFAULT 0, \
         concurrency INTEGER NOT NULL DEFAULT 0)",
    )
    .execute(&pool)
    .await
    .map_err(other)?;
    match crate::quota_data::backfill(&pool).await {
        Ok(stats) if stats.inserted > 0 => eprintln!(
            "quota_data backfill: cutoff={} deleted={} inserted={}",
            stats.cutoff, stats.deleted, stats.inserted
        ),
        Ok(_) => {}
        Err(e) => eprintln!("quota_data backfill failed: {e}"),
    }
    let caches = state::Caches::load(&pool).await.map_err(other)?;
    let jwt_secret =
        auth::session::load_or_create_secret(&config.jwt_secret_path).map_err(other)?;
    let (log_tx, log_rx) = logging::channel(20_000);
    let writer = logging::spawn_writer(pool.clone(), log_rx);
    let limits = std::sync::Arc::new(crate::ratelimit::RateLimitRegistry::default());
    limits.load(&pool).await.map_err(other)?;
    let mut state = state::AppState::new(config.clone(), pool, caches, jwt_secret, Some(log_tx));
    state.rate_limits = limits;
    crate::mem::spawn_periodic_trim();
    crate::retention::spawn(state.clone());

    let login_route = axum::Router::new()
        .route("/api/user/login", axum::routing::post(routes::user::login))
        .route(
            "/api/user/auth/refresh",
            axum::routing::post(routes::user::refresh),
        )
        .route(
            "/api/user/auth/logout",
            axum::routing::post(routes::user::logout),
        );
    let self_route = axum::Router::new()
        .route(
            "/api/user/self",
            axum::routing::get(routes::user::self_info).put(routes::user::update_self),
        )
        .route(
            "/api/user/setting",
            axum::routing::put(routes::user::update_setting),
        )
        .route(
            "/api/user/sessions",
            axum::routing::get(routes::user::list_sessions),
        )
        .route(
            "/api/user/sessions/revoke-others",
            axum::routing::post(routes::user::revoke_other_sessions),
        )
        .route(
            "/api/user/sessions/{sid}",
            axum::routing::delete(routes::user::revoke_session),
        )
        .route(
            "/api/user/self/groups",
            axum::routing::get(routes::user::self_groups),
        )
        .route(
            "/api/user/models",
            axum::routing::get(routes::user::user_models),
        )
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::admin::admin_auth,
        ));
    let relay_routes = axum::Router::new()
        .route(
            "/v1/models",
            axum::routing::get(routes::models::list_models),
        )
        .route(
            "/v1/models/{model}",
            axum::routing::get(routes::models::retrieve_model),
        )
        .route(
            "/v1/chat/completions",
            axum::routing::post(crate::relay::chat::chat_completions),
        )
        .route(
            "/v1/messages",
            axum::routing::post(crate::relay::messages::messages),
        )
        .route(
            "/v1/responses",
            axum::routing::post(crate::relay::responses::responses),
        )
        .route(
            "/v1/embeddings",
            axum::routing::post(crate::relay::extra::embeddings),
        )
        .route(
            "/v1/images/generations",
            axum::routing::post(crate::relay::extra::image_generations),
        )
        .route(
            "/v1/images/edits",
            axum::routing::post(crate::relay::extra::image_edits),
        )
        .route(
            "/v1/audio/transcriptions",
            axum::routing::post(crate::relay::extra::audio_transcriptions),
        )
        .route(
            "/v1/audio/translations",
            axum::routing::post(crate::relay::extra::audio_translations),
        )
        .route(
            "/v1/audio/speech",
            axum::routing::post(crate::relay::extra::audio_speech),
        )
        .route(
            "/v1/rerank",
            axum::routing::post(crate::relay::extra::rerank),
        )
        .route(
            "/v1/completions",
            axum::routing::post(crate::relay::extra::completions),
        )
        .route(
            "/v1/moderations",
            axum::routing::post(crate::relay::extra::moderations),
        )
        .layer(axum::extract::DefaultBodyLimit::max(
            config.max_request_bytes,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::mem::large_body_gate,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::token::token_auth,
        ));
    let admin_routes = routes::admin::router().route_layer(axum::middleware::from_fn_with_state(
        state.clone(),
        auth::admin::admin_auth,
    ));
    // 对话调试页走会话认证，不带客户端 token；选路与 /v1 走同一套流水线。
    let playground_route = axum::Router::new()
        .route(
            "/pg/chat/completions",
            axum::routing::post(crate::relay::playground::chat_completions),
        )
        .layer(axum::extract::DefaultBodyLimit::max(
            config.max_request_bytes,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            crate::mem::large_body_gate,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::admin::admin_auth,
        ));
    let web_dist = config.web_dist.clone();
    let app = axum::Router::new()
        .merge(routes::public_router())
        .merge(login_route)
        .merge(self_route)
        .merge(admin_routes)
        .merge(playground_route)
        .merge(relay_routes)
        .with_state(state)
        .fallback_service(crate::web::spa_service(web_dist));

    tracing::info!("listening on {}", listener.local_addr().map_err(other)?);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown)
        .await
        .map_err(other)?;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), writer).await;
    Ok(())
}
