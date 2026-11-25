//! Installing fluxo on a home screen (G19) and reading it offline (G20).

mod common;
use common::*;

use axum::body::Body;
use axum::http::{HeaderMap, Request, StatusCode, header};
use http_body_util::BodyExt;
use tower::ServiceExt;

async fn token(app: &App, c: &str, path: &str) -> String {
    csrf(&send(app, "GET", path, Some(c), None).await.body)
}

async fn goal(app: &App, c: &str, title: &str, parent: Option<i64>) -> i64 {
    let t = token(app, c, "/goals/new").await;
    let p = parent.map(|p| p.to_string()).unwrap_or_default();
    let r = send(app, "POST", "/goals", Some(c), Some(&format!("csrf={t}&title={}&why=&parent={p}", enc(title)))).await;
    r.location.unwrap().trim_start_matches("/goals/").parse().unwrap()
}

async fn task(app: &App, c: &str, goal: i64, title: &str) -> i64 {
    let t = token(app, c, &format!("/goals/{goal}")).await;
    send(
        app,
        "POST",
        &format!("/goals/{goal}/steps"),
        Some(c),
        Some(&format!("csrf={t}&title={}&kind=task&estimate=2&clarity=low", enc(title))),
    )
    .await;
    sqlx::query_scalar("SELECT MAX(id) FROM nodes").fetch_one(&app.db).await.unwrap()
}

/// A request with its raw response headers and body bytes.
async fn raw(
    app: &App,
    method: &str,
    path: &str,
    cookie: Option<&str>,
    form: Option<&str>,
) -> (StatusCode, HeaderMap, Vec<u8>) {
    let mut req = Request::builder().method(method).uri(path).header(header::HOST, "fluxo.test");
    if let Some(c) = cookie {
        req = req.header(header::COOKIE, format!("fluxo_session={c}"));
    }
    let req = match form {
        Some(f) => {
            req.header(header::CONTENT_TYPE, "application/x-www-form-urlencoded").body(Body::from(f.to_string()))
        }
        None => req.body(Body::empty()),
    }
    .unwrap();
    let res = app.router.clone().oneshot(req).await.unwrap();
    let (status, headers) = (res.status(), res.headers().clone());
    (status, headers, res.into_body().collect().await.unwrap().to_bytes().to_vec())
}

