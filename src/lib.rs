pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod logging;
pub mod model;
pub mod priority;
pub mod relay;
pub mod retry;
pub mod routing;
pub mod routes;
pub mod settings;
pub mod state;
pub mod upstream;
pub mod web;

pub static START_TIME: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
