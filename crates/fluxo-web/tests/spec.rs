//! Checks of `docs/spec.md`: page states, access, and what each kind of visitor sees. Each test
//! names the rules it covers.

mod common;
use common::*;

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
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

async fn make_private(app: &App, c: &str, g: i64) {
    let t = token(app, c, &format!("/goals/{g}")).await;
    send(app, "POST", &format!("/goals/{g}/shape"), Some(c), Some(&format!("csrf={t}&coverage=low&private=1"))).await;
}

async fn role(app: &App, node: i64, user: i64, role: &str) {
    sqlx::query("INSERT INTO memberships (node_id, user_id, role, status, invited_at, joined_at) VALUES (?, ?, ?, 'active', 1, 1)")
        .bind(node)
        .bind(user)
        .bind(role)
        .execute(&app.db)
        .await
        .unwrap();
}

/// Everything an action could change, to check that a refused one changed nothing.
async fn snapshot(app: &App) -> String {
    let mut out = String::new();
    for q in [
        "SELECT group_concat(id || title || why || coalesce(owner_id,'') || coalesce(home_parent_id,'') || status || private || coalesce(archived_at,'') || coverage, ';') FROM nodes",
        "SELECT group_concat(child_id || '>' || parent_id, ';') FROM node_parents",
        "SELECT group_concat(node_id || '>' || needs_id, ';') FROM dependencies",
        "SELECT group_concat(id || username || is_administrator || coalesce(deleted_at,'') || display_name || theme || password_hash, ';') FROM users",
        "SELECT group_concat(node_id || user_id || role || status, ';') FROM memberships",
        "SELECT group_concat(id || status, ';') FROM transfers",
        "SELECT group_concat(id || title, ';') FROM risks",
        "SELECT group_concat(signup_mode, ';') FROM instance",
        "SELECT COUNT(*) FROM signup_invites",
        "SELECT COUNT(*) FROM admin_codes",
        "SELECT COUNT(*) FROM focus_snoozes",
        "SELECT COUNT(*) FROM events",
    ] {
        let v: Option<String> =
            sqlx::query_scalar(&format!("SELECT CAST(({q}) AS TEXT)")).fetch_one(&app.db).await.unwrap();
        out.push_str(&v.unwrap_or_default());
        out.push('\n');
    }
    out
}

/// Every POST route, with a form that has every field any of them needs.
fn post_routes(g: i64, t: i64, r: i64) -> Vec<String> {
    let mut v: Vec<String> = [
        "/updates/read",
        "/focus/snooze",
        "/goals",
        "/logout",
        "/settings/profile",
        "/settings/password",
        "/settings/theme",
        "/settings/calendar",
        "/settings/calendar/days-off",
        "/settings/sessions/end",
        "/settings/sessions/end-others",
        "/settings/2fa/setup",
        "/settings/2fa/codes",
        "/settings/2fa/off",
        "/settings/admin/signup",
        "/settings/admin/retention",
        "/settings/admin/invites",
        "/settings/admin/invites/revoke",
        "/settings/admin/recovery",
        "/settings/admin/recovery/revoke",
        "/settings/admin/role",
        "/settings/admin/assign",
        "/settings/delete",
        "/reset-password",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    for p in ["steps", "statement", "shape", "achieved", "size"] {
        v.push(format!("/goals/{g}/{p}"));
    }
    for p in ["parents", "needs", "risks", "archive", "restore", "people", "transfer"] {
        v.push(format!("/nodes/{t}/{p}"));
    }
    v.push(format!("/tasks/{t}"));
    v.push(format!("/tasks/{t}/start"));
    v.push(format!("/risks/{r}"));
    v.push(format!("/invitations/{g}"));
    v.push("/transfers/1".into());
    v
}

const EVERY_FIELD: &str = "title=x&why=x&what=x&kind=task&estimate=1&clarity=high&coverage=full&private=1&action=remove\
&goal=1&other=1&likelihood=50&min=1&max=2&username=kofi&role=watcher&user=2&note=x&answer=accept&display_name=x\
&current=a+long+enough+password&new=another+long+password&confirm=ada&theme=dark&time_zone=UTC&days_off=&d0=1\
&session=x&code=123456&password=a+long+enough+password&mode=open&invite=1&admin=0&node=1&status=done&upto=999\
&item=task:1&achieved=1&parent=&remaining=1&needs=&size_min=&size_max=";

#[tokio::test]
async fn g3_every_action_needs_the_form_token() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let g = goal(&app, &ada, "Launch", None).await;
    let t = task(&app, &ada, g, "Schema").await;
    let tk = token(&app, &ada, &format!("/goals/{g}")).await;
    send(
        &app,
        "POST",
        &format!("/nodes/{t}/risks"),
        Some(&ada),
        Some(&format!("csrf={tk}&title=Vendor&likelihood=40&min=1&max=3")),
    )
    .await;
    let r: i64 = sqlx::query_scalar("SELECT MAX(id) FROM risks").fetch_one(&app.db).await.unwrap();
    let before = snapshot(&app).await;
    for path in post_routes(g, t, r) {
        for bad in ["csrf=wrong&", ""] {
            let res = send(&app, "POST", &path, Some(&ada), Some(&format!("{bad}{EVERY_FIELD}"))).await;
            assert!(
                matches!(res.status, StatusCode::FORBIDDEN | StatusCode::UNPROCESSABLE_ENTITY),
                "{path} with {bad:?} answered {}",
                res.status
            );
        }
    }
    assert_eq!(snapshot(&app).await, before, "a refused action changed something");
    // And ada is still signed in (the bad sign-out didn't sign her out).
    assert_eq!(send(&app, "GET", "/", Some(&ada), None).await.status, StatusCode::OK);
}

