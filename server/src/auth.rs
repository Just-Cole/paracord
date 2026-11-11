use std::sync::OnceLock;

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use axum::extract::FromRequestParts;
use axum::http::header::COOKIE;
use axum::http::request::Parts;
use axum::http::HeaderMap;
use paracord_shared::User;
use rusqlite::{Connection, OptionalExtension};

use crate::error::{ApiError, ApiResult};
use crate::state::AppState;
use crate::util::{now_ms, sha256_hex};

pub const SESSION_COOKIE: &str = "paracord_session";
pub const SESSION_DAYS: i64 = 30;

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

pub fn create_session(c: &Connection, user_id: &str) -> rusqlite::Result<String> {
    let token = crate::util::new_token();
    let now = now_ms();
    c.execute(
        "INSERT INTO sessions (token_hash, user_id, created_at, expires_at) VALUES (?1, ?2, ?3, ?4)",
        (
            sha256_hex(&token),
            user_id,
            now,
            now + SESSION_DAYS * 24 * 60 * 60 * 1000,
        ),
    )?;
    Ok(token)
}

pub fn user_row(row: &rusqlite::Row<'_>, start: usize) -> rusqlite::Result<User> {
    Ok(User {
        id: row.get(start)?,
        username: row.get(start + 1)?,
        display_name: row.get(start + 2)?,
        avatar_hue: row.get(start + 3)?,
    })
}

/// The signed-in user. Add it to a handler's arguments to require sign-in.
pub struct CurrentUser(pub User);

impl FromRequestParts<AppState> for CurrentUser {
    type Rejection = ApiError;

    async fn from_request_parts(parts: &mut Parts, state: &AppState) -> Result<Self, Self::Rejection> {
        let token = session_token(&parts.headers).ok_or(ApiError::Unauthorized)?;
        let token_hash = sha256_hex(&token);
        let user = state
            .db(move |c| {
                Ok(c.query_row(
                    "SELECT u.id, u.username, u.display_name, u.avatar_hue
                     FROM sessions s JOIN users u ON u.id = s.user_id
                     WHERE s.token_hash = ?1 AND s.expires_at > ?2",
                    (token_hash, now_ms()),
                    |row| user_row(row, 0),
                )
                .optional()?)
            })
            .await?;
        user.map(CurrentUser).ok_or(ApiError::Unauthorized)
    }
}
