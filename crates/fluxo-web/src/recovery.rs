//! Getting a locked-out person back in. An Administrator makes a one-time code in Settings and
//! gives it to them directly. The person enters it at /locked, must then set a new password, and
//! their 2FA is turned off until they set it up again. The Administrator never sees the password.

use askama::Template;
use axum::extract::{Form, Query, State};
use axum::http::HeaderMap;
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use rand::Rng;
use serde::Deserialize;
use sha2::{Digest, Sha256};

use crate::session::{self, Stage};
use crate::{Result, Shared, auth, forbidden, load_user, now, page, user_agent};

const VALID_HOURS: i64 = 24;
const ALPHABET: &[u8] = b"ABCDEFGHJKMNPQRSTUVWXYZ23456789"; // no I, L, O, 0, 1
const MAX_TRIES: i64 = 5;

/// Canonical form `XXXX-XXXX-XXXX`, ignoring case, spaces and dashes.
pub(crate) fn canonical(code: &str) -> Option<String> {
    let s: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_uppercase()).collect();
    (s.len() == 12).then(|| format!("{}-{}-{}", &s[..4], &s[4..8], &s[8..]))
}

pub(crate) fn hash(canonical_code: &str) -> Vec<u8> {
    Sha256::digest(canonical_code.as_bytes()).to_vec()
}

fn new_code() -> String {
    let mut rng = rand::rng();
    let s: String = (0..12).map(|_| ALPHABET[(rng.next_u32() as usize) % ALPHABET.len()] as char).collect();
    format!("{}-{}-{}", &s[..4], &s[4..8], &s[8..])
}

/// A code shown once to the Administrator who made it.
pub struct Issued {
    pub username: String,
    pub code: String,
    pub expires: String,
}

pub enum IssueOutcome {
    Issued(Issued),
    NoSuchUser,
    Yourself,
}

