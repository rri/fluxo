//! Sign-in round trip: password, then authenticator code (or a recovery code), then Focus.

mod common;
use common::*;

use axum::http::StatusCode;
use fluxo_web::auth::totp;
use fluxo_web::now;

#[tokio::test]
async fn password_then_code_then_focus() {
    let (app, secret) = with_ada(true).await;

    let r = send(&app, "GET", "/", None, None).await;
    assert_eq!(r.location.as_deref(), Some("/login"));

    let r = send(&app, "GET", "/login", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.csp, "strict CSP on pages");
    assert!(r.body.contains(r#"<form class="card card-pad" method="post" action="/login">"#), "plain HTML form");
    assert!(!r.body.contains("Create an account"), "sign-up link hidden while sign-up is invitation-only");
    let anon = r.cookie.expect("anonymous session cookie");
    let token = csrf(&r.body);

    let r = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=ada&password=nope"))).await;
    assert!(r.body.contains("don’t match"));
    assert!(r.cookie.is_none());
    let r = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=nobody&password=x"))).await;
    assert!(r.body.contains("don’t match"), "unknown users get the same answer");
    let r = send(&app, "POST", "/login", Some(&anon), Some("csrf=forged&username=ada&password=a+long+enough+password")).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    let r = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=Ada&password=a+long+enough+password"))).await;
    assert_eq!(r.location.as_deref(), Some("/login/2fa"));
    let second = r.cookie.expect("new session ID after the password step");
    assert_ne!(second, anon);
    assert_eq!(send(&app, "GET", "/", Some(&anon), None).await.location.as_deref(), Some("/login"));
    assert_eq!(send(&app, "GET", "/", Some(&second), None).await.location.as_deref(), Some("/login"));

    let token = csrf(&send(&app, "GET", "/login/2fa", Some(&second), None).await.body);
    let r = send(&app, "POST", "/login/2fa", Some(&second), Some(&format!("csrf={token}&code=000000"))).await;
    assert!(r.body.contains("didn’t work"));
    let code = totp::code_at(&secret, now() as u64);
    let r = send(&app, "POST", "/login/2fa", Some(&second), Some(&format!("csrf={token}&code={code}"))).await;
    assert_eq!(r.location.as_deref(), Some("/"));
    let signed_in = r.cookie.expect("new session ID after the code step");

    let r = send(&app, "GET", "/", Some(&signed_in), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Signed in as <b>ada</b>"));
    let token = csrf(&r.body);
    let r = send(&app, "POST", "/logout", Some(&signed_in), Some(&format!("csrf={token}"))).await;
    assert_eq!(r.location.as_deref(), Some("/login"));
    assert_eq!(send(&app, "GET", "/", Some(&signed_in), None).await.location.as_deref(), Some("/login"));

    // The same code can't be used again.
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));
    let second = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=ada&password=a+long+enough+password")))
        .await
        .cookie
        .unwrap();
    let token = csrf(&send(&app, "GET", "/login/2fa", Some(&second), None).await.body);
    let r = send(&app, "POST", "/login/2fa", Some(&second), Some(&format!("csrf={token}&code={code}"))).await;
    assert!(r.body.contains("didn’t work"), "replayed code is refused");
}

#[tokio::test]
async fn five_wrong_codes_end_the_attempt() {
    let (app, _) = with_ada(true).await;
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));
    let second = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=ada&password=a+long+enough+password")))
        .await
        .cookie
        .unwrap();
    let token = csrf(&send(&app, "GET", "/login/2fa", Some(&second), None).await.body);
    for _ in 0..4 {
        let r = send(&app, "POST", "/login/2fa", Some(&second), Some(&format!("csrf={token}&code=000000"))).await;
        assert_eq!(r.status, StatusCode::OK);
    }
    let r = send(&app, "POST", "/login/2fa", Some(&second), Some(&format!("csrf={token}&code=000000"))).await;
    assert_eq!(r.location.as_deref(), Some("/login?reason=codes"));
    assert!(send(&app, "GET", "/login?reason=codes", None, None).await.body.contains("Too many wrong codes"));
    assert_eq!(send(&app, "GET", "/login/2fa", Some(&second), None).await.location.as_deref(), Some("/login"));
}

#[tokio::test]
async fn without_2fa_password_signs_straight_in() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    assert_eq!(send(&app, "GET", "/", Some(&c), None).await.status, StatusCode::OK);
}

#[tokio::test]
async fn static_files_are_served() {
    let (app, _) = with_ada(false).await;
    let r = send(&app, "GET", "/static/styles.css", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("--accent"));
    assert_eq!(send(&app, "GET", "/static/../Cargo.toml", None, None).await.status, StatusCode::NOT_FOUND);
}
