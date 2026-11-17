//! The Settings page: account, password, 2FA status, appearance, working calendar, sessions and,
//! for Administrators, the Administrator section (sign-up mode and invitations).

use askama::Template;
use axum::extract::{Form, Query, State};
use axum::http::{HeaderMap, header};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use rand::Rng;
use serde::Deserialize;

use crate::session::{self, describe_agent};
use crate::util::{ago, date, is_iso_date};
use crate::{AppState, Ctx, Result, Shared, auth, forbidden, initials, now, page, require_user};

const INVITE_DAYS: i64 = 7;
const DAYS: [&str; 7] = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];

pub struct Day {
    pub index: usize,
    pub label: &'static str,
    pub on: bool,
}

pub struct SessionRow {
    pub reference: String,
    pub device: String,
    pub seen: String,
    pub current: bool,
}

pub struct InviteRow {
    pub id: i64,
    pub note: String,
    pub created: String,
    pub status: String,
    pub active: bool,
}

pub struct CodeRow {
    pub id: i64,
    pub username: String,
    pub issued_by: String,
    pub created: String,
    pub status: String,
    pub active: bool,
}

pub struct ArchivedRow {
    pub id: i64,
    pub title: String,
    pub kind: &'static str,
    pub owner: String,
    pub archived: String,
    pub also: usize,
}

pub struct AdminView {
    pub signup_mode: String,
    pub invites: Vec<InviteRow>,
    pub new_link: Option<String>,
    pub codes: Vec<CodeRow>,
    pub issued: Option<crate::recovery::Issued>,
    pub lookup: String,
    pub archived: Vec<ArchivedRow>,
    pub people: Vec<crate::admin::AdminPerson>,
    pub admins: usize,
    pub unowned: Vec<crate::admin::UnownedRow>,
}

/// Things shown once on the page that follows an action.
#[derive(Default)]
pub struct Shown {
    pub new_link: Option<String>,
    pub issued: Option<crate::recovery::Issued>,
    pub lookup: String,
}

#[derive(Template)]
#[template(path = "settings.html")]
struct SettingsPage<'a> {
    theme: &'a str,
    focus_count: usize,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
    display_name: String,
    is_admin: bool,
    totp_enabled: bool,
    totp_since: String,
    codes_left: i64,
    days: Vec<Day>,
    time_zone: String,
    days_off: String,
    sessions: Vec<SessionRow>,
    notice_for: &'a str,
    notice: &'a str,
    is_error: bool,
    admin: Option<AdminView>,
}

#[derive(Deserialize)]
pub struct Notice {
    done: Option<String>,
    error: Option<String>,
}

/// (section, message, is_error) for a `?done=` or `?error=` key.
fn notice(q: &Notice) -> (&'static str, &'static str, bool) {
    if let Some(k) = q.done.as_deref() {
        let (s, m) = match k {
            "profile" => ("account", "Saved."),
            "password" => ("password", "Password changed. Your other devices were signed out."),
            "theme" => ("appearance", "Saved."),
            "calendar" => ("calendar", "Saved."),
            "session" => ("sessions", "Signed out of that device."),
            "others" => ("sessions", "Signed out everywhere else."),
            "2fa-off" => ("tfa", "Two-factor authentication is off. Your recovery codes no longer work."),
            "recovery-used" => ("tfa", "You signed in with a recovery code, which can’t be used again."),
            "signup" => ("admin", "Saved."),
            "revoked" => ("admin", "Invitation revoked."),
            "code-revoked" => ("recovery", "Recovery code cancelled."),
            "deleted" => ("archived", "Deleted permanently."),
            "admin-added" => ("roles", "They’re now an Administrator."),
            "admin-removed" => ("roles", "They’re no longer an Administrator."),
            "assigned" => ("unowned", "Offered to them. It stays here until they accept."),
            _ => return ("", "", false),
        };
        return (s, m, false);
    }
    if let Some(k) = q.error.as_deref() {
        let (s, m) = match k {
            "name-long" => ("account", "Use 64 characters or fewer."),
            "password-current" => ("password", "Your current password isn’t right."),
            "password-short" => ("password", "Use a new password of at least 12 characters."),
            "password-mismatch" => ("password", "The two new passwords don’t match."),
            "codes-password" => ("tfa", "Your password isn’t right, so no new codes were made."),
            "2fa-off" => ("tfa", "Your password or code wasn’t right, so two-factor authentication is still on."),
            "calendar-none" => ("calendar", "Pick at least one working day."),
            "throttled-pw" => ("password", crate::throttle::MESSAGE),
            "throttled-tfa" => ("tfa", crate::throttle::MESSAGE),
            "last-admin" => ("roles", "There has to be at least one Administrator. Make someone else one first."),
            "no-user" => ("unowned", "No one has that username."),
            "calendar-tz" => ("calendar", "Enter a time zone name like Europe/Lisbon or America/New_York."),
            "calendar-days" => ("calendar", "Write days off as dates like 2026-12-24, separated by commas."),
            _ => return ("", "", false),
        };
        return (s, m, true);
    }
    ("", "", false)
}