/// Make a code for `username`, cancelling any earlier unused ones.
pub(crate) async fn issue(app: &crate::AppState, admin_id: i64, username: &str) -> Result<IssueOutcome> {
    let name = username.trim().to_lowercase();
    let Some(user_id) =
        sqlx::query_scalar!(r#"SELECT id AS "id!" FROM users WHERE username = ? AND deleted_at IS NULL"#, name).fetch_optional(&app.db).await?
    else {
        return Ok(IssueOutcome::NoSuchUser);
    };
    if user_id == admin_id {
        return Ok(IssueOutcome::Yourself);
    }
    let code = new_code();
    let h = hash(&code);
    let (t, expires) = (now(), now() + VALID_HOURS * 3_600);
    let mut tx = app.db.begin().await?;
    sqlx::query!(
        "UPDATE admin_codes SET revoked_at = ? WHERE user_id = ? AND used_at IS NULL AND revoked_at IS NULL",
        t,
        user_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO admin_codes (user_id, issued_by, code_hash, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
        user_id,
        admin_id,
        h,
        t,
        expires
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    let secs = expires.rem_euclid(86_400);
    let expires = format!("{} at {:02}:{:02} UTC", crate::util::date(expires), secs / 3_600, (secs / 60) % 60);
    Ok(IssueOutcome::Issued(Issued { username: name, code, expires }))
}

// ---------------------------------------------------------------- /locked

#[derive(Template)]
#[template(path = "locked.html")]
struct LockedPage<'a> {
    theme: &'a str,
    csrf: &'a str,
    username: &'a str,
    error: Option<&'a str>,
}

#[derive(Deserialize)]
pub struct LockedQuery {
    reason: Option<String>,
}

#[derive(Deserialize)]
pub struct LockedForm {
    csrf: String,
    username: String,
    code: String,
}

const WRONG: &str = "That username and code don’t match, or the code has expired or was already used.";
const TOO_MANY: &str = "Too many tries. Ask your Administrator for a new code if you need one.";

pub async fn locked_page(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Query(q): Query<LockedQuery>,
) -> Result<Response> {
    let (s, jar) = match session::load(&app.db, &jar).await? {
        Some(s) if s.stage == Stage::Anonymous => (s, jar),
        Some(s) if s.stage == Stage::SignedIn => return Ok(Redirect::to("/").into_response()),
        other => {
            if let Some(old) = other {
                session::end(&app.db, &old).await?;
            }
            session::create(&app.db, jar, None, Stage::Anonymous, &user_agent(&headers), app.secure_cookies).await?
        }
    };
    let error = (q.reason.as_deref() == Some("tries")).then_some(TOO_MANY);
    Ok((jar, page(LockedPage { theme: "system", csrf: &s.csrf_token, username: "", error })?).into_response())
}

pub async fn locked_submit(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(f): Form<LockedForm>,
) -> Result<Response> {
    let Some(s) = session::load(&app.db, &jar).await?.filter(|s| s.stage == Stage::Anonymous) else {
        return Ok(Redirect::to("/locked").into_response());
    };
    if !session::token_matches(&s.csrf_token, &f.csrf) {
        return Ok(forbidden());
    }
    let username = f.username.trim().to_lowercase();
    let user_id = sqlx::query_scalar!(r#"SELECT id AS "id!" FROM users WHERE username = ? AND deleted_at IS NULL"#, username)
        .fetch_optional(&app.db)
        .await?;
    let t = now();
    let mut tx = app.db.begin().await?;
    let used = match (user_id, canonical(&f.code)) {
        (Some(uid), Some(code)) => {
            let h = hash(&code);
            sqlx::query!(
                "UPDATE admin_codes SET used_at = ?
                 WHERE user_id = ? AND code_hash = ? AND used_at IS NULL AND revoked_at IS NULL AND expires_at > ?",
                t,
                uid,
                h,
                t
            )
            .execute(&mut *tx)
            .await?
            .rows_affected()
                == 1
        }
        _ => false,
    };
    let (Some(user_id), true) = (user_id, used) else {
        drop(tx);
        if session::record_failure(&app.db, &s).await? >= MAX_TRIES {
            session::end(&app.db, &s).await?;
            return Ok((session::clear_cookie(jar), Redirect::to("/locked?reason=tries")).into_response());
        }
        return page(LockedPage { theme: "system", csrf: &s.csrf_token, username: &username, error: Some(WRONG) });
    };
    sqlx::query!(
        "UPDATE users SET must_reset_password = 1, totp_enabled = 0, totp_secret_enc = NULL, totp_pending_enc = NULL,
                          totp_enabled_at = NULL, totp_last_step = NULL
         WHERE id = ?",
        user_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM recovery_codes WHERE user_id = ?", user_id).execute(&mut *tx).await?;
    // Whoever might have been using the account is signed out everywhere.
    sqlx::query!("DELETE FROM sessions WHERE user_id = ?", user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    let (_, jar) =
        session::advance(&app.db, jar, &s, user_id, Stage::SignedIn, &user_agent(&headers), app.secure_cookies).await?;
    Ok((jar, Redirect::to("/reset-password")).into_response())
}

// ---------------------------------------------------------------- /reset-password

#[derive(Template)]
#[template(path = "reset_password.html")]
struct ResetPage<'a> {
    theme: &'a str,
    csrf: &'a str,
    username: &'a str,
    error: Option<&'a str>,
}

#[derive(Deserialize)]
pub struct ResetForm {
    csrf: String,
    new: String,
    confirm: String,
}

pub async fn reset_page(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let ctx = match load_user(&app, &jar, true).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.must_reset {
        return Ok(Redirect::to("/settings#pw-h").into_response());
    }
    page(ResetPage { theme: &ctx.theme, csrf: &ctx.session.csrf_token, username: &ctx.username, error: None })
}

pub async fn reset_submit(State(app): State<Shared>, jar: CookieJar, Form(f): Form<ResetForm>) -> Result<Response> {
    let ctx = match load_user(&app, &jar, true).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(&f.csrf) {
        return Ok(forbidden());
    }
    if !ctx.must_reset {
        return Ok(Redirect::to("/settings#pw-h").into_response());
    }
    let error = if f.new.chars().count() < auth::password::MIN_LENGTH || f.new.len() > 1024 {
        Some("Use a password of at least 12 characters.")
    } else if f.new != f.confirm {
        Some("The two passwords don’t match.")
    } else {
        None
    };
    if let Some(error) = error {
        return page(ResetPage { theme: &ctx.theme, csrf: &ctx.session.csrf_token, username: &ctx.username, error: Some(error) });
    }
    let new = f.new.clone();
    let h = tokio::task::spawn_blocking(move || auth::password::hash(&new)).await?;
    sqlx::query!("UPDATE users SET password_hash = ?, must_reset_password = 0 WHERE id = ?", h, ctx.user_id)
        .execute(&app.db)
        .await?;
    Ok(Redirect::to("/settings/2fa/setup?first=1").into_response())
}

#[cfg(test)]
mod tests {
    #[test]
    fn codes_normalize() {
        assert_eq!(super::canonical("r7kq 2m9x pd4t"), Some("R7KQ-2M9X-PD4T".into()));
        assert_eq!(super::canonical("R7KQ-2M9X-PD4"), None);
        let c = super::new_code();
        assert_eq!(super::canonical(&c), Some(c.clone()));
    }
}
