use solo_new_api_rust::{config, db, routes, state};

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
    let state = state::AppState {
        config: config.clone(),
        pool,
        caches: std::sync::Arc::new(std::sync::RwLock::new(caches)),
    };
    let app = routes::public_router().with_state(state);
    let listener = tokio::net::TcpListener::bind(format!("0.0.0.0:{}", config.port)).await?;
    tracing::info!("listening on :{}", config.port);
    axum::serve(listener, app).await?;
    Ok(())
}