fn link_base(headers: &HeaderMap, secure: bool) -> String {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
    format!("{}://{host}", if secure { "https" } else { "http" })
}

async fn render(app: &AppState, ctx: &Ctx, n: (&str, &str, bool), shown: Shown) -> Result<Response> {
    let u = sqlx::query!("SELECT display_name, totp_enabled, totp_enabled_at FROM users WHERE id = ?", ctx.user_id)
        .fetch_one(&app.db)
        .await?;
    let codes_left =
        sqlx::query_scalar!("SELECT COUNT(*) FROM recovery_codes WHERE user_id = ? AND used_at IS NULL", ctx.user_id)
            .fetch_one(&app.db)
            .await?;
    let cal =
        sqlx::query!("SELECT workdays_mask, time_zone, days_off FROM user_calendar WHERE user_id = ?", ctx.user_id)
            .fetch_optional(&app.db)
            .await?;
    let (mask, time_zone, days_off) = match cal {
        Some(c) => (c.workdays_mask, c.time_zone, c.days_off),
        None => (31, "UTC".into(), "[]".into()),
    };
    let days_off: Vec<String> = serde_json::from_str(&days_off).unwrap_or_default();
    let t = now();
    let current = ctx.session.reference();
    let sessions = session::list_signed_in(&app.db, ctx.user_id)
        .await?
        .into_iter()
        .map(|s| SessionRow {
            current: s.reference == current,
            device: describe_agent(&s.user_agent),
            seen: ago(s.last_seen_at, t),
            reference: s.reference,
        })
        .collect();
    let admin = if ctx.is_admin {
        let signup_mode =
            sqlx::query_scalar!("SELECT signup_mode FROM instance WHERE id = 1").fetch_one(&app.db).await?;
        let rows = sqlx::query!(
            r#"SELECT i.id AS "id!", i.note, i.created_at, i.expires_at, i.used_at, i.revoked_at, u.username AS "used_by?"
               FROM signup_invites i LEFT JOIN users u ON u.id = i.used_by
               ORDER BY i.created_at DESC LIMIT 20"#
        )
        .fetch_all(&app.db)
        .await?;
        let invites = rows
            .into_iter()
            .map(|r| {
                let (status, active) = if let Some(by) = r.used_by.as_ref().filter(|_| r.used_at.is_some()) {
                    (format!("Used by {by}"), false)
                } else if r.revoked_at.is_some() {
                    ("Revoked".into(), false)
                } else if r.expires_at <= t {
                    ("Expired".into(), false)
                } else {
                    (format!("Open until {}", date(r.expires_at)), true)
                };
                InviteRow { id: r.id, note: r.note, created: date(r.created_at), status, active }
            })
            .collect();
        let rows = sqlx::query!(
            r#"SELECT c.id AS "id!", u.username, b.username AS issued_by, c.created_at, c.expires_at, c.used_at, c.revoked_at
               FROM admin_codes c JOIN users u ON u.id = c.user_id JOIN users b ON b.id = c.issued_by
               ORDER BY c.created_at DESC LIMIT 20"#
        )
        .fetch_all(&app.db)
        .await?;
        let codes = rows
            .into_iter()
            .map(|r| {
                let (status, active) = if r.used_at.is_some() {
                    ("Used".to_string(), false)
                } else if r.revoked_at.is_some() {
                    ("Cancelled".into(), false)
                } else if r.expires_at <= t {
                    ("Expired".into(), false)
                } else {
                    ("Active".into(), true)
                };
                let issued_by =
                    if r.issued_by == ctx.username { format!("{} (you)", r.issued_by) } else { r.issued_by };
                CodeRow { id: r.id, username: r.username, issued_by, created: date(r.created_at), status, active }
            })
            .collect();
        let world = crate::store::World::load(&app.db).await?;
        let names = crate::store::usernames(&app.db).await?;
        let mut archived: Vec<&crate::store::Node> = world.nodes.values().filter(|n| n.archived_at.is_some()).collect();
        archived.sort_by_key(|n| std::cmp::Reverse(n.archived_at));
        let archived = archived
            .into_iter()
            .map(|n| ArchivedRow {
                id: n.id,
                title: n.title.clone(),
                kind: if n.is_goal() { "Goal" } else { "Task" },
                owner: n.owner_id.and_then(|o| names.get(&o).cloned()).unwrap_or_else(|| "no one".into()),
                archived: n.archived_at.map(date).unwrap_or_default(),
                also: crate::nodes::doomed(&world, n.id).len() - 1,
            })
            .collect();
        let people = crate::admin::people(&app.db, ctx.user_id).await?;
        let admins = people.iter().filter(|p| p.is_admin).count();
        let unowned = crate::admin::unowned(&app.db, &world).await?;
        Some(AdminView {
            people,
            admins,
            unowned,
            archived,
            signup_mode,
            invites,
            new_link: shown.new_link,
            codes,
            issued: shown.issued,
            lookup: shown.lookup,
        })
    } else {
        None
    };
    page(SettingsPage {
        focus_count: crate::updates::badge(&app.db, ctx.user_id).await?,
        theme: &ctx.theme,
        nav: "settings",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
        display_name: u.display_name,
        is_admin: ctx.is_admin,
        totp_enabled: u.totp_enabled != 0,
        totp_since: u.totp_enabled_at.map(date).unwrap_or_default(),
        codes_left,
        days: DAYS.iter().enumerate().map(|(i, l)| Day { index: i, label: l, on: mask & (1 << i) != 0 }).collect(),
        time_zone,
        days_off: days_off.join(", "),
        sessions,
        notice_for: n.0,
        notice: n.1,
        is_error: n.2,
        admin,
    })
}

