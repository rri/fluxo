//! Phase 3: forecasts on goal, Goals and task pages.

mod common;
use common::*;

async fn token(app: &App, c: &str, path: &str) -> String {
    csrf(&send(app, "GET", path, Some(c), None).await.body)
}

async fn goal(app: &App, c: &str, title: &str, coverage: &str) -> i64 {
    let t = token(app, c, "/goals/new").await;
    let r = send(app, "POST", "/goals", Some(c), Some(&format!("csrf={t}&title={}&why=&parent=", enc(title)))).await;
    let id: i64 = r.location.unwrap().trim_start_matches("/goals/").parse().unwrap();
    send(app, "POST", &format!("/goals/{id}/shape"), Some(c), Some(&format!("csrf={t}&coverage={coverage}"))).await;
    id
}

async fn task(app: &App, c: &str, goal: i64, title: &str, days: &str, needs: Option<i64>) -> i64 {
    let t = token(app, c, &format!("/goals/{goal}")).await;
    let n = needs.map(|n| n.to_string()).unwrap_or_default();
    send(app, "POST", &format!("/goals/{goal}/steps"), Some(c), Some(&format!("csrf={t}&title={}&kind=task&estimate={days}&clarity=high&needs={n}", enc(title)))).await;
    sqlx::query_scalar("SELECT MAX(id) FROM nodes").fetch_one(&app.db).await.unwrap()
}

