//! Sign-in round trip through the real router: password, then authenticator code, then Focus.
//! Uses plain HTML forms only, exactly what a browser without JavaScript sends.

use std::sync::Arc;

use axum::Router;
use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use fluxo_web::auth::{password, secrets::SecretBox, totp};
use fluxo_web::{AppState, MIGRATOR, now, router};
use http_body_util::BodyExt;
use sqlx::sqlite::SqlitePoolOptions;
use tower::ServiceExt;

struct App {
    router: Router,
    totp_secret: Vec<u8>,
}

async fn app(with_totp: bool) -> App {
    let db = SqlitePoolOptions::new().max_connections(1).connect("sqlite::memory:").await.unwrap();
    MIGRATOR.run(&db).await.unwrap();
    let secrets = SecretBox::new(&SecretBox::generate_key()).unwrap();
    let id: i64 = sqlx::query_scalar(
        "INSERT INTO users (username, password_hash, totp_enabled, created_at) VALUES ('ada', ?, ?, ?) RETURNING id",
    )
    .bind(password::hash("a long enough password"))
    .bind(with_totp as i64)
    .bind(now())
    .fetch_one(&db)
    .await
    .unwrap();
    let totp_secret = totp::generate_secret();
    sqlx::query("UPDATE users SET totp_secret_enc = ? WHERE id = ?")
        .bind(secrets.seal(&format!("totp:{id}"), &totp_secret))
        .bind(id)
        .execute(&db)
        .await
        .unwrap();
    App { router: router(Arc::new(AppState { db, secrets, secure_cookies: true })), totp_secret }
}

struct Reply {
    status: StatusCode,
    location: Option<String>,
    cookie: Option<String>,
    body: String,
    csp: bool,
}

async fn send(app: &App, method: &str, path: &str, cookie: Option<&str>, form: Option<&str>) -> Reply {
    let mut req = Request::builder().method(method).uri(path);
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, format!("fluxo_session={c}"));
    }
    let req = match form {
        Some(f) => req.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded").body(Body::from(f.to_string())),
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    let get = |h| res.headers().get(h).map(|v: &header::HeaderValue| v.to_str().unwrap().to_string());
    let location = get(header::LOCATION);
    let csp = get(header::CONTENT_SECURITY_POLICY).is_some_and(|v| v.contains("script-src 'self'"));
    let cookie = res
        .headers()
        .get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find_map(|v| v.strip_prefix("fluxo_session="))
        .map(|v| v.split(';').next().unwrap().to_string());
    let status = res.status();
    let body = String::from_utf8(res.into_body().collect().await.unwrap().to_bytes().to_vec()).unwrap();
    Reply { status, location, cookie, body, csp }
}

fn csrf(body: &str) -> String {
    let start = body.find(r#"name="csrf" value=""#).expect("page has a form token") + r#"name="csrf" value=""#.len();
    body[start..start + body[start..].find('"').unwrap()].to_string()
}

#[tokio::test]
async fn password_then_code_then_focus() {
    let app = app(true).await;

    let r = send(&app, "GET", "/", None, None).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location.as_deref(), Some("/login"));

    let r = send(&app, "GET", "/login", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.csp, "strict CSP on pages");
    assert!(r.body.contains(r#"<form class="card card-pad" method="post" action="/login">"#), "plain HTML form");
    let anon = r.cookie.expect("anonymous session cookie");
    let token = csrf(&r.body);

    // Wrong password: same page, an error, and no new session.
    let r = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=ada&password=nope"))).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("don’t match"));
    assert!(r.cookie.is_none());

    // Unknown user gets the same answer.
    let r = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=nobody&password=x"))).await;
    assert!(r.body.contains("don’t match"));

    // Missing form token is refused.
    let r = send(&app, "POST", "/login", Some(&anon), Some("csrf=forged&username=ada&password=a+long+enough+password")).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);

    // Right password (username is case-insensitive): a new session, at the code step.
    let r = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=Ada&password=a+long+enough+password"))).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location.as_deref(), Some("/login/2fa"));
    let second = r.cookie.expect("new session ID after the password step");
    assert_ne!(second, anon);

    // The old anonymous session is gone, and the half-signed-in one can't see Focus.
    assert_eq!(send(&app, "GET", "/", Some(&anon), None).await.location.as_deref(), Some("/login"));
    assert_eq!(send(&app, "GET", "/", Some(&second), None).await.location.as_deref(), Some("/login"));

    let r = send(&app, "GET", "/login/2fa", Some(&second), None).await;
    assert_eq!(r.status, StatusCode::OK);
    let token = csrf(&r.body);

    let r = send(&app, "POST", "/login/2fa", Some(&second), Some(&format!("csrf={token}&code=000000"))).await;
    assert!(r.body.contains("didn’t work"));

    let code = totp::code_at(&app.totp_secret, now() as u64);
    let r = send(&app, "POST", "/login/2fa", Some(&second), Some(&format!("csrf={token}&code={code}"))).await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(r.location.as_deref(), Some("/"));
    let signed_in = r.cookie.expect("new session ID after the code step");

    let r = send(&app, "GET", "/", Some(&signed_in), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Signed in as <b>ada</b>"));
    let token = csrf(&r.body);

    // Sign out ends the session.
    let r = send(&app, "POST", "/logout", Some(&signed_in), Some(&format!("csrf={token}"))).await;
    assert_eq!(r.location.as_deref(), Some("/login"));
    assert_eq!(send(&app, "GET", "/", Some(&signed_in), None).await.location.as_deref(), Some("/login"));

    // The same code can't be used again by a second sign-in.
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=ada&password=a+long+enough+password"))).await;
    let second = r.cookie.unwrap();
    let token = csrf(&send(&app, "GET", "/login/2fa", Some(&second), None).await.body);
    let r = send(&app, "POST", "/login/2fa", Some(&second), Some(&format!("csrf={token}&code={code}"))).await;
    assert!(r.body.contains("didn’t work"), "replayed code is refused");
}

#[tokio::test]
async fn five_wrong_codes_end_the_attempt() {
    let app = app(true).await;
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=ada&password=a+long+enough+password"))).await;
    let second = r.cookie.unwrap();
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
    let app = app(false).await;
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, token) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={token}&username=ada&password=a+long+enough+password"))).await;
    assert_eq!(r.location.as_deref(), Some("/"));
    assert_eq!(send(&app, "GET", "/", r.cookie.as_deref(), None).await.status, StatusCode::OK);
}

#[tokio::test]
async fn static_files_are_served() {
    let app = app(false).await;
    let r = send(&app, "GET", "/static/styles.css", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("--accent"));
    assert_eq!(send(&app, "GET", "/static/../Cargo.toml", None, None).await.status, StatusCode::NOT_FOUND);
}
