//! Phase 1: first-time setup, sign-up modes and invitations, 2FA setup and recovery codes, Settings.

mod common;
use common::*;

use axum::http::StatusCode;
use fluxo_web::auth::totp;
use fluxo_web::now;

fn signup_form(token: &str, username: &str) -> String {
    format!("csrf={token}&username={}&password=a+long+enough+password&confirm=a+long+enough+password", enc(username))
}

#[tokio::test]
async fn first_person_sets_up_the_instance_as_administrator() {
    let app = empty().await;
    for path in ["/", "/login", "/settings", "/signup"] {
        assert_eq!(send(&app, "GET", path, None, None).await.location.as_deref(), Some("/setup"), "{path}");
    }
    assert_eq!(send(&app, "GET", "/static/styles.css", None, None).await.status, StatusCode::OK);

    let r = send(&app, "GET", "/setup", None, None).await;
    assert!(r.body.contains("becomes the Administrator"));
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));

    let r = send(&app, "POST", "/setup", Some(&anon), Some(&signup_form(&token, "Administrator"))).await;
    assert!(r.body.contains("reserved"), "reserved names are refused");
    let r = send(&app, "POST", "/setup", Some(&anon), Some(&format!("csrf={token}&username=ada&password=short&confirm=short"))).await;
    assert!(r.body.contains("at least 12"));

    let r = send(&app, "POST", "/setup", Some(&anon), Some(&signup_form(&token, "Ada.L"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings/2fa/setup?first=1"), "on to optional 2FA");
    let c = r.cookie.unwrap();
    let r = send(&app, "GET", "/settings", Some(&c), None).await;
    assert!(r.body.contains("Signed in as <b>ada.l</b>"));
    assert!(r.body.contains("<span class=\"pill accent\">Administrator</span>"));
    assert!(r.body.contains("id=\"adm-h\""), "Administrator section shows");

    assert_eq!(send(&app, "GET", "/setup", None, None).await.location.as_deref(), Some("/login"), "setup runs once");
}

#[tokio::test]
async fn sign_up_follows_the_administrators_choice() {
    let (app, _) = with_ada(false).await;
    let admin = sign_in(&app, "ada", None).await;
    let settings = send(&app, "GET", "/settings", Some(&admin), None).await;
    let atoken = csrf(&settings.body);

    // Default: invitation only.
    assert!(send(&app, "GET", "/signup", None, None).await.body.contains("Sign-up isn’t open"));

    // An invitation link works once.
    let r = send(&app, "POST", "/settings/admin/invites", Some(&admin), Some(&format!("csrf={atoken}&note=For+kofi"))).await;
    assert!(r.body.contains("won’t be shown again"));
    let link = field(&r.body, r#"id="new-link" class="input mono grow" value=""#);
    assert!(link.starts_with("https://fluxo.test/join/"));
    let path = link.trim_start_matches("https://fluxo.test");
    let r = send(&app, "GET", path, None, None).await;
    assert!(r.body.contains("Create your account"));
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(&app, "POST", path, Some(&anon), Some(&signup_form(&token, "ada"))).await;
    assert!(r.body.contains("taken"), "existing username refused, link still usable");
    let r = send(&app, "POST", path, Some(&anon), Some(&signup_form(&token, "kofi"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings/2fa/setup?first=1"));
    assert!(send(&app, "GET", path, None, None).await.body.contains("can’t be used"), "single use");
    let r = send(&app, "GET", "/settings", Some(&admin), None).await;
    assert!(r.body.contains("Used by kofi"));
    let kofi = sign_in(&app, "kofi", None).await;
    assert!(!send(&app, "GET", "/settings", Some(&kofi), None).await.body.contains("id=\"adm-h\""), "members don’t see it");

    // Members can't change Administrator settings.
    let ktoken = csrf(&send(&app, "GET", "/settings", Some(&kofi), None).await.body);
    let r = send(&app, "POST", "/settings/admin/signup", Some(&kofi), Some(&format!("csrf={ktoken}&mode=open"))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // Revoked links stop working.
    let r = send(&app, "POST", "/settings/admin/invites", Some(&admin), Some(&format!("csrf={atoken}&note="))).await;
    let path2 = field(&r.body, r#"id="new-link" class="input mono grow" value=""#).replace("https://fluxo.test", "");
    let id = field(&r.body, r#"name="invite" value=""#);
    send(&app, "POST", "/settings/admin/invites/revoke", Some(&admin), Some(&format!("csrf={atoken}&invite={id}"))).await;
    assert!(send(&app, "GET", &path2, None, None).await.body.contains("can’t be used"));

    // Open: anyone can sign up, and the sign-in page offers it.
    send(&app, "POST", "/settings/admin/signup", Some(&admin), Some(&format!("csrf={atoken}&mode=open"))).await;
    let r = send(&app, "GET", "/login", None, None).await;
    assert!(r.body.contains("Create an account"));
    let r = send(&app, "GET", "/signup", None, None).await;
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(&app, "POST", "/signup", Some(&anon), Some(&signup_form(&token, "lin"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings/2fa/setup?first=1"));

    // Closed: no sign-up, and even fresh invitation links don't work.
    let r = send(&app, "POST", "/settings/admin/invites", Some(&admin), Some(&format!("csrf={atoken}&note="))).await;
    let path3 = field(&r.body, r#"id="new-link" class="input mono grow" value=""#).replace("https://fluxo.test", "");
    send(&app, "POST", "/settings/admin/signup", Some(&admin), Some(&format!("csrf={atoken}&mode=closed"))).await;
    assert!(send(&app, "GET", "/signup", None, None).await.body.contains("Sign-up isn’t open"));
    assert!(send(&app, "GET", &path3, None, None).await.body.contains("can’t be used"));
}

#[tokio::test]
async fn turning_on_2fa_gives_recovery_codes_that_work_once() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let r = send(&app, "GET", "/settings/2fa/setup", Some(&c), None).await;
    assert!(r.body.contains("data:image/png;base64,"), "QR code");
    assert!(r.body.contains(r#"<p class="secret">"#), "typed key for manual entry");
    let token = csrf(&r.body);
    // The pending secret is stored sealed; read it the way the server does.
    let id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'ada'").fetch_one(&app.db).await.unwrap();
    let sealed: Vec<u8> = sqlx::query_scalar("SELECT totp_pending_enc FROM users WHERE id = ?").bind(id).fetch_one(&app.db).await.unwrap();
    let secret = app.secrets.open(&format!("totp-pending:{id}"), &sealed).unwrap();
    // Reloading keeps the same pending secret.
    send(&app, "GET", "/settings/2fa/setup", Some(&c), None).await;
    let again: Vec<u8> = sqlx::query_scalar("SELECT totp_pending_enc FROM users WHERE id = ?").bind(id).fetch_one(&app.db).await.unwrap();
    assert_eq!(app.secrets.open(&format!("totp-pending:{id}"), &again).unwrap(), secret);

    let r = send(&app, "POST", "/settings/2fa/setup", Some(&c), Some(&format!("csrf={token}&code=000000&first="))).await;
    assert!(r.body.contains("didn’t match"));
    let code = totp::code_at(&secret, now() as u64);
    let r = send(&app, "POST", "/settings/2fa/setup", Some(&c), Some(&format!("csrf={token}&code={code}&first="))).await;
    assert!(r.body.contains("Save your recovery codes"));
    let codes: Vec<String> = r.body.split("<span>").skip(1).map(|s| s[..9].to_string()).collect();
    assert_eq!(codes.len(), 8);
    assert!(codes.iter().all(|c| c.len() == 9 && &c[4..5] == "-"));

    // Signing in now needs a second step; a recovery code works once.
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));
    let second = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=ada&password=a+long+enough+password")))
        .await
        .cookie
        .unwrap();
    let token = csrf(&send(&app, "GET", "/login/recovery", Some(&second), None).await.body);
    let upper = codes[0].to_uppercase();
    let r = send(&app, "POST", "/login/recovery", Some(&second), Some(&format!("csrf={token}&code={upper}"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?done=recovery-used#tfa-h"));
    let c2 = r.cookie.unwrap();
    assert!(send(&app, "GET", "/settings", Some(&c2), None).await.body.contains("7 of 8 unused"));

    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));
    let second = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=ada&password=a+long+enough+password")))
        .await
        .cookie
        .unwrap();
    let token = csrf(&send(&app, "GET", "/login/recovery", Some(&second), None).await.body);
    let r = send(&app, "POST", "/login/recovery", Some(&second), Some(&format!("csrf={token}&code={}", codes[0]))).await;
    assert!(r.body.contains("works only once"));

    // Turning 2FA off needs the password and a current code.
    let token = csrf(&send(&app, "GET", "/settings", Some(&c2), None).await.body);
    let r = send(&app, "POST", "/settings/2fa/off", Some(&c2), Some(&format!("csrf={token}&password=wrong&code=000000"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?error=2fa-off#tfa-h"));
    let fresh = totp::code_at(&secret, now() as u64 + 30);
    let r = send(&app, "POST", "/settings/2fa/off", Some(&c2), Some(&format!("csrf={token}&password=a+long+enough+password&code={fresh}"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?done=2fa-off#tfa-h"));
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM recovery_codes").fetch_one(&app.db).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn settings_save_and_validate() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let other = sign_in(&app, "ada", None).await;
    let r = send(&app, "GET", "/settings", Some(&c), None).await;
    let token = csrf(&r.body);
    assert!(r.body.contains("Sign out all other sessions"));

    // Theme is stored in the account and rendered into every page.
    send(&app, "POST", "/settings/theme", Some(&c), Some(&format!("csrf={token}&theme=dark"))).await;
    assert!(send(&app, "GET", "/", Some(&c), None).await.body.contains(r#"<html lang="en" data-theme="dark">"#));
    assert!(send(&app, "GET", "/", Some(&other), None).await.body.contains(r#"data-theme="dark""#), "follows to other devices");

    // Calendar.
    let r = send(&app, "POST", "/settings/calendar", Some(&c), Some(&format!("csrf={token}&time_zone=Europe%2FLisbon&days_off="))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?error=calendar-none#cal-h"));
    let r = send(&app, "POST", "/settings/calendar", Some(&c), Some(&format!("csrf={token}&d0=1&time_zone=Europe%2FLisbon&days_off=24%2F12"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?error=calendar-days#cal-h"));
    let r = send(&app, "POST", "/settings/calendar", Some(&c), Some(&format!("csrf={token}&d0=1&d2=1&time_zone=Europe%2FLisbon&days_off=2026-12-25%2C+2026-12-24"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?done=calendar#cal-h"));
    let (mask, days): (i64, String) = sqlx::query_as("SELECT workdays_mask, days_off FROM user_calendar").fetch_one(&app.db).await.unwrap();
    assert_eq!((mask, days.as_str()), (0b101, r#"["2026-12-24","2026-12-25"]"#));

    // Changing the password signs out other devices.
    let r = send(&app, "POST", "/settings/password", Some(&c), Some(&format!("csrf={token}&current=nope&new=another+long+password&confirm=another+long+password"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?error=password-current#pw-h"));
    let r = send(&app, "POST", "/settings/password", Some(&c), Some(&format!("csrf={token}&current=a+long+enough+password&new=another+long+password&confirm=another+long+password"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?done=password#pw-h"));
    assert_eq!(send(&app, "GET", "/", Some(&other), None).await.location.as_deref(), Some("/login"));
    assert_eq!(send(&app, "GET", "/", Some(&c), None).await.status, StatusCode::OK, "this device stays signed in");
}