macro_rules! signed_in {
    ($app:expr, $jar:expr) => {
        match require_user(&$app, &$jar).await? {
            Ok(c) => c,
            Err(r) => return Ok(r),
        }
    };
}

macro_rules! check_csrf {
    ($ctx:expr, $token:expr) => {
        if !$ctx.csrf_ok(&$token) {
            return Ok(forbidden());
        }
    };
}

fn back(query: &str, anchor: &str) -> Response {
    Redirect::to(&format!("/settings?{query}#{anchor}")).into_response()
}

pub async fn show(State(app): State<Shared>, jar: CookieJar, Query(q): Query<Notice>) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    render(&app, &ctx, notice(&q), Shown::default()).await
}

#[derive(Deserialize)]
pub struct ProfileForm {
    csrf: String,
    display_name: String,
}

pub async fn profile(State(app): State<Shared>, jar: CookieJar, Form(f): Form<ProfileForm>) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    let name = crate::views::clean(&f.display_name, false);
    let name = name.as_str();
    if name.chars().count() > 64 {
        return Ok(back("error=name-long", "acc-h"));
    }
    sqlx::query!("UPDATE users SET display_name = ? WHERE id = ?", name, ctx.user_id).execute(&app.db).await?;
    Ok(back("done=profile", "acc-h"))
}

#[derive(Deserialize)]
pub struct PasswordForm {
    csrf: String,
    current: String,
    new: String,
    confirm: String,
}

