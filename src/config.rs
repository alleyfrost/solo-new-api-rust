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
            max_request_bytes: env_or("MAX_REQUEST_BYTES", "33554432").parse().unwrap_or(33_554_432),
        }
    }
}

fn env_or(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
