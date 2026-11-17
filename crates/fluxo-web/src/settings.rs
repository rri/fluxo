//! The Settings page: account, password, 2FA status, appearance, working calendar, sessions and,
//! for Administrators, the Admin section.
//!
//! Every form posts, then redirects back (so reloading never resubmits). Secrets made by a post
//! (an invitation link, a recovery code) ride across that redirect in memory, keyed by session,
//! and are shown once, next to where they were made.

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
use crate::util::{ago, date, day_number, iso};
use crate::{AppState, Ctx, Result, Shared, auth, forbidden, initials, now, page, require_user};

const INVITE_DAYS: i64 = 7;
/// How long archived items can be kept, in days (Administrators pick one).
pub const RETENTION_CHOICES: [i64; 5] = [7, 30, 90, 180, 365];
/// Longest days-off range added at once, and most days off kept.
const MAX_RANGE: i64 = 366;
const MAX_DAYS_OFF: usize = 3_000;
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
    /// Just made: the link, shown this once.
    pub link: Option<String>,
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
    pub title: String,
    pub kind: &'static str,
    pub owner: String,
    pub archived: String,
    pub purge: String,
    pub also: usize,
}

/// A day off, or a run of consecutive days off, as one chip.
pub struct DayOff {
    pub from: String,
    pub to: String,
    pub label: String,
}

pub struct AdminView {
    pub signup_mode: String,
    pub invites: Vec<InviteRow>,
    pub codes: Vec<CodeRow>,
    pub issued: Option<crate::recovery::Issued>,
    pub lookup: String,
    pub archived: Vec<ArchivedRow>,
    pub people: Vec<crate::admin::AdminPerson>,
    pub admins: usize,
    pub retention: i64,
    pub unowned: Vec<crate::admin::UnownedRow>,
}

/// Things shown once on the page that follows an action.
#[derive(Default)]
pub struct Shown {
    /// The invitation just made, and its link.
    pub new_invite: Option<(i64, String)>,
    pub issued: Option<crate::recovery::Issued>,
    pub lookup: String,
    /// New 2FA recovery codes, and whether this is first-time setup.
    pub codes: Option<(Vec<String>, bool)>,
}

/// Shown values waiting for the redirect that follows a post, by session. Never written to disk.
#[derive(Default)]
pub struct Flash(std::sync::Mutex<std::collections::HashMap<String, (i64, Shown)>>);

impl Flash {
    const TTL: i64 = 300;

    pub(crate) fn put(&self, session: String, shown: Shown) {
        let t = now();
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        m.retain(|_, (at, _)| t - *at < Self::TTL);
        m.insert(session, (t, shown));
    }

    pub(crate) fn take(&self, session: &str) -> Shown {
        let mut m = self.0.lock().unwrap_or_else(|e| e.into_inner());
        match m.remove(session) {
            Some((at, shown)) if now() - at < Self::TTL => shown,
            _ => Shown::default(),
        }
    }
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
    days_off: Vec<DayOff>,
    sessions: Vec<SessionRow>,
    retention_choices: [i64; 5],
    notice_for: &'a str,
    notice: Vec<(String, bool)>,
    is_error: bool,
    admin: Option<AdminView>,
}

#[derive(Deserialize)]
pub struct Notice {
    done: Option<String>,
    error: Option<String>,
    /// The person a notice is about (by id, so a link can't put words in fluxo's mouth).
    user: Option<i64>,
}

/// A notice as text runs; `true` runs are bold.
type Runs = Vec<(String, bool)>;

fn plain(s: &str) -> Runs {
    vec![(s.to_string(), false)]
}

/// (section, message, is_error) for a `?done=` or `?error=` key.
fn notice(q: &Notice, who: &str) -> (&'static str, Runs, bool) {
    let about = |before: &str, after: Runs| {
        let mut r = plain(before);
        r.push((who.to_string(), true));
        r.extend(after);
        r.retain(|(t, _)| !t.is_empty());
        r
    };
    match (q.done.as_deref(), who.is_empty()) {
        (Some("admin-added"), false) => {
            let after = vec![(" is now an ".into(), false), ("Administrator".into(), true), (".".into(), false)];
            return ("roles", about("", after), false);
        }
        (Some("admin-removed"), false) => {
            let after = vec![(" is no longer an ".into(), false), ("Administrator".into(), true), (".".into(), false)];
            return ("roles", about("", after), false);
        }
        (Some("assigned"), false) => {
            return ("unowned", about("Offered to ", plain(". It stays here until they accept.")), false);
        }
        _ => {}
    }
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
            "signup" => ("signup", "Saved."),
            "revoked" => ("invites", "Invitation revoked."),
            "code-revoked" => ("recovery", "Recovery code cancelled."),
            "retention" => ("archived", "Saved."),
            "off-added" => ("days-off", "Added."),
            "off-removed" => ("days-off", "Removed."),
            _ => return ("", vec![], false),
        };
        return (s, plain(m), false);
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
            "off-dates" => ("days-off", "Pick a start date, and an end date on or after it."),
            "off-long" => ("days-off", "Add at most a year at a time."),
            "off-many" => ("days-off", "That’s too many days off. Remove some old ones first."),
            "recovery-user" => ("recovery", "No one has that username."),
            "recovery-self" => {
                ("recovery", "You can’t generate a recovery code for yourself. Ask another Administrator.")
            }
            _ => return ("", vec![], false),
        };
        return (s, plain(m), true);
    }
    ("", vec![], false)
}

