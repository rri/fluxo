//! Phase 7: health check, sign-in throttling, export, error pages, backups.

mod common;
use common::*;

use axum::http::StatusCode;

#[tokio::test]
async fn health_check_answers_even_before_setup() {
    let app = empty().await;
    let r = send(&app, "GET", "/api/health", None, None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert_eq!(r.body, "ok");
}

#[tokio::test]
async fn unknown_pages_get_a_proper_404() {
    let (app, _) = with_ada(false).await;
    let r = send(&app, "GET", "/no/such/page", None, None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("<h1>Not found</h1>"));
    assert!(r.csp);
}

#[tokio::test]
async fn repeated_wrong_passwords_are_slowed_down() {
    let (app, _) = with_ada(false).await;
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, t) = (r.cookie.unwrap(), csrf(&r.body));
    for _ in 0..10 {
        let r = send(&app, "POST", "/login", Some(&anon), Some(&format!("csrf={t}&username=ada&password=nope"))).await;
        assert!(r.body.contains("don’t match"));
    }
    // Now even the right password is refused for a while.
    let r = send(
        &app,
        "POST",
        "/login",
        Some(&anon),
        Some(&format!("csrf={t}&username=ada&password=a+long+enough+password")),
    )
    .await;
    assert_eq!(r.status, StatusCode::TOO_MANY_REQUESTS);
    assert!(r.body.contains("Too many unsuccessful attempts"));
    // The window passes.
    sqlx::query("UPDATE sign_in_failures SET at = at - 16 * 60").execute(&app.db).await.unwrap();
    let r = send(
        &app,
        "POST",
        "/login",
        Some(&anon),
        Some(&format!("csrf={t}&username=ada&password=a+long+enough+password")),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some("/"));
}

#[tokio::test]
async fn export_holds_what_you_can_see() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let t = csrf(&send(&app, "GET", "/goals/new", Some(&ada), None).await.body);
    send(&app, "POST", "/goals", Some(&ada), Some(&format!("csrf={t}&title=Public&why=&parent="))).await;
    let r = send(&app, "POST", "/goals", Some(&ada), Some(&format!("csrf={t}&title=Secret&why=&parent="))).await;
    let secret: i64 = r.location.unwrap().trim_start_matches("/goals/").parse().unwrap();
    send(
        &app,
        "POST",
        &format!("/goals/{secret}/shape"),
        Some(&ada),
        Some(&format!("csrf={t}&coverage=low&private=1")),
    )
    .await;

    let r = send(&app, "GET", "/settings/export", Some(&ada), None).await;
    let doc: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    assert_eq!(doc["format"], "fluxo-export-1");
    let titles: Vec<&str> = doc["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Public", "Secret"]);
    assert!(doc["items"][1]["history"].as_array().unwrap().iter().any(|h| h["kind"] == "shape_changed"));
    let r = send(&app, "GET", "/settings/export", Some(&kofi), None).await;
    let doc: serde_json::Value = serde_json::from_str(&r.body).unwrap();
    let titles: Vec<&str> = doc["items"].as_array().unwrap().iter().map(|i| i["title"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Public"], "private goals stay out of other people's exports");
}

#[tokio::test]
async fn snapshots_are_complete_and_rotated() {
    // A file-backed database, as on the server (an in-memory one copies into memory).
    let dir = std::env::temp_dir().join(format!("fluxo-backup-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let opts = sqlx::sqlite::SqliteConnectOptions::new().filename(dir.join("fluxo.db")).create_if_missing(true);
    let db = sqlx::sqlite::SqlitePoolOptions::new().connect_with(opts).await.unwrap();
    fluxo_web::MIGRATOR.run(&db).await.unwrap();
    sqlx::query("INSERT INTO users (username, password_hash, created_at) VALUES ('ada', 'x', 0)")
        .execute(&db)
        .await
        .unwrap();

    let backups = dir.join("backups");
    let copy = backups.join("one.db");
    if let Err(e) = fluxo_web::backup::snapshot(&db, &copy).await {
        panic!("snapshot failed: {}", e.0)
    }
    let opened = sqlx::sqlite::SqlitePoolOptions::new().connect(&format!("sqlite://{}", copy.display())).await.unwrap();
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users").fetch_one(&opened).await.unwrap();
    assert_eq!(n, 1, "the copy has the data");
    assert!(fluxo_web::backup::snapshot(&db, &copy).await.is_err(), "never overwrites");
    for day in ["20260101", "20260102", "20260103"] {
        std::fs::write(backups.join(format!("nightly-{day}.db")), b"old").unwrap();
    }
    assert!(fluxo_web::backup::nightly(&db, &backups, 2).await.ok().unwrap().is_some());
    let mut left: Vec<String> = std::fs::read_dir(&backups)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| n.starts_with("nightly-"))
        .collect();
    left.sort();
    assert_eq!(left.len(), 2);
    assert_eq!(left[0], "nightly-20260103.db", "oldest removed first");
    assert!(fluxo_web::backup::nightly(&db, &backups, 2).await.ok().unwrap().is_none(), "one a day");
    let _ = std::fs::remove_dir_all(&dir);
}
