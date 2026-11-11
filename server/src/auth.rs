use std::sync::OnceLock;

use argon2::Argon2;
use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use axum::extract::FromRequestParts;
use axum::http::HeaderMap;
use axum::http::header::COOKIE;
use axum::http::request::Parts;
use paracord_shared::User;
use sqlx::MySqlConnection;

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::{now_ms, sha256_hex};

pub const SESSION_COOKIE: &str = "paracord_session";
pub const SESSION_DAYS: i64 = 30;

/// id, username, display_name, avatar_hue
pub type UserRow = (String, String, String, i64);
pub const USER_COLUMNS: &str = "u.id, u.username, u.display_name, u.avatar_hue";

pub fn to_user((id, username, display_name, avatar_hue): UserRow) -> User {
    User {
        id,
        username,
        display_name,
        avatar_hue,
    }
}

pub fn hash_password(password: &str) -> ApiResult<String> {
    let salt = SaltString::encode_b64(&crate::util::random_bytes::<16>()).map_err(ApiError::internal)?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(ApiError::internal)
}

pub fn verify_password(password: &str, stored: &str) -> bool {
    PasswordHash::new(stored)
        .map(|hash| Argon2::default().verify_password(password.as_bytes(), &hash).is_ok())
        .unwrap_or(false)
}

/// Checked when a username doesn't exist, so a wrong username takes as long
/// as a wrong password and can't be used to find out who has an account.
pub fn dummy_hash() -> &'static str {
    static HASH: OnceLock<String> = OnceLock::new();
    HASH.get_or_init(|| hash_password("paracord-timing-pad").unwrap_or_default())
}

pub fn session_cookie(token: &str, secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}={token}; Path=/; HttpOnly; SameSite=Lax; Max-Age={}{}",
        SESSION_DAYS * 24 * 60 * 60,
        if secure { "; Secure" } else { "" }
    )
}

pub fn clear_cookie(secure: bool) -> String {
    format!(
        "{SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Lax; Max-Age=0{}",
        if secure { "; Secure" } else { "" }
    )
}

pub fn session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get_all(COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .filter_map(|pair| pair.trim().split_once('='))
        .find(|(name, _)| *name == SESSION_COOKIE)
        .map(|(_, value)| value.to_string())
        .filter(|value| !value.is_empty())
}

pub async fn create_session(c: &mut MySqlConnection, user_id: &str) -> ApiResult<String> {
    let token = crate::util::new_token();
    let now = now_ms();
    sqlx::query("INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?, ?, ?, ?)")
        .bind(sha256_hex(&token))
        .bind(user_id)
        .bind(now)
        .bind(now + SESSION_DAYS * 24 * 60 * 60 * 1000)
        .execute(&mut *c)
        .await?;
    Ok(token)
}

/// The signed-in user. Add it to a handler's arguments to require sign-in.
pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = session_token(&parts.headers).ok_or(ApiError::Unauthorized)?;
        let row: Option<UserRow> = sqlx::query_as(sql!(
            "SELECT {USER_COLUMNS} FROM sessions s JOIN users u ON u.id = s.user_id
             WHERE s.token_hash = ? AND s.expires_at > ?"
        ))
        .bind(sha256_hex(&token))
        .bind(now_ms())
        .fetch_optional(&state.pool)
        .await?;
        row.map(|r| CurrentUser(to_user(r))).ok_or(ApiError::Unauthorized)
    }
}
