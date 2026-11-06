use solo_new_api_rust::{auth, config, db, routes, state};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
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
    let state = state::AppState {
        config: config.clone(),
        pool,
        caches: std::sync::Arc::new(std::sync::RwLock::new(caches)),
        jwt_secret: std::sync::Arc::new(jwt_secret),
    };

    let login_route = axum::Router::new()
        .route("/api/user/login", axum::routing::post(routes::user::login))
        .route("/api/user/auth/refresh", axum::routing::post(routes::user::refresh))
        .route("/api/user/auth/logout", axum::routing::post(routes::user::logout));
    let self_route = axum::Router::new()
        .route("/api/user/self", axum::routing::get(routes::user::self_info))
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
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::token::token_auth,
        ));
    let web_dist = config.web_dist.clone();
    let app = axum::Router::new()
        .merge(routes::public_router())
        .merge(login_route)
        .merge(self_route)
        .merge(relay_routes)
        .with_state(state)
        .fallback_service(solo_new_api_rust::web::spa_service(web_dist));

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    tracing::info!("listening on :{}", config.port);
    axum::serve(listener, app)
        .with_graceful_shutdown(shutdown_signal())
        .await?;
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
