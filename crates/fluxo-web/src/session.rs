//! Server-side sessions (D8) with a per-session form token (D9).
//!
//! The cookie holds 32 random bytes (base64url). Only their SHA-256 is stored, so a leaked
//! database can't be used to hijack sessions. A session moves through stages:
//! `anonymous` → (password ok) → `second_factor` → (code ok) → `signed_in`. Each step up issues a
//! brand-new session ID, so an ID seen before sign-in is useless afterwards.

use axum_extra::extract::cookie::{Cookie, CookieJar, SameSite};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;

use crate::now;

pub const COOKIE: &str = "fluxo_session";

/// Idle timeouts in seconds.
const ANONYMOUS_IDLE: i64 = 60 * 60;
const SECOND_FACTOR_IDLE: i64 = 10 * 60;
const SIGNED_IN_IDLE: i64 = 30 * 24 * 60 * 60;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Stage {
    Anonymous,
    SecondFactor,
    SignedIn,
}

impl Stage {
    fn as_str(self) -> &'static str {
        match self {
            Stage::Anonymous => "anonymous",
            Stage::SecondFactor => "second_factor",
            Stage::SignedIn => "signed_in",
        }
    }

    fn parse(s: &str) -> Option<Stage> {
        match s {
            "anonymous" => Some(Stage::Anonymous),
            "second_factor" => Some(Stage::SecondFactor),
            "signed_in" => Some(Stage::SignedIn),
            _ => None,
        }
    }

    fn idle_limit(self) -> i64 {
        match self {
            Stage::Anonymous => ANONYMOUS_IDLE,
            Stage::SecondFactor => SECOND_FACTOR_IDLE,
            Stage::SignedIn => SIGNED_IN_IDLE,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Session {
    id_hash: Vec<u8>,
    pub user_id: Option<i64>,
    pub stage: Stage,
    pub csrf_token: String,
    pub failed_attempts: i64,
}

fn random_token() -> String {
    let mut b = [0u8; 32];
    rand::rng().fill_bytes(&mut b);
    URL_SAFE_NO_PAD.encode(b)
}

fn hash_id(cookie_value: &str) -> Vec<u8> {
    Sha256::digest(cookie_value.as_bytes()).to_vec()
}

fn cookie(value: String, secure: bool) -> Cookie<'static> {
    Cookie::build((COOKIE, value)).path("/").http_only(true).secure(secure).same_site(SameSite::Lax).build()
}

/// Constant-time comparison for form tokens.
pub fn token_matches(expected: &str, given: &str) -> bool {
    expected.len() == given.len() && expected.bytes().zip(given.bytes()).fold(0u8, |a, (x, y)| a | (x ^ y)) == 0
}

/// Load the session named by the cookie, if it exists and hasn't idled out. Touches `last_seen_at`.
pub async fn load(db: &SqlitePool, jar: &CookieJar) -> sqlx::Result<Option<Session>> {
    let Some(c) = jar.get(COOKIE) else { return Ok(None) };
    let id_hash = hash_id(c.value());
    let row = sqlx::query!(
        r#"SELECT user_id, stage, csrf_token, failed_attempts, last_seen_at FROM sessions WHERE id_hash = ?"#,
        id_hash
    )
    .fetch_optional(db)
    .await?;
    let Some(row) = row else { return Ok(None) };
    let Some(stage) = Stage::parse(&row.stage) else { return Ok(None) };
    let t = now();
    if t - row.last_seen_at > stage.idle_limit() {
        sqlx::query!("DELETE FROM sessions WHERE id_hash = ?", id_hash).execute(db).await?;
        return Ok(None);
    }
    sqlx::query!("UPDATE sessions SET last_seen_at = ? WHERE id_hash = ?", t, id_hash).execute(db).await?;
    Ok(Some(Session {
        id_hash,
        user_id: row.user_id,
        stage,
        csrf_token: row.csrf_token,
        failed_attempts: row.failed_attempts,
    }))
}

/// Create a new session and return it with the cookie that names it.
pub async fn create(
    db: &SqlitePool,
    jar: CookieJar,
    user_id: Option<i64>,
    stage: Stage,
    user_agent: &str,
    secure: bool,
) -> sqlx::Result<(Session, CookieJar)> {
    let value = random_token();
    let id_hash = hash_id(&value);
    let csrf_token = random_token();
    let t = now();
    let stage_str = stage.as_str();
    sqlx::query!(
        "INSERT INTO sessions (id_hash, user_id, stage, csrf_token, created_at, last_seen_at, user_agent)
         VALUES (?, ?, ?, ?, ?, ?, ?)",
        id_hash,
        user_id,
        stage_str,
        csrf_token,
        t,
        t,
        user_agent
    )
    .execute(db)
    .await?;
    let session = Session { id_hash, user_id, stage, csrf_token, failed_attempts: 0 };
    Ok((session, jar.add(cookie(value, secure))))
}

/// Replace `old` with a new session at a higher stage (new ID, new form token).
pub async fn advance(
    db: &SqlitePool,
    jar: CookieJar,
    old: &Session,
    user_id: i64,
    stage: Stage,
    user_agent: &str,
    secure: bool,
) -> sqlx::Result<(Session, CookieJar)> {
    end(db, old).await?;
    create(db, jar, Some(user_id), stage, user_agent, secure).await
}

pub async fn end(db: &SqlitePool, s: &Session) -> sqlx::Result<()> {
    sqlx::query!("DELETE FROM sessions WHERE id_hash = ?", s.id_hash).execute(db).await?;
    Ok(())
}

/// Record a failed code entry and return the new count.
pub async fn record_failure(db: &SqlitePool, s: &Session) -> sqlx::Result<i64> {
    let row = sqlx::query!(
        "UPDATE sessions SET failed_attempts = failed_attempts + 1 WHERE id_hash = ? RETURNING failed_attempts",
        s.id_hash
    )
    .fetch_one(db)
    .await?;
    Ok(row.failed_attempts)
}

pub fn clear_cookie(jar: CookieJar) -> CookieJar {
    jar.remove(Cookie::build(COOKIE).path("/"))
}
