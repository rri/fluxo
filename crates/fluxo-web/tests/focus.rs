//! Phase 5: the Focus ranking, blocked work, snoozing, starting, and the daily slip check.

mod common;
use common::*;

async fn token(app: &App, c: &str, path: &str) -> String {
    csrf(&send(app, "GET", path, Some(c), None).await.body)
}

async fn goal(app: &App, c: &str, title: &str, coverage: &str, parent: Option<i64>) -> i64 {
    let t = token(app, c, "/goals/new").await;
    let p = parent.map(|p| p.to_string()).unwrap_or_default();
    let r = send(app, "POST", "/goals", Some(c), Some(&format!("csrf={t}&title={}&why=&parent={p}", enc(title)))).await;
    let id: i64 = r.location.unwrap().trim_start_matches("/goals/").parse().unwrap();
    send(app, "POST", &format!("/goals/{id}/shape"), Some(c), Some(&format!("csrf={t}&coverage={coverage}"))).await;
    id
}

async fn task(app: &App, c: &str, goal: i64, title: &str, days: &str, clarity: &str, needs: Option<i64>) -> i64 {
    let t = token(app, c, &format!("/goals/{goal}")).await;
    let n = needs.map(|n| n.to_string()).unwrap_or_default();
    send(app, "POST", &format!("/goals/{goal}/steps"), Some(c), Some(&format!("csrf={t}&title={}&kind=task&estimate={days}&clarity={clarity}&needs={n}", enc(title)))).await;
    sqlx::query_scalar("SELECT MAX(id) FROM nodes").fetch_one(&app.db).await.unwrap()
}