/// Consecutive dates (sorted) as chips: "Dec 24, 2026" or "Dec 24, 2026 – Jan 2, 2027".
fn day_off_chips(days: &[String]) -> Vec<DayOff> {
    let mut runs: Vec<(i64, i64)> = Vec::new();
    let mut nums: Vec<i64> = days.iter().filter_map(|d| day_number(d)).collect();
    nums.sort_unstable();
    nums.dedup();
    for n in nums {
        match runs.last_mut() {
            Some((_, to)) if *to + 1 == n => *to = n,
            _ => runs.push((n, n)),
        }
    }
    runs.into_iter()
        .map(|(a, b)| DayOff {
            from: iso(a),
            to: iso(b),
            label: if a == b { date(a * 86_400) } else { format!("{} – {}", date(a * 86_400), date(b * 86_400)) },
        })
        .collect()
}

fn link_base(headers: &HeaderMap, secure: bool) -> String {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok()).unwrap_or("localhost");
    format!("{}://{host}", if secure { "https" } else { "http" })
}

async fn render(app: &AppState, ctx: &Ctx, n: (&str, Runs, bool), shown: Shown) -> Result<Response> {
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
                let link = shown.new_invite.as_ref().filter(|(id, _)| *id == r.id).map(|(_, l)| l.clone());
                InviteRow { id: r.id, link, note: r.note, created: date(r.created_at), status, active }
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
        let retention = crate::store::retention_days(&app.db).await?;
        let world = crate::store::World::load(&app.db).await?;
        let names = crate::store::usernames(&app.db).await?;
        let mut archived: Vec<&crate::store::Node> = world.nodes.values().filter(|n| n.archived_at.is_some()).collect();
        archived.sort_by_key(|n| std::cmp::Reverse(n.archived_at));
        let archived = archived
            .into_iter()
            .map(|n| ArchivedRow {
                title: n.title.clone(),
                kind: if n.is_goal() { "Goal" } else { "Task" },
                owner: n.owner_id.and_then(|o| names.get(&o).cloned()).unwrap_or_else(|| "no one".into()),
                archived: n.archived_at.map(date).unwrap_or_default(),
                purge: n.archived_at.map(|a| date(a + retention * 86_400)).unwrap_or_default(),
                also: crate::nodes::doomed(&world, n.id).len() - 1,
            })
            .collect();
        let people = crate::admin::people(&app.db, ctx.user_id).await?;
        let admins = people.iter().filter(|p| p.is_admin).count();
        let unowned = crate::admin::unowned(&app.db, &world).await?;
        Some(AdminView {
            people,
            admins,
            retention,
            unowned,
            archived,
            signup_mode,
            invites,
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
        days_off: day_off_chips(&days_off),
        sessions,
        retention_choices: RETENTION_CHOICES,
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
    let who = match q.user {
        Some(u) => sqlx::query_scalar!("SELECT username FROM users WHERE id = ?", u)
            .fetch_optional(&app.db)
            .await?
            .unwrap_or_default(),
        None => String::new(),
    };
    let shown = app.flash.take(&ctx.session.reference());
    render(&app, &ctx, notice(&q, &who), shown).await
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
    sqlx::query!(
        "INSERT INTO user_calendar (user_id, workdays_mask, time_zone, days_off) VALUES (?, ?, ?, '[]')
         ON CONFLICT (user_id) DO UPDATE SET workdays_mask = excluded.workdays_mask, time_zone = excluded.time_zone",
        ctx.user_id,
        mask,
        tz
    )
    .execute(&app.db)
    .await?;
    Ok(back("done=calendar", "cal-h"))
}

#[derive(Deserialize)]
pub struct DaysOffForm {
    csrf: String,
    action: String,
    from: String,
    #[serde(default)]
    to: String,
}

/// Add or remove one day off, or a range of them (a blank `to` means just `from`).
pub async fn days_off(State(app): State<Shared>, jar: CookieJar, Form(f): Form<DaysOffForm>) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    let from = day_number(f.from.trim());
    let to = if f.to.trim().is_empty() { from } else { day_number(f.to.trim()) };
    let (Some(from), Some(to)) = (from, to) else {
        return Ok(Redirect::to("/settings?error=off-dates").into_response());
    };
    if to < from {
        return Ok(Redirect::to("/settings?error=off-dates").into_response());
    }
    if to - from >= MAX_RANGE {
        return Ok(Redirect::to("/settings?error=off-long").into_response());
    }
    let stored = sqlx::query_scalar!("SELECT days_off FROM user_calendar WHERE user_id = ?", ctx.user_id)
        .fetch_optional(&app.db)
        .await?
        .unwrap_or_else(|| "[]".into());
    let mut days: std::collections::BTreeSet<String> = serde_json::from_str::<Vec<String>>(&stored)
        .unwrap_or_default()
        .into_iter()
        .filter(|d| day_number(d).is_some())
        .collect();
    let range = (from..=to).map(iso);
    let adding = f.action == "add";
    if adding {
        days.extend(range);
        if days.len() > MAX_DAYS_OFF {
            return Ok(Redirect::to("/settings?error=off-many").into_response());
        }
    } else {
        for d in range {
            days.remove(&d);
        }
    }
    let json = serde_json::to_string(&days)?;
    sqlx::query!(
        "INSERT INTO user_calendar (user_id, workdays_mask, time_zone, days_off) VALUES (?, 31, 'UTC', ?)
         ON CONFLICT (user_id) DO UPDATE SET days_off = excluded.days_off",
        ctx.user_id,
        json
    )
    .execute(&app.db)
    .await?;
    // No #fragment, so the From box's autofocus works (it scrolls there too).
    Ok(Redirect::to(if adding { "/settings?done=off-added" } else { "/settings?done=off-removed" }).into_response())
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
    Ok(back("done=signup", "signup-h"))
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
    let id = sqlx::query!(
        "INSERT INTO signup_invites (token_hash, created_by, note, created_at, expires_at) VALUES (?, ?, ?, ?, ?)",
        hash,
        ctx.user_id,
        note,
        t,
        expires
    )
    .execute(&app.db)
    .await?
    .last_insert_rowid();
    // The link is shown once, next to its invitation, and never stored.
    let base = app.public_url.clone().unwrap_or_else(|| link_base(&headers, app.secure_cookies));
    let link = format!("{base}/join/{token}");
    app.flash.put(ctx.session.reference(), Shown { new_invite: Some((id, link)), ..Default::default() });
    // No #fragment: browsers skip autofocus when there is one, and focusing the link scrolls to it.
    Ok(Redirect::to("/settings").into_response())
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
    Ok(back("done=revoked", "inv-h"))
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
    let lookup: String = f.username.trim().chars().take(64).collect();
    let (to, shown) = match crate::recovery::issue(&app, ctx.user_id, &f.username).await? {
        IssueOutcome::Issued(i) => ("", Shown { issued: Some(i), ..Default::default() }),
        IssueOutcome::NoSuchUser => ("?error=recovery-user#rec-h", Shown { lookup, ..Default::default() }),
        IssueOutcome::Yourself => ("?error=recovery-self#rec-h", Shown { lookup, ..Default::default() }),
    };
    app.flash.put(ctx.session.reference(), shown);
    Ok(Redirect::to(&format!("/settings{to}")).into_response())
}

#[derive(Deserialize)]
pub struct RetentionForm {
    csrf: String,
    days: i64,
}

/// How long archived goals and tasks are kept before they're deleted for good.
pub async fn retention(State(app): State<Shared>, jar: CookieJar, Form(f): Form<RetentionForm>) -> Result<Response> {
    let ctx = signed_in!(app, jar);
    check_csrf!(ctx, f.csrf);
    if !ctx.is_admin {
        return Ok(forbidden());
    }
    if RETENTION_CHOICES.contains(&f.days) {
        sqlx::query!("UPDATE instance SET archive_retention_days = ? WHERE id = 1", f.days).execute(&app.db).await?;
        tracing::info!(by = ctx.user_id, days = f.days, "archive retention changed");
    }
    Ok(back("done=retention", "archived-h"))
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