#[tokio::test]
async fn v1_signed_out_visitors_are_sent_to_sign_in() {
    let (app, _) = with_ada(false).await;
    let ada = sign_in(&app, "ada", None).await;
    let g = goal(&app, &ada, "Launch", None).await;
    let t = task(&app, &ada, g, "Schema").await;
    let before = snapshot(&app).await;
    for path in [
        "/",
        "/goals",
        "/goals/new",
        &format!("/goals/{g}"),
        &format!("/tasks/{t}"),
        "/settings",
        "/settings/export",
        "/settings/delete",
        "/settings/2fa/setup",
        &format!("/invitations/{g}"),
        "/transfers/1",
    ] {
        let r = send(&app, "GET", path, None, None).await;
        assert_eq!(r.location.as_deref(), Some("/login"), "GET {path}");
    }
    for path in post_routes(g, t, 1) {
        if path == "/logout" {
            continue;
        }
        let r = send(&app, "POST", &path, None, Some(&format!("csrf=x&{EVERY_FIELD}"))).await;
        assert!(
            r.location.as_deref() == Some("/login") || r.status == StatusCode::UNPROCESSABLE_ENTITY,
            "POST {path}: {} {:?}",
            r.status,
            r.location
        );
    }
    assert_eq!(snapshot(&app).await, before);
}

#[tokio::test]
async fn f1_f2_a_fresh_instance_needs_the_setup_link() {
    let app = empty().await;
    for path in ["/", "/goals", "/login", "/signup", "/settings"] {
        assert_eq!(send(&app, "GET", path, None, None).await.location.as_deref(), Some("/setup"), "{path}");
    }
    let r = send(&app, "GET", "/setup", None, None).await;
    assert!(r.body.contains("isn’t set up yet") && !r.body.contains("<form"), "no form without the link");
    let r = send(&app, "GET", "/setup?token=guess", None, None).await;
    assert!(!r.body.contains("<form"));
    assert_eq!(send(&app, "GET", "/api/health", None, None).await.body, "ok");
    assert_eq!(send(&app, "GET", "/static/styles.css", None, None).await.status, StatusCode::OK);
}