pub async fn password(
    State(app): State<Shared>,
    jar: CookieJar,
    axum::Extension(crate::throttle::ClientIp(ip)): axum::Extension<crate::throttle::ClientIp>,
    Form(f): Form<PasswordForm>,
) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    let hash =
        sqlx::query_scalar!("SELECT password_hash FROM users WHERE id = ?", ctx.user_id).fetch_one(&app.db).await?;
    match crate::throttle::confirm_password(&app.db, &ctx.username, &ip, &hash, &f.current).await? {
        crate::throttle::Check::Ok => {}
        crate::throttle::Check::Wrong => return Ok(back("error=password-current", "pw-h")),
        crate::throttle::Check::Blocked => return Ok(back("error=throttled-pw", "pw-h")),
    }
    if f.new.chars().count() < auth::password::MIN_LENGTH || f.new.len() > 1024 {
        return Ok(back("error=password-short", "pw-h"));
    }
    if f.new != f.confirm {
        return Ok(back("error=password-mismatch", "pw-h"));
    }
    let new = f.new.clone();
    let hash = tokio::task::spawn_blocking(move || auth::password::hash(&new)).await?;
    sqlx::query!("UPDATE users SET password_hash = ? WHERE id = ?", hash, ctx.user_id).execute(&app.db).await?;
    session::end_others(&app.db, ctx.user_id, &ctx.session).await?;
    Ok(back("done=password", "pw-h"))
}

#[derive(Deserialize)]
pub struct ThemeForm {
    csrf: String,
    theme: String,
}

pub async fn theme(State(app): State<Shared>, jar: CookieJar, Form(f): Form<ThemeForm>) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    if !matches!(f.theme.as_str(), "system" | "light" | "dark") {
        return Ok(back("", "app-h"));
    }
    sqlx::query!("UPDATE users SET theme = ? WHERE id = ?", f.theme, ctx.user_id).execute(&app.db).await?;
    Ok(back("done=theme", "app-h"))
}

#[derive(Deserialize)]
pub struct CalendarForm {
    csrf: String,
    d0: Option<String>,
    d1: Option<String>,
    d2: Option<String>,
    d3: Option<String>,
    d4: Option<String>,
    d5: Option<String>,
    d6: Option<String>,
    time_zone: String,
    days_off: String,
}

pub async fn calendar(State(app): State<Shared>, jar: CookieJar, Form(f): Form<CalendarForm>) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    let mask = [&f.d0, &f.d1, &f.d2, &f.d3, &f.d4, &f.d5, &f.d6]
        .iter()
        .enumerate()
        .fold(0i64, |m, (i, d)| if d.is_some() { m | (1 << i) } else { m });
    if mask == 0 {
        return Ok(back("error=calendar-none", "cal-h"));
    }
    let tz = f.time_zone.trim();
    let tz_ok = tz.len() <= 64 && fluxo_core::calendar::valid_time_zone(tz);
    if !tz_ok {
        return Ok(back("error=calendar-tz", "cal-h"));
    }
    let days: Vec<&str> = f.days_off.split([',', '\n', ' ']).map(str::trim).filter(|d| !d.is_empty()).collect();
    if !days.iter().all(|d| is_iso_date(d)) {
        return Ok(back("error=calendar-days", "cal-h"));
    }
    let mut days: Vec<&str> = days;
    days.sort_unstable();
    days.dedup();
    let json = serde_json::to_string(&days)?;
    sqlx::query!(
        "INSERT INTO user_calendar (user_id, workdays_mask, time_zone, days_off) VALUES (?, ?, ?, ?)
         ON CONFLICT (user_id) DO UPDATE SET workdays_mask = excluded.workdays_mask,
             time_zone = excluded.time_zone, days_off = excluded.days_off",
        ctx.user_id,
        mask,
        tz,
        json
    )
    .execute(&app.db)
    .await?;
    Ok(back("done=calendar", "cal-h"))
}

#[derive(Deserialize)]
pub struct EndSessionForm {
    csrf: String,
    session: String,
}

pub async fn end_session(State(app): State<Shared>, jar: CookieJar, Form(f): Form<EndSessionForm>) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    session::end_by_reference(&app.db, ctx.user_id, &f.session).await?;
    Ok(back("done=session", "ses-h"))
}

#[derive(Deserialize)]
pub struct CsrfForm {
    csrf: String,
}

pub async fn end_other_sessions(
    State(app): State<Shared>,
    jar: CookieJar,
    Form(f): Form<CsrfForm>,
) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    session::end_others(&app.db, ctx.user_id, &ctx.session).await?;
    Ok(back("done=others", "ses-h"))
}

// ---------------------------------------------------------------- Administrator section

