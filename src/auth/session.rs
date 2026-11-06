use std::collections::HashMap;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use axum::http::HeaderMap;
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

pub fn parse_refresh_cookie(headers: &HeaderMap) -> Option<(String, String)> {
    let cookie = headers.get("cookie")?.to_str().ok()?;
    let value = cookie
        .split(';')
        .find_map(|part| part.trim().strip_prefix("new_api_refresh="))?;
    let (sid, secret) = value.split_once('.')?;
    Some((sid.to_string(), secret.to_string()))
}

pub async fn refresh_session(
    pool: &sqlx::SqlitePool,
    users: &HashMap<i64, UserRow>,
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