/// The value shown under a forecast label, e.g. `band(&body, "P50 finish")`.
fn band(body: &str, label: &str) -> String {
    let marker = format!(r#"<div class="k">{label}</div><div class="v"#);
    let i = body.find(&marker).unwrap_or_else(|| panic!("no {label}"));
    let rest = &body[i + marker.len()..];
    let rest = &rest[rest.find('>').unwrap() + 1..];
    rest[..rest.find('<').unwrap()].to_string()
}

#[tokio::test]
async fn goal_page_shows_dates_and_the_critical_path() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let g = goal(&app, &c, "Ship", "full").await;
    let long = task(&app, &c, g, "Long", "10", None).await;
    let short = task(&app, &c, g, "Short", "1", None).await;
    let after = task(&app, &c, g, "After long", "2", Some(long)).await;

    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await;
    let (p50, p85) = (band(&r.body, "P50 finish"), band(&r.body, "P85 finish"));
    assert!(!p50.is_empty() && !p85.is_empty());
    assert!(r.body.contains("10,000 runs"));
    assert!(r.body.contains(r#"<svg class="dist-svg""#), "distribution drawn on the server");
    // Long → After long is always the longest chain; Short never is.
    assert!(r.body.contains(&format!(r#"<li data-id="{long}" data-path="1">"#)));
    assert!(r.body.contains(&format!(r#"<li data-id="{after}" data-deps="{long}" data-path="1">"#)));
    assert!(r.body.contains(&format!(r#"<li data-id="{short}">"#)), "not on the path");
    assert!(r.body.contains("on-path c10"));
    assert!(r.body.contains("high · 100%") || r.body.contains("<span>100%</span>"));

    // The Goals page summarizes it.
    let r = send(&app, "GET", "/goals", Some(&c), None).await;
    assert!(r.body.contains(&format!("P50 {p50} · P85 {p85}")));
    assert!(r.body.contains("Critical now <b>Long</b>"), "of the critical steps, the one that can start now");

    // The task page shows its criticality and finish dates.
    let r = send(&app, "GET", &format!("/tasks/{long}"), Some(&c), None).await;
    assert!(r.body.contains(r#"<div class="v accent-text">100%</div>"#));
    assert!(r.body.contains("P50 done"));

    // A certain 20-day risk on the long task moves the finish: the forecast is recalculated.
    let t = token(&app, &c, &format!("/tasks/{long}")).await;
    send(&app, "POST", &format!("/nodes/{long}/risks"), Some(&c), Some(&format!("csrf={t}&title=Vendor+slip&likelihood=100&min=20&max=20"))).await;
    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await;
    assert_ne!(band(&r.body, "P50 finish"), p50, "changed inputs, new forecast");
    assert_eq!(band(&r.body, "Risk exposure").trim(), "20d");
}

#[tokio::test]
async fn coverage_shapes_the_forecast() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let low = goal(&app, &c, "Fuzzy", "low").await;
    task(&app, &c, low, "Something", "5", None).await;
    let r = send(&app, "GET", &format!("/goals/{low}"), Some(&c), None).await;
    assert_eq!(band(&r.body, "Finish"), "nebulous");
    assert!(r.body.contains("can’t be dated"));
    assert!(send(&app, "GET", "/goals", Some(&c), None).await.body.contains("<b>nebulous</b>"));

    let partial = goal(&app, &c, "Mostly known", "partial").await;
    task(&app, &c, partial, "Known part", "8", None).await;
    let r = send(&app, "GET", &format!("/goals/{partial}"), Some(&c), None).await;
    assert!(r.body.contains(r#"<li data-id="gap" data-path="1">"#), "unknown work is drawn");
    assert!(r.body.contains("Not broken down yet"));
    assert!(r.body.contains(" gap\" data-path=\"1\" data-wide=\"1\""), "and feeds the goal");

    // No steps yet: no forecast to show.
    let empty = goal(&app, &c, "Empty", "full").await;
    let r = send(&app, "GET", &format!("/goals/{empty}"), Some(&c), None).await;
    assert!(r.body.contains("No steps yet"));
}

#[tokio::test]
async fn achieved_goals_leave_the_forecast() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let g = goal(&app, &c, "Done thing", "full").await;
    task(&app, &c, g, "Work", "3", None).await;
    let t = token(&app, &c, &format!("/goals/{g}")).await;
    assert!(send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await.body.contains("P50 finish"));
    send(&app, "POST", &format!("/goals/{g}/achieved"), Some(&c), Some(&format!("csrf={t}&achieved=1"))).await;
    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await;
    assert!(!r.body.contains("P50 finish"));
    assert!(r.body.contains("pill accent\">Achieved"));
}

#[tokio::test]
async fn time_zones_are_checked_for_real() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let t = token(&app, &c, "/settings").await;
    let r = send(&app, "POST", "/settings/calendar", Some(&c), Some(&format!("csrf={t}&d0=1&time_zone=Mars%2FPhobos&days_off="))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?error=calendar-tz#cal-h"));
    let r = send(&app, "POST", "/settings/calendar", Some(&c), Some(&format!("csrf={t}&d0=1&time_zone=Asia%2FKolkata&days_off="))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?done=calendar#cal-h"));
}

#[tokio::test]
async fn empty_sub_goals_are_flagged_until_sized_or_broken_down() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let g = goal(&app, &c, "Launch", "full").await;
    task(&app, &c, g, "Build", "5", None).await;
    let t = token(&app, &c, &format!("/goals/{g}")).await;
    send(&app, "POST", &format!("/goals/{g}/steps"), Some(&c), Some(&format!("csrf={t}&title=Accounts&kind=goal&coverage=full"))).await;
    let sub: i64 = sqlx::query_scalar("SELECT id FROM nodes WHERE title = 'Accounts'").fetch_one(&app.db).await.unwrap();

    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await;
    assert!(r.body.contains("1 sub-goal isn’t sized or broken down yet, so these dates leave it out"));
    assert!(r.body.contains(&format!(r#"<a href="/goals/{sub}">Accounts</a>"#)));
    assert!(r.body.contains("not sized yet"));
    let before = band(&r.body, "P50 finish");
    let r = send(&app, "GET", &format!("/goals/{sub}"), Some(&c), None).await;
    assert!(!r.body.contains("P50 finish"), "nothing to forecast yet");
    assert!(r.body.contains("Set size"));

    // A bad size is explained; a good one counts and clears the flag.
    let r = send(&app, "POST", &format!("/goals/{sub}/size"), Some(&c), Some(&format!("csrf={t}&min=15&max=5"))).await;
    assert_eq!(r.location.as_deref(), Some(format!("/goals/{sub}?error=size#path").as_str()));
    send(&app, "POST", &format!("/goals/{sub}/size"), Some(&c), Some(&format!("csrf={t}&min=20&max=30"))).await;
    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await;
    assert!(!r.body.contains("isn’t sized"));
    assert!(r.body.contains("about 20–30d"));
    assert_ne!(band(&r.body, "P50 finish"), before, "the size now counts");
    let r = send(&app, "GET", &format!("/goals/{sub}"), Some(&c), None).await;
    assert!(r.body.contains("P50 finish"), "sized goals have their own forecast");
    assert!(r.body.contains("rough size of about 20–30 working days"));

    // A rough size can also be given when the sub-goal is added.
    send(&app, "POST", &format!("/goals/{g}/steps"), Some(&c), Some(&format!("csrf={t}&title=Docs&kind=goal&coverage=full&size_min=2&size_max=4"))).await;
    assert!(send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await.body.contains("about 2–4d"));
}
