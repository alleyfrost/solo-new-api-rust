use solo_new_api_rust::{auth, config, db, routes, state};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::from_default_env())
        .init();
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
        .route("/api/user/login", axum::routing::post(routes::user::login));
    let self_route = axum::Router::new()
        .route("/api/user/self", axum::routing::get(routes::user::self_info))
        .route_layer(axum::middleware::from_fn_with_state(
            state.clone(),
            auth::admin::admin_auth,
        ));
    let app = axum::Router::new()
        .merge(routes::public_router())
        .merge(login_route)
        .merge(self_route)
        .with_state(state);

    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    tracing::info!("listening on :{}", config.port);
    axum::serve(listener, app).await?;
    Ok(())
}
