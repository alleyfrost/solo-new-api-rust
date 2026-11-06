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