/// Titles of the ranked items on Focus, in order, with their gain text.
fn ranked(body: &str) -> Vec<(String, String)> {
    body.split(r#"<li class="focus-item">"#)
        .skip(1)
        .map(|chunk| {
            let t = chunk.split(r#"class="title" href=""#).nth(1).unwrap();
            let t = &t[t.find('>').unwrap() + 1..];
            let title = t[..t.find('<').unwrap()].to_string();
            let g = chunk.split(r#"<div class="gain">"#).nth(1).unwrap();
            let g = &g[g.find(r#"class="v"#).unwrap()..];
            let g = &g[g.find('>').unwrap() + 1..];
            (title, g[..g.find('<').unwrap()].to_string())
        })
        .collect()
}

#[tokio::test]
async fn ranks_by_expected_delay_removed() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let g = goal(&app, &c, "Launch", "full", None).await;
    let long = task(&app, &c, g, "Long and fuzzy", "10", "low", None).await;
    task(&app, &c, g, "Short and clear", "1", "high", None).await;
    let t = token(&app, &c, &format!("/tasks/{long}")).await;
    send(&app, "POST", &format!("/nodes/{long}/risks"), Some(&c), Some(&format!("csrf={t}&title=Vendor+slip&likelihood=50&min=4&max=8"))).await;

    let r = send(&app, "GET", "/", Some(&c), None).await;
    let items = ranked(&r.body);
    let titles: Vec<&str> = items.iter().map(|(t, _)| t.as_str()).collect();
    // Retiring the 50% risk of 4–8 days (~3 days) beats settling the task's own uncertainty.
    assert_eq!(titles[..2], ["Vendor slip", "Long and fuzzy"], "{items:?}");
    assert_eq!(*titles.last().unwrap(), "Short and clear", "off the critical path, little to gain");
    assert!(items.iter().all(|(_, g)| g.starts_with('−')), "top items are exact rechecks, no ~: {items:?}");
    let gain = |name: &str| -> f64 { items.iter().find(|(t, _)| t == name).unwrap().1.trim_start_matches('−').trim_end_matches('d').parse().unwrap() };
    // A 50% risk of 4–8 days on a task that's always critical removes about 3 days.
    assert!((gain("Vendor slip") - 3.0).abs() < 0.6, "{}", gain("Vendor slip"));
    assert!(gain("Long and fuzzy") > 1.0);
    assert!(r.body.contains("Long and fuzzy</a>"));
    assert!(r.body.contains("On the critical path of Launch in 100% of runs"));

    // Tabs filter, and work without JavaScript (plain links).
    let r = send(&app, "GET", "/?show=derisk", Some(&c), None).await;
    let only: Vec<String> = ranked(&r.body).into_iter().map(|(t, _)| t).collect();
    assert_eq!(only, vec!["Vendor slip"]);
}

#[tokio::test]
async fn blocked_work_starting_and_snoozing() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let c = sign_in(&app, "ada", None).await;
    let g = goal(&app, &c, "Ship", "full", None).await;
    let first = task(&app, &c, g, "Schema", "3", "medium", None).await;
    let second = task(&app, &c, g, "API", "5", "medium", Some(first)).await;
    let r = send(&app, "GET", "/", Some(&c), None).await;
    assert!(r.body.contains(&format!(r#"blocked by <a href="/tasks/{first}">Schema</a>"#)));
    assert!(r.body.contains(&format!(r#"action="/tasks/{first}/start""#)));
    assert!(!r.body.contains(&format!(r#"action="/tasks/{second}/start""#)), "blocked: can't start yet");

    let t = csrf(&r.body);
    send(&app, "POST", &format!("/tasks/{first}/start"), Some(&c), Some(&format!("csrf={t}"))).await;
    let status: String = sqlx::query_scalar("SELECT status FROM nodes WHERE id = ?").bind(first).fetch_one(&app.db).await.unwrap();
    assert_eq!(status, "doing");

    send(&app, "POST", "/focus/snooze", Some(&c), Some(&format!("csrf={t}&item=task:{second}"))).await;
    let titles: Vec<String> = ranked(&send(&app, "GET", "/", Some(&c), None).await.body).into_iter().map(|(t, _)| t).collect();
    assert!(!titles.contains(&"API".to_string()), "snoozed until tomorrow");

    // Someone who can only read sees nothing to do; a Contributor sees it all, snooze is personal.
    let kofi = sign_in(&app, "kofi", None).await;
    assert!(ranked(&send(&app, "GET", "/", Some(&kofi), None).await.body).is_empty());
    let kofi_id: i64 = sqlx::query_scalar("SELECT id FROM users WHERE username = 'kofi'").fetch_one(&app.db).await.unwrap();
    sqlx::query("INSERT INTO memberships (node_id, user_id, role, invited_at) VALUES (?, ?, 'contributor', 0)").bind(g).bind(kofi_id).execute(&app.db).await.unwrap();
    let titles: Vec<String> = ranked(&send(&app, "GET", "/", Some(&kofi), None).await.body).into_iter().map(|(t, _)| t).collect();
    assert!(titles.contains(&"API".to_string()));
}

#[tokio::test]
async fn clarify_items_for_unknown_work() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let g = goal(&app, &c, "Partial", "partial", None).await;
    task(&app, &c, g, "Known", "8", "high", None).await;
    goal(&app, &c, "Empty sub-goal", "full", Some(g)).await;
    let r = send(&app, "GET", "/?show=clarify", Some(&c), None).await;
    let items = ranked(&r.body);
    assert_eq!(items[0].0, "Partial isn’t fully covered");
    assert!(items[0].1.starts_with('−'));
    assert_eq!(items[1], ("Empty sub-goal isn’t sized or broken down".to_string(), "?".to_string()));
}

#[tokio::test]
async fn daily_check_reports_dates_slipping() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let c = sign_in(&app, "ada", None).await;
    let g = goal(&app, &c, "Slow goal", "full", None).await;
    task(&app, &c, g, "Work", "5", "high", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let kt = token(&app, &kofi, &format!("/goals/{g}")).await;
    send(&app, "POST", &format!("/nodes/{g}/people"), Some(&kofi), Some(&format!("csrf={kt}&action=watch"))).await;

    assert_eq!(fluxo_web::slips::check(&app.state).await.ok(), Some(0), "first pass sets the baseline");
    assert_eq!(fluxo_web::slips::check(&app.state).await.ok(), Some(0), "nothing slipped yet");
    // Pretend the baseline was reported two weeks ago: time passed with no progress.
    sqlx::query("UPDATE slip_marks SET p85_date = date(p85_date, '-14 days')").execute(&app.db).await.unwrap();
    assert_eq!(fluxo_web::slips::check(&app.state).await.ok(), Some(1));
    let f = send(&app, "GET", "/", Some(&kofi), None).await.body;
    assert!(f.contains("Dates for"));
    assert!(f.contains("are slipping: P85"));
    assert!(f.contains("No progress recorded as time passed"));
    assert_eq!(fluxo_web::slips::check(&app.state).await.ok(), Some(0), "reported once, then the baseline moves");
}
