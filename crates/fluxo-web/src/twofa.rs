//! Two-factor authentication: setting it up, recovery codes, and turning it off.

use askama::Template;
use axum::extract::{Form, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use rand::Rng;
use serde::Deserialize;

use crate::{AppState, Result, Shared, auth, forbidden, now, page, require_user};

const CODES: usize = 8;
const CODE_ALPHABET: &[u8] = b"abcdefghjkmnpqrstuvwxyz23456789"; // no i, l, o, 0, 1

#[derive(Template)]
#[template(path = "twofa_setup.html")]
struct SetupPage<'a> {
    theme: &'a str,
    csrf: &'a str,
    first: bool,
    replacing: bool,
    qr_png_base64: String,
    secret_text: String,
    error: Option<&'a str>,
}

#[derive(Template)]
#[template(path = "twofa_codes.html")]
struct CodesPage<'a> {
    theme: &'a str,
    codes: Vec<String>,
    codes_text: String,
    continue_to: &'a str,
    first: bool,
}

#[derive(Deserialize)]
pub struct FirstQuery {
    first: Option<String>,
}

#[derive(Deserialize)]
pub struct SetupForm {
    csrf: String,
    code: String,
    #[serde(default)]
    first: String,
}

#[derive(Deserialize)]
pub struct PasswordForm {
    csrf: String,
    password: String,
}

#[derive(Deserialize)]
pub struct OffForm {
    csrf: String,
    password: String,
    code: String,
}

fn label(user_id: i64) -> String {
    format!("totp:{user_id}")
}

/// Check a code for a user with 2FA on, and claim its time step so it can't be used again.
pub(crate) async fn accept_code(app: &AppState, user_id: i64, code: &str) -> Result<bool> {
    let u = sqlx::query!("SELECT totp_secret_enc, totp_last_step FROM users WHERE id = ? AND totp_enabled = 1", user_id)
        .fetch_optional(&app.db)
        .await?;
    let Some(u) = u else { return Ok(false) };
    let secret = app.secrets.open(&label(user_id), &u.totp_secret_enc.ok_or("2FA is on but no secret is stored")?)?;
    let Some(step) = auth::totp::check(&secret, code, now() as u64, u.totp_last_step.map(|x| x as u64)) else {
        return Ok(false);
    };
    let step = step as i64;
    // Claim the step atomically so two simultaneous requests can't both use one code.
    let claimed = sqlx::query!(
        "UPDATE users SET totp_last_step = ? WHERE id = ? AND (totp_last_step IS NULL OR totp_last_step < ?)",
        step,
        user_id,
        step
    )
    .execute(&app.db)
    .await?
    .rows_affected();
    Ok(claimed == 1)
}

/// Canonical form of a recovery code: lowercase, `xxxx-xxxx`.
fn canonical(code: &str) -> Option<String> {
    let s: String = code.chars().filter(|c| c.is_ascii_alphanumeric()).map(|c| c.to_ascii_lowercase()).collect();
    (s.len() == 8).then(|| format!("{}-{}", &s[..4], &s[4..]))
}

/// Use one of a person's recovery codes. Each works once.
pub(crate) async fn use_recovery_code(app: &AppState, user_id: i64, code: &str) -> Result<bool> {
    let Some(code) = canonical(code) else { return Ok(false) };
    let rows = sqlx::query!("SELECT id, code_hash FROM recovery_codes WHERE user_id = ? AND used_at IS NULL", user_id)
        .fetch_all(&app.db)
        .await?;
    let hit = tokio::task::spawn_blocking(move || {
        rows.into_iter().find(|r| auth::password::verify(&code, &r.code_hash)).map(|r| r.id)
    })
    .await?;
    let Some(id) = hit else { return Ok(false) };
    let t = now();
    let used = sqlx::query!("UPDATE recovery_codes SET used_at = ? WHERE id = ? AND used_at IS NULL", t, id)
        .execute(&app.db)
        .await?
        .rows_affected();
    Ok(used == 1)
}

