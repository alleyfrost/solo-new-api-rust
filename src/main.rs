use solo_new_api_rust::{auth, config, db, logging, routes, state};

fn main() -> Result<(), Box<dyn std::error::Error>> {
    tune_glibc_allocator();
    tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()?
        .block_on(run())
}

/// argon2id 的 19 MiB 工作内存与大请求体缓冲都在堆上按次分配；glibc 默认会把
/// 这类大块留在按线程划分的 arena 里，释放后不还给内核（实测 4×8 MiB 请求后
/// RSS 长期停在 170 MiB）。把 mmap 阈值压到 1 MiB，让大块走 mmap/munmap 直接
/// 归还；trim 兜住堆顶空闲页，arena 限 2 避免按线程膨胀。
#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn tune_glibc_allocator() {
    unsafe {
        libc::mallopt(libc::M_MMAP_THRESHOLD, 1024 * 1024);
        libc::mallopt(libc::M_TRIM_THRESHOLD, 128 * 1024);
        libc::mallopt(libc::M_ARENA_MAX, 2);
    }
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn tune_glibc_allocator() {}

async fn run() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
    let start = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    solo_new_api_rust::START_TIME.set(start).ok();
    let config = config::Config::from_env();
    let pool = db::connect(&config.db_path).await?;
    db::validate_schema(&pool).await.map_err(|e| {
        eprintln!("{e}");
        std::process::exit(2);
    })?;
    let caches = state::Caches::load(&pool).await?;
    let jwt_secret = auth::session::load_or_create_secret(&config.jwt_secret_path)?;
    let (log_tx, log_rx) = logging::channel(20_000);
    let writer = logging::spawn_writer(pool.clone(), log_rx);
    let state = state::AppState::new(config.clone(), pool, caches, jwt_secret, Some(log_tx));
    solo_new_api_rust::mem::spawn_periodic_trim();

    let login_route = axum::Router::new()
        .route("/api/user/login", axum::routing::post(routes::user::login))
        .route("/api/user/auth/refresh", axum::routing::post(routes::user::refresh))
        .route("/api/user/auth/logout", axum::routing::post(routes::user::logout));
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
        .route("/v1/models", axum::routing::get(routes::models::list_models))
        .route("/v1/models/{model}", axum::routing::get(routes::models::retrieve_model))
        .route(
            "/v1/chat/completions",
            axum::routing::post(solo_new_api_rust::relay::chat::chat_completions),
        )
        .route(
            "/v1/messages",
            axum::routing::post(solo_new_api_rust::relay::messages::messages),
        )
        .route(
            "/v1/responses",
            axum::routing::post(solo_new_api_rust::relay::responses::responses),
        )
        .route(
            "/v1/embeddings",
            axum::routing::post(solo_new_api_rust::relay::extra::embeddings),
        )
        .route(
            "/v1/images/generations",
            axum::routing::post(solo_new_api_rust::relay::extra::image_generations),
        )
        .route(
            "/v1/images/edits",
            axum::routing::post(solo_new_api_rust::relay::extra::image_edits),
        )
        .route(
            "/v1/audio/transcriptions",
            axum::routing::post(solo_new_api_rust::relay::extra::audio_transcriptions),
        )
        .route(
            "/v1/audio/translations",
            axum::routing::post(solo_new_api_rust::relay::extra::audio_translations),
        )
        .route(
            "/v1/audio/speech",
            axum::routing::post(solo_new_api_rust::relay::extra::audio_speech),
        )
        .route(
            "/v1/rerank",
            axum::routing::post(solo_new_api_rust::relay::extra::rerank),
        )
        .route(
            "/v1/completions",
            axum::routing::post(solo_new_api_rust::relay::extra::completions),
        )
        .route(
            "/v1/moderations",
            axum::routing::post(solo_new_api_rust::relay::extra::moderations),
        )
        .layer(axum::extract::DefaultBodyLimit::max(
            config.max_request_bytes,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            solo_new_api_rust::mem::large_body_gate,
        ))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::token::token_auth,
        ));
    let admin_routes = routes::admin::router().route_layer(
        axum::middleware::from_fn_with_state(state.clone(), auth::admin::admin_auth),
    );
    // 对话调试页走会话认证，不带客户端 token；选路与 /v1 走同一套流水线。
    let playground_route = axum::Router::new()
        .route(
            "/pg/chat/completions",
            axum::routing::post(solo_new_api_rust::relay::playground::chat_completions),
        )
        .layer(axum::extract::DefaultBodyLimit::max(
            config.max_request_bytes,
        ))
        .layer(axum::middleware::from_fn_with_state(
            state.clone(),
            solo_new_api_rust::mem::large_body_gate,
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
        .fallback_service(solo_new_api_rust::web::spa_service(web_dist));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    tracing::info!("listening on :{}", config.port);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
    let _ = tokio::time::timeout(std::time::Duration::from_secs(5), writer).await;
    Ok(())
}

async fn shutdown_signal() {
    let ctrl_c = async {
        tokio::signal::ctrl_c().await.ok();
    };
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