#[derive(Deserialize)]
pub struct SignupModeForm {
    csrf: String,
    mode: String,
}

pub async fn signup_mode(State(app): State<Shared>, jar: CookieJar, Form(f): Form<SignupModeForm>) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    if !ctx.is_admin {
        return Ok(forbidden());
    }
    if matches!(f.mode.as_str(), "open" | "invite" | "closed") {
        sqlx::query!("UPDATE instance SET signup_mode = ? WHERE id = 1", f.mode).execute(&app.db).await?;
    }
    Ok(back("done=signup", "adm-h"))
}

#[derive(Deserialize)]
pub struct InviteForm {
    csrf: String,
    #[serde(default)]
    note: String,
}

pub async fn create_invite(
    State(app): State<Shared>,
    jar: CookieJar,
    headers: HeaderMap,
    Form(f): Form<InviteForm>,
) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    if !ctx.is_admin {
        return Ok(forbidden());
    }
    let mut raw = [0u8; 24];
    rand::rng().fill_bytes(&mut raw);
    let token = URL_SAFE_NO_PAD.encode(raw);
    let hash = crate::account::token_hash(&token);
    let note: String = crate::views::clean(&f.note, false).chars().take(100).collect();
    let (t, expires) = (now(), now() + INVITE_DAYS * 86_400);
    sqlx::query!(
        "INSERT INTO signup_invites (token_hash, created_by, note, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
        hash,
        ctx.user_id,
        note,
        t,
        expires
    )
    .execute(&app.db)
    .await?;
    // The link is shown once, on this page, and never stored.
    let base = app.public_url.clone().unwrap_or_else(|| link_base(&headers, app.secure_cookies));
    let link = format!("{base}/join/{token}");
    let shown = Shown { new_link: Some(link), ..Default::default() };
    render(&app, &ctx, ("admin", "Invitation created. Copy the link now: it won’t be shown again.", false), shown).await
}

#[derive(Deserialize)]
pub struct RevokeForm {
    csrf: String,
    invite: i64,
}

pub async fn revoke_invite(State(app): State<Shared>, jar: CookieJar, Form(f): Form<RevokeForm>) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    if !ctx.is_admin {
        return Ok(forbidden());
    }
    let t = now();
    sqlx::query!(
        "UPDATE signup_invites SET revoked_at = ? WHERE id = ? AND used_at IS NULL AND revoked_at IS NULL",
        t,
        f.invite
    )
    .execute(&app.db)
    .await?;
    Ok(back("done=revoked", "adm-h"))
}

#[derive(Deserialize)]
pub struct RecoveryForm {
    csrf: String,
    username: String,
}

pub async fn issue_recovery_code(
    State(app): State<Shared>,
    jar: CookieJar,
    Form(f): Form<RecoveryForm>,
) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    if !ctx.is_admin {
        return Ok(forbidden());
    }
    use crate::recovery::IssueOutcome;
    let lookup = f.username.trim().to_string();
    let (n, shown) = match crate::recovery::issue(&app, ctx.user_id, &f.username).await? {
        IssueOutcome::Issued(i) => (
            ("recovery", "Recovery code created. Give it to them directly; it won’t be shown again.", false),
            Shown { issued: Some(i), ..Default::default() },
        ),
        IssueOutcome::NoSuchUser => {
            (("recovery", "No one has that username.", true), Shown { lookup, ..Default::default() })
        }
        IssueOutcome::Yourself => (
            ("recovery", "You can’t make a recovery code for yourself. Ask another Administrator.", true),
            Shown { lookup, ..Default::default() },
        ),
    };
    render(&app, &ctx, n, shown).await
}

#[derive(Deserialize)]
pub struct RevokeCodeForm {
    csrf: String,
    code: i64,
}

pub async fn revoke_recovery_code(
    State(app): State<Shared>,
    jar: CookieJar,
    Form(f): Form<RevokeCodeForm>,
) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    if !ctx.is_admin {
        return Ok(forbidden());
    }
    let t = now();
    sqlx::query!(
        "UPDATE admin_codes SET revoked_at = ? WHERE id = ? AND used_at IS NULL AND revoked_at IS NULL",
        t,
        f.code
    )
    .execute(&app.db)
    .await?;
    Ok(back("done=code-revoked", "rec-h"))
}