/// Replace a person's recovery codes with a fresh set and return them (shown once, never stored).
async fn new_codes(app: &AppState, user_id: i64) -> Result<Vec<String>> {
    let codes: Vec<String> = (0..CODES)
        .map(|_| {
            let mut rng = rand::rng();
            let mut pick = || CODE_ALPHABET[(rng.next_u32() as usize) % CODE_ALPHABET.len()] as char;
            let a: String = (0..4).map(|_| pick()).collect();
            let b: String = (0..4).map(|_| pick()).collect();
            format!("{a}-{b}")
        })
        .collect();
    let to_hash = codes.clone();
    let hashes = tokio::task::spawn_blocking(move || to_hash.iter().map(|c| auth::password::hash(c)).collect::<Vec<_>>())
        .await?;
    let t = now();
    let mut tx = app.db.begin().await?;
    sqlx::query!("DELETE FROM recovery_codes WHERE user_id = ?", user_id).execute(&mut *tx).await?;
    for h in &hashes {
        sqlx::query!("INSERT INTO recovery_codes (user_id, code_hash, created_at) VALUES (?, ?, ?)", user_id, h, t)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    Ok(codes)
}

fn codes_page(theme: &str, codes: Vec<String>, first: bool) -> Result<Response> {
    let codes_text = codes.join("\n");
    page(CodesPage {
        theme,
        codes,
        codes_text,
        continue_to: if first { "/" } else { "/settings#tfa-h" },
        first,
    })
}

pub async fn setup_page(State(app): State<Shared>, jar: CookieJar, Query(q): Query<FirstQuery>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let u = sqlx::query!("SELECT totp_pending_enc, totp_enabled FROM users WHERE id = ?", ctx.user_id)
        .fetch_one(&app.db)
        .await?;
    // Keep a pending secret across reloads, so a code scanned a moment ago still matches.
    let pending = match u.totp_pending_enc.map(|p| app.secrets.open(&format!("totp-pending:{}", ctx.user_id), &p)) {
        Some(Ok(secret)) => secret,
        _ => {
            let secret = auth::totp::generate_secret();
            let sealed = app.secrets.seal(&format!("totp-pending:{}", ctx.user_id), &secret);
            sqlx::query!("UPDATE users SET totp_pending_enc = ? WHERE id = ?", sealed, ctx.user_id)
                .execute(&app.db)
                .await?;
            secret
        }
    };
    let (qr_png_base64, secret_text) = auth::totp::provisioning(&pending, &ctx.username);
    page(SetupPage {
        theme: &ctx.theme,
        csrf: &ctx.session.csrf_token,
        first: q.first.is_some(),
        replacing: u.totp_enabled != 0,
        qr_png_base64,
        secret_text,
        error: None,
    })
}

pub async fn setup_submit(State(app): State<Shared>, jar: CookieJar, Form(f): Form<SetupForm>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(&f.csrf) {
        return Ok(forbidden());
    }
    let first = !f.first.is_empty();
    let u = sqlx::query!("SELECT totp_pending_enc, totp_enabled FROM users WHERE id = ?", ctx.user_id)
        .fetch_one(&app.db)
        .await?;
    let Some(sealed) = u.totp_pending_enc else {
        return Ok(Redirect::to("/settings/2fa/setup").into_response());
    };
    let secret = app.secrets.open(&format!("totp-pending:{}", ctx.user_id), &sealed)?;
    let Some(step) = auth::totp::check(&secret, &f.code, now() as u64, None) else {
        let (qr_png_base64, secret_text) = auth::totp::provisioning(&secret, &ctx.username);
        return page(SetupPage {
            theme: &ctx.theme,
            csrf: &ctx.session.csrf_token,
            first,
            replacing: u.totp_enabled != 0,
            qr_png_base64,
            secret_text,
            error: Some("That code didn’t match. Enter the newest code your app shows for fluxo."),
        });
    };
    let active = app.secrets.seal(&label(ctx.user_id), &secret);
    let (t, step) = (now(), step as i64);
    sqlx::query!(
        "UPDATE users SET totp_secret_enc = ?, totp_pending_enc = NULL, totp_enabled = 1, totp_last_step = ?,
                          totp_enabled_at = ? WHERE id = ?",
        active,
        step,
        t,
        ctx.user_id
    )
    .execute(&app.db)
    .await?;
    let codes = new_codes(&app, ctx.user_id).await?;
    codes_page(&ctx.theme, codes, first)
}

pub async fn regenerate_codes(State(app): State<Shared>, jar: CookieJar, Form(f): Form<PasswordForm>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(&f.csrf) {
        return Ok(forbidden());
    }
    let u = sqlx::query!("SELECT password_hash, totp_enabled FROM users WHERE id = ?", ctx.user_id)
        .fetch_one(&app.db)
        .await?;
    if u.totp_enabled == 0 || !auth::password::verify(&f.password, &u.password_hash) {
        return Ok(Redirect::to("/settings?error=codes-password#tfa-h").into_response());
    }
    let codes = new_codes(&app, ctx.user_id).await?;
    codes_page(&ctx.theme, codes, false)
}

pub async fn turn_off(State(app): State<Shared>, jar: CookieJar, Form(f): Form<OffForm>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(&f.csrf) {
        return Ok(forbidden());
    }
    let hash = sqlx::query_scalar!("SELECT password_hash FROM users WHERE id = ?", ctx.user_id)
        .fetch_one(&app.db)
        .await?;
    if !auth::password::verify(&f.password, &hash) || !accept_code(&app, ctx.user_id, &f.code).await? {
        return Ok(Redirect::to("/settings?error=2fa-off#tfa-h").into_response());
    }
    let mut tx = app.db.begin().await?;
    sqlx::query!(
        "UPDATE users SET totp_enabled = 0, totp_secret_enc = NULL, totp_pending_enc = NULL, totp_enabled_at = NULL
         WHERE id = ?",
        ctx.user_id
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!("DELETE FROM recovery_codes WHERE user_id = ?", ctx.user_id).execute(&mut *tx).await?;
    tx.commit().await?;
    Ok(Redirect::to("/settings?done=2fa-off#tfa-h").into_response())
}

#[cfg(test)]
mod tests {
    #[test]
    fn recovery_codes_normalize() {
        assert_eq!(super::canonical("K7QD-2MXA"), Some("k7qd-2mxa".into()));
        assert_eq!(super::canonical(" k7qd 2mxa "), Some("k7qd-2mxa".into()));
        assert_eq!(super::canonical("k7qd2mx"), None);
    }
}