fn session_cookie(h: &HeaderMap) -> Option<String> {
    h.get_all(header::SET_COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .find(|v| v.starts_with("fluxo_session="))
        .map(String::from)
}

#[tokio::test]
async fn g19_pages_can_be_installed() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    for page in [send(&app, "GET", "/", Some(&c), None).await.body, send(&app, "GET", "/login", None, None).await.body]
    {
        assert!(page.contains(r#"<link rel="manifest" href="/static/manifest.webmanifest">"#));
        assert!(page.contains(r#"<link rel="apple-touch-icon" href="/static/icon-180.png">"#));
        assert!(page.contains(r#"<meta name="apple-mobile-web-app-title" content="fluxo">"#));
        assert!(page.contains("viewport-fit=cover"));
        assert!(
            page.contains(r##"<meta name="theme-color" content="#fafafa" media="(prefers-color-scheme: light)">"##)
        );
    }
    // A chosen theme gives the bar that theme's colour.
    let t = token(&app, &c, "/settings").await;
    send(&app, "POST", "/settings/theme", Some(&c), Some(&format!("csrf={t}&theme=dark"))).await;
    assert!(
        send(&app, "GET", "/", Some(&c), None).await.body.contains(r##"<meta name="theme-color" content="#0e0e11">"##)
    );

    let (status, h, body) = raw(&app, "GET", "/static/manifest.webmanifest", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h[header::CONTENT_TYPE], "application/manifest+json");
    let m: serde_json::Value = serde_json::from_slice(&body).unwrap();
    assert_eq!(
        (m["name"].as_str(), m["start_url"].as_str(), m["display"].as_str()),
        (Some("fluxo"), Some("/"), Some("standalone"))
    );
    for icon in m["icons"].as_array().unwrap() {
        let (status, h, body) = raw(&app, "GET", icon["src"].as_str().unwrap(), None, None).await;
        assert_eq!(
            (status, &h[header::CONTENT_TYPE]),
            (StatusCode::OK, &"image/png".parse::<header::HeaderValue>().unwrap())
        );
        assert!(body.starts_with(b"\x89PNG"));
    }
    assert!(raw(&app, "GET", "/static/icon-180.png", None, None).await.2.starts_with(b"\x89PNG"));
}

#[tokio::test]
async fn g7_only_the_named_static_files_are_served() {
    let (app, _) = with_ada(false).await;
    for f in [
        "styles.css",
        "app.js",
        "theme.js",
        "favicon.svg",
        "logo.svg",
        "manifest.webmanifest",
        "icon-180.png",
        "icon-192.png",
        "icon-512.png",
        "icon-maskable-512.png",
    ] {
        assert_eq!(raw(&app, "GET", &format!("/static/{f}"), None, None).await.0, StatusCode::OK, "{f}");
    }
    for f in ["sw.js", "mark.html", "../Cargo.toml", "icon.png"] {
        assert_eq!(raw(&app, "GET", &format!("/static/{f}"), None, None).await.0, StatusCode::NOT_FOUND, "{f}");
    }
}

#[tokio::test]
async fn g19_the_session_cookie_lasts_30_days_and_renews_while_used() {
    let (app, _) = with_ada(false).await;
    // Signing in sets a cookie that outlives the browser session.
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, t) = (r.cookie.unwrap(), csrf(&r.body));
    let form = format!("csrf={t}&username=ada&password=a+long+enough+password");
    let (_, h, _) = raw(&app, "POST", "/login", Some(&anon), Some(&form)).await;
    let set = session_cookie(&h).expect("signed in");
    assert!(
        set.contains("Max-Age=2592000")
            && set.contains("HttpOnly")
            && set.contains("Secure")
            && set.contains("SameSite=Lax"),
        "{set}"
    );
    let c = set.trim_start_matches("fluxo_session=").split(';').next().unwrap().to_string();

    // Every page that loads renews it, unchanged.
    let (status, h, _) = raw(&app, "GET", "/goals", Some(&c), None).await;
    assert_eq!(status, StatusCode::OK);
    let renewed = session_cookie(&h).expect("renewed");
    assert!(renewed.starts_with(&format!("fluxo_session={c};")) && renewed.contains("Max-Age=2592000"), "{renewed}");

    // Not for static files, posts, or values fluxo didn't issue.
    assert!(session_cookie(&raw(&app, "GET", "/static/styles.css", Some(&c), None).await.1).is_none());
    assert!(
        session_cookie(&raw(&app, "GET", "/login", Some("not-ours;x"), None).await.1)
            .is_none_or(|v| !v.contains("not-ours"))
    );

    // Signing out removes it, and a dead session's cookie isn't renewed.
    let t = token(&app, &c, "/settings").await;
    let (_, h, _) = raw(&app, "POST", "/logout", Some(&c), Some(&format!("csrf={t}"))).await;
    let gone = session_cookie(&h).expect("removed");
    assert!(gone.starts_with("fluxo_session=;") && !gone.contains(&c), "{gone}");
    let (status, h, _) = raw(&app, "GET", "/goals", Some(&c), None).await;
    assert!(status.is_redirection());
    assert!(session_cookie(&h).is_none());
}

#[tokio::test]
async fn g20_the_offline_list_is_the_persons_own_readable_pages() {
    let (app, _) = with_ada(false).await;
    let bob_id = add_user(&app, "bob", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let bob = sign_in(&app, "bob", None).await;
    let mine = goal(&app, &bob, "Bob's goal", None).await;
    let my_task = task(&app, &bob, mine, "Bob's task").await;
    let watched = goal(&app, &ada, "Watched goal", None).await;
    let below = task(&app, &ada, watched, "Beneath the watched goal").await;
    let elsewhere = goal(&app, &ada, "Someone else's public goal", None).await;
    let archived = task(&app, &bob, mine, "Archived task").await;
    sqlx::query("UPDATE nodes SET archived_at = 1, archived_by = owner_id WHERE id = ?")
        .bind(archived)
        .execute(&app.db)
        .await
        .unwrap();
    sqlx::query("INSERT INTO memberships (node_id, user_id, role, status, invited_at, joined_at) VALUES (?, ?, 'watcher', 'active', 1, 1)")
        .bind(watched)
        .bind(bob_id)
        .execute(&app.db)
        .await
        .unwrap();

    let (status, h, body) = raw(&app, "GET", "/offline/pages", Some(&bob), None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h[header::CONTENT_TYPE], "application/json");
    assert_eq!(h[header::CACHE_CONTROL], "no-store");
    let list: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let got: Vec<&str> = list["pages"].as_array().unwrap().iter().map(|p| p.as_str().unwrap()).collect();
    let want = [
        "/".to_string(),
        "/goals".to_string(),
        format!("/goals/{mine}"),
        format!("/goals/{watched}"),
        format!("/tasks/{my_task}"),
        format!("/tasks/{below}"),
    ];
    assert_eq!(got, want.iter().map(String::as_str).collect::<Vec<_>>());
    assert!(!got.contains(&format!("/goals/{elsewhere}").as_str()));

    // Signed out: sent to sign in, like any page.
    let r = send(&app, "GET", "/offline/pages", None, None).await;
    assert_eq!(r.location.as_deref(), Some("/login"));
}

#[tokio::test]
async fn g20_the_service_worker_and_offline_page_are_served_before_setup_too() {
    let app = empty().await;
    let (status, h, body) = raw(&app, "GET", "/sw.js", None, None).await;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(h[header::CONTENT_TYPE], "text/javascript; charset=utf-8");
    assert_eq!(h[header::CACHE_CONTROL], "no-store", "browsers always check for a new worker");
    let js = String::from_utf8(body).unwrap();
    let build = js.split("const BUILD = '").nth(1).unwrap().split('\'').next().unwrap();
    assert!(build.len() == 12 && build.bytes().all(|b| b.is_ascii_hexdigit()), "{build}");
    // Only Focus, Goals, goal pages and task pages are ever kept.
    assert!(js.contains(r"const KEEPABLE = /^\/(?:goals(?:\/\d+)?|tasks\/\d+)?$/;"));

    let r = send(&app, "GET", "/offline", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Not saved on this device") && r.body.contains("github.com/rri/fluxo"));
}