#[tokio::test]
async fn g1_security_headers() {
    let (app, _) = with_ada(false).await;
    for path in ["/login", "/static/app.js", "/nope"] {
        let req = Request::builder().uri(path).header(header::HOST, "fluxo.test").body(Body::empty()).unwrap();
        let res = app.router.clone().oneshot(req).await.unwrap();
        let h = res.headers();
        let csp = h[header::CONTENT_SECURITY_POLICY].to_str().unwrap();
        for part in [
            "default-src 'self'",
            "script-src 'self'",
            "style-src 'self'",
            "frame-ancestors 'none'",
            "object-src 'none'",
        ] {
            assert!(csp.contains(part), "{path}: {part}");
        }
        assert!(!csp.contains("unsafe"), "{path}");
        assert_eq!(h[header::X_CONTENT_TYPE_OPTIONS], "nosniff");
        assert_eq!(h[header::X_FRAME_OPTIONS], "DENY");
        assert_eq!(h[header::REFERRER_POLICY], "same-origin");
        assert!(h[header::STRICT_TRANSPORT_SECURITY].to_str().unwrap().starts_with("max-age="));
        assert!(h.contains_key("permissions-policy"));
        let cache = h[header::CACHE_CONTROL].to_str().unwrap();
        assert_eq!(cache == "no-store", !path.starts_with("/static/"), "{path}: {cache}");
    }
}

#[tokio::test]
async fn g4_g7_not_found_reveals_nothing() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "lin", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let lin = sign_in(&app, "lin", None).await;
    let p = goal(&app, &ada, "Secret plan", None).await;
    make_private(&app, &ada, p).await;
    let missing = send(&app, "GET", "/goals/987654", Some(&lin), None).await;
    let private = send(&app, "GET", &format!("/goals/{p}"), Some(&lin), None).await;
    assert_eq!(missing.status, StatusCode::NOT_FOUND);
    assert_eq!(private.status, StatusCode::NOT_FOUND);
    assert_eq!(missing.body, private.body, "a private goal looks exactly like a missing one");
    assert!(!private.body.contains("Secret"));
    let r = send(&app, "GET", "/no/such/page", Some(&lin), None).await;
    assert_eq!(r.status, StatusCode::NOT_FOUND);
    assert!(r.body.contains("Go to Focus"));
    for path in ["/static/nope.js", "/static/..%2fCargo.toml", "/static/../Cargo.toml"] {
        assert_eq!(send(&app, "GET", path, None, None).await.status, StatusCode::NOT_FOUND, "{path}");
    }
}

