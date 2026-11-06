pub mod auth;
pub mod config;
pub mod db;
pub mod error;
pub mod model;
pub mod retry;
pub mod routing;
pub mod routes;
pub mod state;
pub mod upstream;
pub mod web;

pub static START_TIME: std::sync::OnceLock<i64> = std::sync::OnceLock::new();
