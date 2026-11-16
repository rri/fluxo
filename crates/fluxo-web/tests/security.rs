//! Checks for the attacks in `docs/threat-model.md`.

mod common;
use common::*;

use axum::http::StatusCode;

/// Start a sign-in with the right password; returns the half-signed-in cookie and its form token.
async fn half(app: &App) -> (String, String) {
    let r = send(app, "GET", "/login", None, None).await;
    let (anon, t) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(
        app,
        "POST",
        "/login",
        Some(&anon),
        Some(&format!("csrf={t}&username=ada&password=a+long+enough+password")),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some("/login/2fa"));
    let c = r.cookie.unwrap();
    let t = csrf(&send(app, "GET", "/login/2fa", Some(&c), None).await.body);
    (c, t)
}

fn wrong_code(secret: &[u8]) -> String {
    let right = fluxo_web::auth::totp::code_at(secret, fluxo_web::now() as u64);
    // Not the current code, nor either neighbour.
    (0..1_000_000)
        .map(|n| format!("{n:06}"))
        .find(|c| {
            let t = fluxo_web::now() as u64;
            *c != right
                && *c != fluxo_web::auth::totp::code_at(secret, t - 30)
                && *c != fluxo_web::auth::totp::code_at(secret, t + 30)
        })
        .unwrap()
}

#[tokio::test]
async fn v4_guessing_codes_across_fresh_sign_ins_is_limited() {
    let (app, secret) = with_ada(true).await;
    let bad = wrong_code(&secret);
    // Two sign-ins, five wrong codes each: each attempt ends, and the account's limit is reached.
    for _ in 0..2 {
        let (c, t) = half(&app).await;
        for _ in 0..5 {
            send(&app, "POST", "/login/2fa", Some(&c), Some(&format!("csrf={t}&code={bad}"))).await;
        }
    }
    // Now even the right password is refused for a while, so there are no fresh tries.
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, t) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(
        &app,
        "POST",
        "/login",
        Some(&anon),
        Some(&format!("csrf={t}&username=ada&password=a+long+enough+password")),
    )
    .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
}

#[tokio::test]
async fn v4_a_half_signed_in_attempt_stops_once_the_limit_is_reached() {
    let (app, secret) = with_ada(true).await;
    let (c, t) = half(&app).await;
    // Failures from elsewhere (say, password guesses) fill the account's limit.
    for _ in 0..10 {
        sqlx::query("INSERT INTO sign_in_failures (username, ip, at) VALUES ('ada', 'elsewhere', ?)")
            .bind(fluxo_web::now())
            .execute(&app.db)
            .await
            .unwrap();
    }
    let code = fluxo_web::auth::totp::code_at(&secret, fluxo_web::now() as u64);
    let r = send(&app, "POST", "/login/2fa", Some(&c), Some(&format!("csrf={t}&code={code}"))).await;
    assert_eq!(r.location.as_deref(), Some("/login?reason=throttled"), "even the right code waits");
    let r = send(&app, "GET", "/login?reason=throttled", None, None).await;
    assert!(r.body.contains("Too many unsuccessful attempts"));
}

#[tokio::test]
async fn v4_recovery_code_guesses_count_too() {
    let (app, _) = with_ada(true).await;
    for _ in 0..2 {
        let (c, _) = half(&app).await;
        let t = csrf(&send(&app, "GET", "/login/recovery", Some(&c), None).await.body);
        for _ in 0..5 {
            send(&app, "POST", "/login/recovery", Some(&c), Some(&format!("csrf={t}&code=aaaa-bbbb"))).await;
        }
    }
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sign_in_failures WHERE username = 'ada'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 10);
}

#[tokio::test]
async fn v4_a_stolen_session_cannot_guess_the_password_in_settings() {
    let (app, _) = with_ada(false).await;
    let ada = sign_in(&app, "ada", None).await;
    let t = csrf(&send(&app, "GET", "/settings", Some(&ada), None).await.body);
    for i in 0..10 {
        let r = send(
            &app,
            "POST",
            "/settings/password",
            Some(&ada),
            Some(&format!("csrf={t}&current=guess{i}&new=another+long+password&confirm=another+long+password")),
        )
        .await;
        assert_eq!(r.location.as_deref(), Some("/settings?error=password-current#pw-h"));
    }
    let r = send(
        &app,
        "POST",
        "/settings/password",
        Some(&ada),
        Some(&format!(
            "csrf={t}&current=a+long+enough+password&new=another+long+password&confirm=another+long+password"
        )),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some("/settings?error=throttled-pw#pw-h"), "the right one waits too");
    let page = send(&app, "GET", "/settings?error=throttled-pw", Some(&ada), None).await.body;
    assert!(page.contains("Too many unsuccessful attempts"));
    // Deleting the account is refused the same way.
    let r = send(
        &app,
        "POST",
        "/settings/delete",
        Some(&ada),
        Some(&format!("csrf={t}&confirm=ada&password=a+long+enough+password")),
    )
    .await;
    assert!(r.body.contains("You’re the only Administrator") || r.body.contains("Too many unsuccessful attempts"));
}

