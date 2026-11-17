//! Phase 6: Administrator roles, goals and tasks without an owner, and deleting an account.

mod common;
use common::*;

use axum::http::StatusCode;

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
        Some(&format!("csrf={t}&title={}&kind=task&estimate=2&clarity=high", enc(title))),
    )
    .await;
    sqlx::query_scalar("SELECT MAX(id) FROM nodes").fetch_one(&app.db).await.unwrap()
}

async fn id_of(app: &App, name: &str) -> i64 {
    sqlx::query_scalar("SELECT id FROM users WHERE username = ?").bind(name).fetch_one(&app.db).await.unwrap()
}

async fn contributor(app: &App, node: i64, user: i64, since: i64) {
    sqlx::query("INSERT INTO memberships (node_id, user_id, role, status, invited_at, joined_at) VALUES (?, ?, 'contributor', 'active', ?, ?)")
        .bind(node).bind(user).bind(since).bind(since).execute(&app.db).await.unwrap();
}

#[tokio::test]
async fn there_is_always_an_administrator() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let kofi_id = id_of(&app, "kofi").await;
    let ada_id = id_of(&app, "ada").await;
    let t = token(&app, &ada, "/settings").await;
    let kt = token(&app, &kofi, "/settings").await;

    let r = send(&app, "POST", "/settings/admin/role", Some(&kofi), Some(&format!("csrf={kt}&user={kofi_id}&admin=1")))
        .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "members can't promote themselves");
    let r =
        send(&app, "POST", "/settings/admin/role", Some(&ada), Some(&format!("csrf={t}&user={ada_id}&admin=0"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?error=last-admin#roles-h"));
    let r =
        send(&app, "POST", "/settings/admin/role", Some(&ada), Some(&format!("csrf={t}&user={kofi_id}&admin=1"))).await;
    let to = r.location.unwrap();
    assert_eq!(to, format!("/settings?done=admin-added&user={kofi_id}#roles-h"));
    let page = send(&app, "GET", to.trim_end_matches("#roles-h"), Some(&ada), None).await.body;
    assert!(page.contains("<b>kofi</b> is now an <b>Administrator</b>."), "the notice names them");
    assert!(
        send(&app, "GET", "/settings", Some(&kofi), None).await.body.contains(r#"id="adm-h""#),
        "kofi is now an Administrator"
    );
    let r =
        send(&app, "POST", "/settings/admin/role", Some(&ada), Some(&format!("csrf={t}&user={ada_id}&admin=0"))).await;
    assert_eq!(
        r.location.as_deref(),
        Some(format!("/settings?done=admin-removed&user={ada_id}#roles-h").as_str()),
        "fine while another remains"
    );
}

#[tokio::test]
async fn deleting_an_account_hands_work_over() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    add_user(&app, "lin", false).await;
    let (kofi_id, lin_id) = (id_of(&app, "kofi").await, id_of(&app, "lin").await);
    let ada = sign_in(&app, "ada", None).await;
    let lin = sign_in(&app, "lin", None).await;
    let kofi = sign_in(&app, "kofi", None).await;

    let g = goal(&app, &ada, "Launch", None).await; // ada's; kofi contributes
    contributor(&app, g, kofi_id, 100).await;
    let t_under_g = task(&app, &ada, g, "Schema").await; // ada's task under ada's goal
    let h = goal(&app, &lin, "Lin goal", None).await; // lin's; ada contributes
    contributor(&app, h, id_of(&app, "ada").await, 50).await;
    let s = goal(&app, &ada, "Ada sub-goal", Some(h)).await; // ada's, under lin's
    let lonely = goal(&app, &ada, "Lonely", None).await; // nobody else involved

    // Only Administrator: can't leave yet.
    let r = send(&app, "GET", "/settings/delete", Some(&ada), None).await;
    assert!(r.body.contains("You’re the only Administrator"));
    let t = token(&app, &ada, "/settings").await;
    send(&app, "POST", "/settings/admin/role", Some(&ada), Some(&format!("csrf={t}&user={kofi_id}&admin=1"))).await;

    let r = send(&app, "GET", "/settings/delete", Some(&ada), None).await;
    let body = &r.body;
    let selected = |id: i64| {
        let sel = body.split(&format!(r#"name="to_{id}""#)).nth(1).unwrap();
        let sel = &sel[..sel.find("</select>").unwrap()];
        sel.split("<option ").find(|o| o.contains(" selected>")).unwrap().to_string()
    };
    assert!(selected(g).contains(&format!(r#"value="{kofi_id}""#)), "top-level: longest-standing Contributor");
    assert!(selected(t_under_g).contains(&format!(r#"value="{kofi_id}""#)), "inherited Contributor counts");
    assert!(selected(s).contains(&format!(r#"value="{lin_id}""#)), "nearest owner above");
    assert!(selected(lonely).contains(r#"value="admins""#), "no one: the Administrators");

    let t = csrf(body);
    let r = send(
        &app,
        "POST",
        "/settings/delete",
        Some(&ada),
        Some(&format!("csrf={t}&confirm=ada&password=wrong&to_{g}={kofi_id}")),
    )
    .await;
    assert!(r.body.contains("That password isn’t right"));
    // Change one suggestion: the task goes to lin instead.
    let form = format!(
        "csrf={t}&confirm=Ada&password=a+long+enough+password&to_{g}={kofi_id}&to_{t_under_g}={lin_id}&to_{s}={lin_id}&to_{lonely}=admins"
    );
    let r = send(&app, "POST", "/settings/delete", Some(&ada), Some(&form)).await;
    assert_eq!(r.location.as_deref(), Some("/login"));
    assert_eq!(send(&app, "GET", "/", Some(&ada), None).await.location.as_deref(), Some("/login"), "signed out");
    let r = send(&app, "GET", "/login", None, None).await;
    let (anon, lt) = (r.cookie.unwrap(), csrf(&r.body));
    let r = send(
        &app,
        "POST",
        "/login",
        Some(&anon),
        Some(&format!("csrf={lt}&username=ada&password=a+long+enough+password")),
    )
    .await;
    assert!(r.body.contains("don’t match"), "deleted accounts can't sign in");
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nodes").fetch_one(&app.db).await.unwrap();
    assert_eq!(n, 5, "nothing was deleted");

    // kofi accepts Launch; lin declines the task, which goes to the Administrators.
    let f = send(&app, "GET", "/", Some(&kofi), None).await.body;
    assert!(f.contains("wants you to own <b>Launch</b>"));
    let tid: i64 =
        sqlx::query_scalar("SELECT id FROM transfers WHERE node_id = ?").bind(g).fetch_one(&app.db).await.unwrap();
    let r = send(&app, "GET", &format!("/transfers/{tid}"), Some(&kofi), None).await;
    assert!(r.body.contains("ada (account deleted)"));
    assert!(r.body.contains("deleted their account and asked you to take this over"));
    send(
        &app,
        "POST",
        &format!("/transfers/{tid}"),
        Some(&kofi),
        Some(&format!("csrf={}&answer=accept", csrf(&r.body))),
    )
    .await;
    assert!(send(&app, "GET", &format!("/goals/{g}"), Some(&kofi), None).await.body.contains("Owner: you"));
    let tid: i64 = sqlx::query_scalar("SELECT id FROM transfers WHERE node_id = ?")
        .bind(t_under_g)
        .fetch_one(&app.db)
        .await
        .unwrap();
    let lt = token(&app, &lin, &format!("/transfers/{tid}")).await;
    send(&app, "POST", &format!("/transfers/{tid}"), Some(&lin), Some(&format!("csrf={lt}&answer=decline"))).await;

    // The Administrator (kofi) sees what has no owner and offers it to someone.
    let r = send(&app, "GET", "/settings", Some(&kofi), None).await;
    assert!(r.body.contains("<b>Schema</b>"));
    assert!(r.body.contains("<b>Lonely</b>"));
    let kt = csrf(&r.body);
    let r = send(
        &app,
        "POST",
        "/settings/admin/assign",
        Some(&kofi),
        Some(&format!("csrf={kt}&node={lonely}&username=lin")),
    )
    .await;
    assert!(r.location.unwrap().starts_with("/settings?done=assigned&user="));
    assert!(send(&app, "GET", "/settings", Some(&kofi), None).await.body.contains("Offered to <b>lin</b>"));
    let f = send(&app, "GET", "/", Some(&lin), None).await.body;
    assert!(f.contains("<b>An Administrator</b> wants you to own <b>Lonely</b>"));
}

#[tokio::test]
async fn administrators_see_titles_of_private_unowned_items_but_cannot_open_them() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", true).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let p = goal(&app, &ada, "Secret plan", None).await;
    let t = token(&app, &ada, &format!("/goals/{p}")).await;
    send(&app, "POST", &format!("/goals/{p}/shape"), Some(&ada), Some(&format!("csrf={t}&coverage=low&private=1")))
        .await;
    send(
        &app,
        "POST",
        "/settings/delete",
        Some(&ada),
        Some(&format!("csrf={t}&confirm=ada&password=a+long+enough+password&to_{p}=admins")),
    )
    .await;
    assert!(send(&app, "GET", "/settings", Some(&kofi), None).await.body.contains("<b>Secret plan</b>"));
    assert_eq!(send(&app, "GET", &format!("/goals/{p}"), Some(&kofi), None).await.status, StatusCode::NOT_FOUND);
}