#[tokio::test]
async fn g8_v2_s3_what_each_kind_of_person_sees() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let r = send(&app, "GET", "/login", None, None).await;
    assert!(r.body.contains("Locked out?") && !r.body.contains("Create an account"), "invite-only by default");
    sqlx::query("UPDATE instance SET signup_mode = 'open'").execute(&app.db).await.unwrap();
    assert!(send(&app, "GET", "/login", None, None).await.body.contains("Create an account"));

    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let focus = send(&app, "GET", "/", Some(&kofi), None).await.body;
    assert!(focus.contains(r#"href="/goals">Goals</a>"#), "Goals is a plain link");
    assert!(focus.contains(r#"href="/" aria-current="page">Focus"#));
    assert!(focus.contains("Sign out") && focus.contains(r#"href="/settings""#));
    let goals = send(&app, "GET", "/goals", Some(&kofi), None).await.body;
    assert!(goals.contains(r#"href="/goals" aria-current="page""#));

    let member = send(&app, "GET", "/settings", Some(&kofi), None).await.body;
    let admin = send(&app, "GET", "/settings", Some(&ada), None).await.body;
    assert!(!member.contains(r#"id="adm-h""#) && admin.contains(r#"id="adm-h""#));
    for page in [&member, &admin, &focus] {
        assert!(!page.contains("Site Administrator") && !page.contains("Site Administration"));
    }
    let kt = csrf(&member);
    for (path, form) in [
        ("/settings/admin/signup", "mode=closed".to_string()),
        ("/settings/admin/invites", "note=x".into()),
        ("/settings/admin/recovery", "username=ada".into()),
        ("/settings/admin/role", "user=2&admin=1".into()),
        ("/settings/admin/assign", "node=1&username=kofi".into()),
        ("/settings/admin/invites/revoke", "invite=1".into()),
        ("/settings/admin/recovery/revoke", "code=1".into()),
        ("/settings/admin/retention", "days=7".into()),
    ] {
        let r = send(&app, "POST", path, Some(&kofi), Some(&format!("csrf={kt}&{form}"))).await;
        assert_eq!(r.status, StatusCode::FORBIDDEN, "{path}");
    }
}

#[tokio::test]
async fn go2_fo3_fo4_empty_states() {
    let (app, _) = with_ada(false).await;
    let ada = sign_in(&app, "ada", None).await;
    let goals = send(&app, "GET", "/goals", Some(&ada), None).await.body;
    assert!(goals.contains("No goals yet."));
    assert!(!goals.contains("Tasks not attached") && !goals.contains("Archived"));
    let focus = send(&app, "GET", "/", Some(&ada), None).await.body;
    assert!(focus.contains("Nothing yet. Changes to goals"));
    assert!(focus.contains("Nothing to rank yet."));
    assert!(!focus.contains("Waiting on you") && !focus.contains("Mark all read"));
    assert!(!focus.contains(r#"class="count""#), "no badge with nothing new");
    let other = send(&app, "GET", "/?show=derisk", Some(&ada), None).await.body;
    assert!(other.contains("Nothing here right now."));
    let g = goal(&app, &ada, "Launch", None).await;
    let page = send(&app, "GET", &format!("/goals/{g}"), Some(&ada), None).await.body;
    assert!(page.contains("No steps yet."));
}

#[tokio::test]
async fn fo5_gp1_gp4_t1_titles_of_private_goals_never_leak() {
    let (app, _) = with_ada(false).await;
    let kofi_id = add_user(&app, "kofi", false).await;
    add_user(&app, "lin", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let lin = sign_in(&app, "lin", None).await;

    // ada's private goal "Secret plan" with a task kofi contributes to directly, and a sub-goal
    // kofi contributes to.
    let p = goal(&app, &ada, "Secret plan", None).await;
    make_private(&app, &ada, p).await;
    let t = task(&app, &ada, p, "Schema").await;
    let sub = goal(&app, &ada, "Hidden sub", Some(p)).await;
    role(&app, t, kofi_id, "contributor").await;
    role(&app, sub, kofi_id, "contributor").await;
    // ada finishes a step, an outlook change kofi hears about.
    let at = token(&app, &ada, &format!("/tasks/{t}")).await;
    let t2 = task(&app, &ada, p, "Other step").await;
    send(
        &app,
        "POST",
        &format!("/tasks/{t2}"),
        Some(&ada),
        Some(&format!("csrf={at}&title=Other+step&estimate=2&clarity=high&status=done")),
    )
    .await;
    send(
        &app,
        "POST",
        &format!("/tasks/{t}"),
        Some(&ada),
        Some(&format!("csrf={at}&title=Schema&estimate=2&clarity=high&status=done")),
    )
    .await;
    send(
        &app,
        "POST",
        &format!("/tasks/{t}"),
        Some(&ada),
        Some(&format!("csrf={at}&title=Schema&estimate=2&clarity=high&status=todo")),
    )
    .await;

    for path in ["/".to_string(), "/goals".into(), "/goals/new".into(), format!("/tasks/{t}"), format!("/goals/{sub}")]
    {
        let r = send(&app, "GET", &path, Some(&kofi), None).await;
        assert_eq!(r.status, StatusCode::OK, "{path}");
        assert!(!r.body.contains("Secret plan"), "kofi can see the title of ada's private goal on {path}");
    }
    let task_page = send(&app, "GET", &format!("/tasks/{t}"), Some(&kofi), None).await.body;
    assert!(task_page.contains("Private goal"), "breadcrumb placeholder");
    let focus = send(&app, "GET", "/", Some(&kofi), None).await.body;
    assert!(focus.contains("Schema") && focus.contains("a private goal"), "Focus lists the work, not the goal's title");

    // A transfer offer shows the goals it feeds, masked for someone who can't read them.
    let tk = token(&app, &ada, &format!("/tasks/{t}")).await;
    send(
        &app,
        "POST",
        &format!("/nodes/{t}/transfer"),
        Some(&ada),
        Some(&format!("csrf={tk}&action=request&username=lin&note=")),
    )
    .await;
    let tid: i64 = sqlx::query_scalar("SELECT MAX(id) FROM transfers").fetch_one(&app.db).await.unwrap();
    let offer = send(&app, "GET", &format!("/transfers/{tid}"), Some(&lin), None).await.body;
    assert!(offer.contains("Schema") && !offer.contains("Secret plan"));

    // On a public goal, a private sub-goal's title doesn't leak through "Needs".
    let g = goal(&app, &ada, "Public goal", None).await;
    let hidden = goal(&app, &ada, "Covert sub", Some(g)).await;
    make_private(&app, &ada, hidden).await;
    let tk = token(&app, &ada, &format!("/goals/{g}")).await;
    send(
        &app,
        "POST",
        &format!("/goals/{g}/steps"),
        Some(&ada),
        Some(&format!("csrf={tk}&title=Visible&kind=task&estimate=1&clarity=high&needs={hidden}")),
    )
    .await;
    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&lin), None).await.body;
    assert!(r.contains("Visible") && r.contains("Private step") && !r.contains("Covert"));
}

#[tokio::test]
async fn l2_contributors_cannot_take_or_expose_work_by_relinking() {
    let (app, _) = with_ada(false).await;
    let kofi_id = add_user(&app, "kofi", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;

    let g = goal(&app, &ada, "Ada goal", None).await;
    let t = task(&app, &ada, g, "Ada task").await;
    role(&app, t, kofi_id, "contributor").await;
    let mine = goal(&app, &kofi, "Kofi goal", None).await;

    // kofi links ada's task under his own goal: that would make him an owner of it.
    let kt = token(&app, &kofi, &format!("/tasks/{t}")).await;
    let r = send(
        &app,
        "POST",
        &format!("/nodes/{t}/parents"),
        Some(&kofi),
        Some(&format!("csrf={kt}&action=add&goal={mine}")),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some(&*format!("/tasks/{t}?error=link-owner#links-h")));
    let page = send(&app, "GET", &r.location.unwrap(), Some(&kofi), None).await.body;
    assert!(page.contains("Only an owner can make that change"));
    let links: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM node_parents WHERE child_id = ?")
        .bind(t)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(links, 1);
    let kt = token(&app, &kofi, "/").await;
    let r = send(&app, "POST", &format!("/nodes/{t}/archive"), Some(&kofi), Some(&format!("csrf={kt}"))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "still not an owner");

    // Linking it under another of ada's goals changes no one's control: fine for kofi.
    let g2 = goal(&app, &ada, "Ada goal two", None).await;
    role(&app, g2, kofi_id, "contributor").await;
    let r = send(
        &app,
        "POST",
        &format!("/nodes/{t}/parents"),
        Some(&kofi),
        Some(&format!("csrf={kt}&action=add&goal={g2}")),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some(&*format!("/tasks/{t}#links-h")));
    // Making a private goal the home would hide it from people who can see it now: owner only.
    make_private(&app, &ada, g2).await;
    let r = send(
        &app,
        "POST",
        &format!("/nodes/{t}/parents"),
        Some(&kofi),
        Some(&format!("csrf={kt}&action=home&goal={g2}")),
    )
    .await;
    assert!(r.location.unwrap().contains("error=link-owner"));
    // The owner can do all of it (given Contributor on the goal it joins).
    let ada_id: i64 =
        sqlx::query_scalar("SELECT id FROM users WHERE username = 'ada'").fetch_one(&app.db).await.unwrap();
    role(&app, mine, ada_id, "contributor").await;
    let at = token(&app, &ada, "/").await;
    let r = send(
        &app,
        "POST",
        &format!("/nodes/{t}/parents"),
        Some(&ada),
        Some(&format!("csrf={at}&action=add&goal={mine}")),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some(&*format!("/tasks/{t}#links-h")));
}

#[tokio::test]
async fn d3_i1_handover_replaces_earlier_offers_and_archived_invitations_wait() {
    let (app, _) = with_ada(false).await;
    let kofi_id = add_user(&app, "kofi", true).await;
    add_user(&app, "lin", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let lin = sign_in(&app, "lin", None).await;
    let g = goal(&app, &ada, "Launch", None).await;
    let t = token(&app, &ada, &format!("/goals/{g}")).await;
    send(
        &app,
        "POST",
        &format!("/nodes/{g}/transfer"),
        Some(&ada),
        Some(&format!("csrf={t}&action=request&username=lin&note=")),
    )
    .await;
    send(
        &app,
        "POST",
        &format!("/nodes/{g}/people"),
        Some(&ada),
        Some(&format!("csrf={t}&action=invite&username=lin&role=contributor")),
    )
    .await;
    // Archived: lin's invitation can't be accepted.
    send(&app, "POST", &format!("/nodes/{g}/archive"), Some(&ada), Some(&format!("csrf={t}"))).await;
    let lt = token(&app, &lin, "/").await;
    send(&app, "POST", &format!("/invitations/{g}"), Some(&lin), Some(&format!("csrf={lt}&answer=accept"))).await;
    let status: String = sqlx::query_scalar("SELECT status FROM memberships WHERE node_id = ?")
        .bind(g)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(status, "pending");
    send(&app, "POST", &format!("/nodes/{g}/restore"), Some(&ada), Some(&format!("csrf={t}"))).await;

    send(
        &app,
        "POST",
        "/settings/delete",
        Some(&ada),
        Some(&format!("csrf={t}&confirm=ada&password=a+long+enough+password&to_{g}={kofi_id}")),
    )
    .await;
    let pending: Vec<i64> =
        sqlx::query_scalar("SELECT to_user FROM transfers WHERE node_id = ? AND status = 'pending'")
            .bind(g)
            .fetch_all(&app.db)
            .await
            .unwrap();
    assert_eq!(pending, vec![kofi_id], "one pending offer, to the chosen person");
}

#[tokio::test]
async fn se1_idle_sessions_are_purged() {
    let (app, _) = with_ada(false).await;
    let ada = sign_in(&app, "ada", None).await;
    for (stage, age) in [("anonymous", 3_700), ("second_factor", 700), ("signed_in", 31 * 86_400)] {
        sqlx::query("INSERT INTO sessions (id_hash, user_id, stage, csrf_token, created_at, last_seen_at, user_agent) VALUES (randomblob(32), 1, ?, 'x', 0, ?, '')")
            .bind(stage)
            .bind(fluxo_web::now() - age)
            .execute(&app.db)
            .await
            .unwrap();
    }
    let removed = fluxo_web::session::purge_expired(&app.db).await.unwrap();
    assert_eq!(removed, 3);
    assert_eq!(send(&app, "GET", "/", Some(&ada), None).await.status, StatusCode::OK, "active sessions stay");
}

#[tokio::test]
async fn g6_health_says_nothing_else() {
    let (app, _) = with_ada(false).await;
    let r = send(&app, "GET", "/api/health", None, None).await;
    assert_eq!((r.status, r.body.as_str()), (StatusCode::OK, "ok"));
    assert!(r.cookie.is_none(), "no session for health checks");
}

#[tokio::test]
async fn g13_control_and_bidi_characters_are_removed_from_text() {
    let (app, _) = with_ada(false).await;
    let ada = sign_in(&app, "ada", None).await;
    let t = token(&app, &ada, "/goals/new").await;
    let title = "\u{202E}Pay\u{0}\u{7} invoice\u{2066}\r\n";
    let why = "Line one\r\nLine two\u{202D}\u{1b}[31m";
    let r = send(
        &app,
        "POST",
        "/goals",
        Some(&ada),
        Some(&format!("csrf={t}&title={}&why={}&parent=", enc(title), enc(why))),
    )
    .await;
    let id: i64 = r.location.unwrap().trim_start_matches("/goals/").parse().unwrap();
    let (stored_title, stored_why): (String, String) =
        sqlx::query_as("SELECT title, why FROM nodes WHERE id = ?").bind(id).fetch_one(&app.db).await.unwrap();
    assert_eq!(stored_title, "Pay invoice");
    assert_eq!(stored_why, "Line one\nLine two[31m", "line breaks stay in multi-line text");
    // A title made only of such characters is empty, so it's refused like an empty one.
    let r = send(
        &app,
        "POST",
        "/goals",
        Some(&ada),
        Some(&format!("csrf={t}&title={}&why=&parent=", enc("\u{202E}\u{0}"))),
    )
    .await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(r.body.contains("Give it a title"));
    // Display names too.
    let t = token(&app, &ada, "/settings").await;
    send(
        &app,
        "POST",
        "/settings/profile",
        Some(&ada),
        Some(&format!("csrf={t}&display_name={}", enc("A\u{202E}da\n"))),
    )
    .await;
    let name: String =
        sqlx::query_scalar("SELECT display_name FROM users WHERE username = 'ada'").fetch_one(&app.db).await.unwrap();
    assert_eq!(name, "Ada");
}

#[tokio::test]
async fn go6_a_malformed_parent_creates_nothing() {
    let (app, _) = with_ada(false).await;
    let ada = sign_in(&app, "ada", None).await;
    let t = token(&app, &ada, "/goals/new").await;
    for parent in ["abc", "9223372036854775808", "1.5", "-1", "999"] {
        let r = send(&app, "POST", "/goals", Some(&ada), Some(&format!("csrf={t}&title=x&why=&parent={parent}"))).await;
        assert!(matches!(r.status, StatusCode::NOT_FOUND | StatusCode::FORBIDDEN), "parent {parent}: {}", r.status);
    }
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM nodes").fetch_one(&app.db).await.unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn g14_picker_labels_show_at_most_two_goals_above() {
    let (app, _) = with_ada(false).await;
    let ada = sign_in(&app, "ada", None).await;
    let mut parent = None;
    for i in 1..=6 {
        parent = Some(goal(&app, &ada, &format!("Level {i}"), parent).await);
    }
    let page = send(&app, "GET", "/goals/new", Some(&ada), None).await.body;
    assert!(page.contains("… › Level 4 › Level 5 › Level 6"), "deep goals are shortened");
    assert!(page.contains(">Level 1 › Level 2 › Level 3<"), "shallow ones are shown whole");
    assert!(!page.contains("Level 1 › Level 2 › Level 3 › Level 4"));
}

/// A member nesting goals thousands deep must not make pages slow or huge, for them or anyone
/// else: access is worked out in one pass, and labels don't grow with depth (PF1).
#[tokio::test]
async fn pf1_a_very_deep_tree_stays_cheap_for_everyone() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let t0 = fluxo_web::now();
    let mut tx = app.db.begin().await.unwrap();
    for id in 1..=3000i64 {
        let parent = (id > 1).then_some(id - 1);
        sqlx::query(
            "INSERT INTO nodes (id, kind, title, owner_id, home_parent_id, coverage, statement_changed_at, created_by, created_at)
             VALUES (?, 'goal', ?, 1, ?, 'low', ?, 1, ?)",
        )
        .bind(id)
        .bind(format!("Depth {id}"))
        .bind(parent)
        .bind(t0)
        .bind(t0)
        .execute(&mut *tx)
        .await
        .unwrap();
        if let Some(p) = parent {
            sqlx::query("INSERT INTO node_parents (child_id, parent_id, added_at) VALUES (?, ?, ?)")
                .bind(id)
                .bind(p)
                .bind(t0)
                .execute(&mut *tx)
                .await
                .unwrap();
        }
    }
    tx.commit().await.unwrap();
    let kofi = sign_in(&app, "kofi", None).await;
    let ada = sign_in(&app, "ada", None).await;
    let started = std::time::Instant::now();
    for (c, path) in
        [(&kofi, "/"), (&kofi, "/goals"), (&kofi, "/goals/new"), (&kofi, "/goals/3000"), (&ada, "/goals/new")]
    {
        let r = send(&app, "GET", path, Some(c), None).await;
        assert_eq!(r.status, StatusCode::OK, "{path}");
        assert!(r.body.len() < 1_500_000, "{path} is {} bytes", r.body.len());
    }
    // Generous, so it only fails if the cost is back to growing with depth squared (that took
    // minutes in debug builds at this depth).
    assert!(started.elapsed().as_secs() < 60, "took {:?}", started.elapsed());
}