#[tokio::test]
async fn v6_session_ids_change_at_sign_in_and_old_ones_die() {
    let (app, _) = with_ada(false).await;
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, t) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(
        &app,
        "POST",
        "/login",
        Some(&anon),
        Some(&format!("csrf={t}&username=ada&password=a+long+enough+password")),
    )
    .await;
    let signed = r.cookie.unwrap();
    assert_ne!(anon, signed);
    assert_eq!(send(&app, "GET", "/", Some(&anon), None).await.location.as_deref(), Some("/login"));
    assert_eq!(send(&app, "GET", "/", Some(&signed), None).await.status, StatusCode::OK);
    // Only a hash of the cookie is stored.
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE CAST(id_hash AS TEXT) = ?")
        .bind(&signed)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn se2_cookie_flags() {
    use axum::body::Body;
    use axum::http::{Request, header};
    use tower::ServiceExt;
    let (app, _) = with_ada(false).await;
    let req = Request::builder().uri("/login").header(header::HOST, "fluxo.test").body(Body::empty()).unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    let c = res.headers()[header::SET_COOKIE].to_str().unwrap().to_string();
    for flag in ["HttpOnly", "Secure", "SameSite=Lax", "Path=/"] {
        assert!(c.contains(flag), "{c} lacks {flag}");
    }
}

#[tokio::test]
async fn g10_user_text_is_escaped_everywhere_it_shows() {
    let (app, _) = with_ada(false).await;
    let ada = sign_in(&app, "ada", None).await;
    let evil = r#"<script>alert(1)</script>"><img src=x onerror=alert(2)>"#;
    let t = csrf(&send(&app, "GET", "/goals/new", Some(&ada), None).await.body);
    let r = send(
        &app,
        "POST",
        "/goals",
        Some(&ada),
        Some(&format!("csrf={t}&title={}&why={}&parent=", enc(evil), enc(evil))),
    )
    .await;
    let g = r.location.unwrap();
    let id: i64 = g.trim_start_matches("/goals/").parse().unwrap();
    send(
        &app,
        "POST",
        &format!("/goals/{id}/steps"),
        Some(&ada),
        Some(&format!("csrf={t}&title={}&kind=task&estimate=1&clarity=low", enc(evil))),
    )
    .await;
    send(
        &app,
        "POST",
        &format!("/nodes/{id}/risks"),
        Some(&ada),
        Some(&format!("csrf={t}&title={}&likelihood=50&min=1&max=2", enc(evil))),
    )
    .await;
    send(&app, "POST", "/settings/profile", Some(&ada), Some(&format!("csrf={t}&display_name={}", enc(evil)))).await;
    send(&app, "POST", "/settings/admin/invites", Some(&ada), Some(&format!("csrf={t}&note={}", enc(evil)))).await;
    let task: i64 = sqlx::query_scalar("SELECT MAX(id) FROM nodes").fetch_one(&app.db).await.unwrap();
    for path in
        ["/".to_string(), "/goals".into(), g.clone(), format!("/tasks/{task}"), "/settings".into(), "/goals/new".into()]
    {
        let body = send(&app, "GET", &path, Some(&ada), None).await.body;
        assert!(!body.contains("<script>alert") && !body.contains("<img src=x"), "{path} has raw markup");
    }
}

#[tokio::test]
async fn changing_the_case_of_a_username_does_not_dodge_the_limit() {
    // The username a failure is recorded under is normalised, so case changes don't dodge the limit.
    let (app, _) = with_ada(false).await;
    for name in ["ADA", "Ada", " ada ", "aDa", "ada", "ADA", "Ada", "ada", "aDA", "AdA"] {
        let r = send(&app, "GET", "/login", None, None).await;
        let (anon, t) = (r.cookie.unwrap(), csrf(&r.body));
        send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={t}&username={}&password=wrong", enc(name))))
            .await;
    }
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, t) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(
        &app,
        "POST",
        "/login",
        Some(&anon),
        Some(&format!("csrf={t}&username=ada&password=a+long+enough+password")),
    )
    .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
}
