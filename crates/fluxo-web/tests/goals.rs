//! Phase 2: goals and tasks, links, risks, access, archiving.

mod common;
use common::*;

use axum::http::StatusCode;

async fn new_goal(app: &App, c: &str, title: &str, parent: Option<i64>) -> i64 {
    let token = csrf(&send(app, "GET", "/goals/new", Some(c), None).await.body);
    let p = parent.map(|p| p.to_string()).unwrap_or_default();
    let r = send(
        app,
        "POST",
        "/goals",
        Some(c),
        Some(&format!("csrf={token}&title={}&why=Because&parent={p}", enc(title))),
    )
    .await;
    let loc = r.location.expect("redirect to the new goal");
    loc.trim_start_matches("/goals/").parse().unwrap()
}

async fn add_task(app: &App, c: &str, goal: i64, title: &str, estimate: &str, needs: Option<i64>) -> i64 {
    let token = csrf(&send(app, "GET", &format!("/goals/{goal}"), Some(c), None).await.body);
    let n = needs.map(|n| n.to_string()).unwrap_or_default();
    let r = send(
        app,
        "POST",
        &format!("/goals/{goal}/steps"),
        Some(c),
        Some(&format!("csrf={token}&title={}&kind=task&estimate={estimate}&clarity=high&needs={n}", enc(title))),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some(format!("/goals/{goal}#path").as_str()));
    sqlx::query_scalar("SELECT MAX(id) FROM nodes").fetch_one(&app.db).await.unwrap()
}

async fn token(app: &App, c: &str, path: &str) -> String {
    csrf(&send(app, "GET", path, Some(c), None).await.body)
}

#[tokio::test]
async fn break_a_goal_down_into_a_path() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    assert!(send(&app, "GET", "/goals", Some(&c), None).await.body.contains("No goals yet"));

    let goal = new_goal(&app, &c, "Launch fluxo.dev beta", None).await;
    let r = send(&app, "GET", &format!("/goals/{goal}"), Some(&c), None).await;
    assert!(r.body.contains("<h1>Launch fluxo.dev beta</h1>"));
    assert!(r.body.contains("No steps yet"));
    assert!(r.body.contains("Coverage low"), "new goals start with low coverage");

    let schema = add_task(&app, &c, goal, "SQLite schema v1", "3", None).await;
    let api = add_task(&app, &c, goal, "Rust API skeleton", "5", Some(schema)).await;
    let t = token(&app, &c, &format!("/goals/{goal}")).await;
    send(
        &app,
        "POST",
        &format!("/goals/{goal}/steps"),
        Some(&c),
        Some(&format!("csrf={t}&title=Accounts+%26+auth&kind=goal&coverage=partial&needs={api}")),
    )
    .await;
    let auth: i64 =
        sqlx::query_scalar("SELECT id FROM nodes WHERE title = 'Accounts & auth'").fetch_one(&app.db).await.unwrap();

    let r = send(&app, "GET", &format!("/goals/{goal}"), Some(&c), None).await;
    assert!(r.body.contains(&format!(r#"<li data-id="{api}" data-deps="{schema}""#)), "graph knows API needs schema");
    assert!(
        r.body.contains(&format!(r#"data-id="goal" data-deps="{auth} gap""#)),
        "goal is fed by the last step, then unknown work"
    );
    assert!(r.body.contains("Needs: SQLite schema v1"), "list for no-JS readers");
    assert!(r.body.contains(r#"class="node subgoal"#));

    let r = send(&app, "GET", "/goals", Some(&c), None).await;
    assert!(r.body.contains("Launch fluxo.dev beta"));
    assert!(r.body.contains(">Accounts &#38; auth</a><span class=\"pill xs\">sub-goal</span>"));
    assert!(r.body.contains("2 tasks"));

    // Sub-goal page shows where it lives.
    let r = send(&app, "GET", &format!("/goals/{auth}"), Some(&c), None).await;
    assert!(r.body.contains(&format!(r#"<a href="/goals/{goal}">Launch fluxo.dev beta</a>"#)));
    assert!(r.body.contains(">Sub-goal</span>"));
    assert_eq!(
        send(&app, "GET", &format!("/tasks/{auth}"), Some(&c), None).await.location.as_deref(),
        Some(format!("/goals/{auth}").as_str())
    );
}

#[tokio::test]
async fn task_progress_and_achieving_the_goal() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let goal = new_goal(&app, &c, "Ship", None).await;
    let task = add_task(&app, &c, goal, "Do it", "4", None).await;
    let path = format!("/tasks/{task}");
    let t = token(&app, &c, &path).await;
    let r = send(
        &app,
        "POST",
        &path,
        Some(&c),
        Some(&format!("csrf={t}&title=Do+it&what=W&why=Y&estimate=0&clarity=low&status=todo")),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some(format!("/tasks/{task}?error=estimate#top").as_str()));
    send(
        &app,
        "POST",
        &path,
        Some(&c),
        Some(&format!("csrf={t}&title=Do+it&what=W&why=Y&estimate=4&clarity=low&status=doing&remaining=2.5")),
    )
    .await;
    let r = send(&app, "GET", &path, Some(&c), None).await;
    assert!(r.body.contains("In progress"));
    assert!(r.body.contains(r#"name="remaining" class="input num" value="2.5""#));
    assert!(r.body.contains("simulated as <span class=\"num\">1.5–5.5</span>"), "low clarity spread on remaining days");
    assert!(!send(&app, "GET", &format!("/goals/{goal}"), Some(&c), None).await.body.contains("Every step is done"));

    send(
        &app,
        "POST",
        &path,
        Some(&c),
        Some(&format!("csrf={t}&title=Do+it&what=W&why=Y&estimate=4&clarity=low&status=done")),
    )
    .await;
    let r = send(&app, "GET", &format!("/goals/{goal}"), Some(&c), None).await;
    assert!(r.body.contains("Every step is done"), "owner is asked to mark the goal achieved");
    let t = csrf(&r.body);
    send(&app, "POST", &format!("/goals/{goal}/achieved"), Some(&c), Some(&format!("csrf={t}&achieved=1"))).await;
    let r = send(&app, "GET", &format!("/goals/{goal}"), Some(&c), None).await;
    assert!(r.body.contains("pill accent\">Achieved"));
    let events: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE kind IN ('status_changed', 'achieved')")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(events, 3);
}

#[tokio::test]
async fn access_follows_ownership_roles_and_privacy() {
    let (app, _) = with_ada(false).await;
    let kofi_id = add_user(&app, "kofi", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let goal = new_goal(&app, &ada, "Public goal", None).await;
    let task = add_task(&app, &ada, goal, "A task", "2", None).await;

    // Everyone signed in can read a public goal, but not change it.
    let r = send(&app, "GET", &format!("/goals/{goal}"), Some(&kofi), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!r.body.contains("Add a step"));
    assert!(!r.body.contains(r#"id="edit-h""#), "no Goal settings for non-owners");
    let kt = csrf(&r.body);
    let r = send(
        &app,
        "POST",
        &format!("/goals/{goal}/steps"),
        Some(&kofi),
        Some(&format!("csrf={kt}&title=Sneaky&kind=task&estimate=1")),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(send(&app, "GET", "/goals", Some(&kofi), None).await.body.contains("open to everyone"));

    // A Contributor changes the path, but not the goal itself.
    sqlx::query("INSERT INTO memberships (node_id, user_id, role, invited_at) VALUES (?, ?, 'contributor', 0)")
        .bind(goal)
        .bind(kofi_id)
        .execute(&app.db)
        .await
        .unwrap();
    let r = send(
        &app,
        "POST",
        &format!("/goals/{goal}/steps"),
        Some(&kofi),
        Some(&format!("csrf={kt}&title=Helpful&kind=task&estimate=1")),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    let r = send(
        &app,
        "POST",
        &format!("/goals/{goal}/statement"),
        Some(&kofi),
        Some(&format!("csrf={kt}&title=Renamed&why=")),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "only the owner rewords a goal");
    let r = send(&app, "POST", &format!("/nodes/{task}/archive"), Some(&kofi), Some(&format!("csrf={kt}"))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "only owners archive");

    // Private: invisible without a role, and it cascades down.
    let secret = new_goal(&app, &ada, "Secret goal", None).await;
    let inner = add_task(&app, &ada, secret, "Inner", "1", None).await;
    let t = token(&app, &ada, &format!("/goals/{secret}")).await;
    send(
        &app,
        "POST",
        &format!("/goals/{secret}/shape"),
        Some(&ada),
        Some(&format!("csrf={t}&coverage=partial&private=1")),
    )
    .await;
    assert_eq!(send(&app, "GET", &format!("/goals/{secret}"), Some(&kofi), None).await.status, StatusCode::NOT_FOUND);
    assert_eq!(send(&app, "GET", &format!("/tasks/{inner}"), Some(&kofi), None).await.status, StatusCode::NOT_FOUND);
    assert!(!send(&app, "GET", "/goals", Some(&kofi), None).await.body.contains("Secret goal"));
    assert!(send(&app, "GET", &format!("/goals/{secret}"), Some(&ada), None).await.body.contains(">Private<"));

    // The statement change is recorded, and the stamp says so.
    send(
        &app,
        "POST",
        &format!("/goals/{goal}/statement"),
        Some(&ada),
        Some(&format!("csrf={t}&title=Public+goal&why=Clearer+why")),
    )
    .await;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE kind = 'statement_changed'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 1);
}

#[tokio::test]
async fn links_refuse_loops_and_keep_tasks_attached() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let g1 = new_goal(&app, &c, "One", None).await;
    let g2 = new_goal(&app, &c, "Two", None).await;
    let a = add_task(&app, &c, g1, "A", "1", None).await;
    let b = add_task(&app, &c, g1, "B", "1", Some(a)).await;
    let t = token(&app, &c, &format!("/tasks/{a}")).await;

    let r = send(&app, "POST", &format!("/nodes/{a}/needs"), Some(&c), Some(&format!("csrf={t}&action=add&other={b}")))
        .await;
    assert_eq!(r.location.as_deref(), Some(format!("/tasks/{a}?error=loop#links-h").as_str()));
    assert!(
        send(&app, "GET", &format!("/tasks/{a}?error=loop"), Some(&c), None).await.body.contains("would make a loop")
    );

    // A shared task: it feeds a second goal; home can move.
    send(&app, "POST", &format!("/nodes/{a}/parents"), Some(&c), Some(&format!("csrf={t}&action=add&goal={g2}"))).await;
    let r = send(&app, "GET", &format!("/goals/{g2}"), Some(&c), None).await;
    assert!(r.body.contains("Task · feeds 2 goals"));
    send(&app, "POST", &format!("/nodes/{a}/parents"), Some(&c), Some(&format!("csrf={t}&action=home&goal={g2}")))
        .await;
    assert!(
        send(&app, "GET", &format!("/tasks/{a}"), Some(&c), None)
            .await
            .body
            .contains(&format!(r#"<a href="/goals/{g2}">Two</a>"#))
    );

    // A goal can't contain itself.
    let r =
        send(&app, "POST", &format!("/nodes/{g1}/parents"), Some(&c), Some(&format!("csrf={t}&action=add&goal={g1}")))
            .await;
    assert_eq!(r.location.as_deref(), Some(format!("/goals/{g1}?error=loop#links-h").as_str()));

    // A task can be left attached to no goal: allowed, with a notice and a place to find it.
    let r = send(
        &app,
        "POST",
        &format!("/nodes/{b}/parents"),
        Some(&c),
        Some(&format!("csrf={t}&action=remove&goal={g1}")),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some(format!("/tasks/{b}#top").as_str()));
    let r = send(&app, "GET", &format!("/tasks/{b}"), Some(&c), None).await;
    assert!(r.body.contains("This task isn’t attached to any goal."));
    assert!(r.body.contains("Attach to this goal"));
    let r = send(&app, "GET", "/goals", Some(&c), None).await;
    assert!(r.body.contains("Tasks not attached to any goal"));
    assert!(r.body.contains(&format!(r#"<a class="grow" href="/tasks/{b}#unattached">B</a>"#)));
    let r = send(&app, "GET", &format!("/goals/{g1}"), Some(&c), None).await;
    assert!(!r.body.contains(&format!(r#"data-id="{b}""#)), "no longer on that goal's path");
    // Attach it again from the notice.
    send(&app, "POST", &format!("/nodes/{b}/parents"), Some(&c), Some(&format!("csrf={t}&action=add&goal={g2}"))).await;
    assert!(!send(&app, "GET", &format!("/tasks/{b}"), Some(&c), None).await.body.contains("isn’t attached"));
    assert!(!send(&app, "GET", "/goals", Some(&c), None).await.body.contains("Tasks not attached"));
}

#[tokio::test]
async fn risks_keep_their_likelihood_history() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let g = new_goal(&app, &c, "G", None).await;
    let task = add_task(&app, &c, g, "Engine", "10", None).await;
    let t = token(&app, &c, &format!("/tasks/{task}")).await;
    let r = send(
        &app,
        "POST",
        &format!("/nodes/{task}/risks"),
        Some(&c),
        Some(&format!("csrf={t}&title=Too+slow&likelihood=60&min=8&max=3")),
    )
    .await;
    assert_eq!(
        r.location.as_deref(),
        Some(format!("/tasks/{task}?error=risk#risks").as_str()),
        "min above max is refused"
    );
    send(
        &app,
        "POST",
        &format!("/nodes/{task}/risks"),
        Some(&c),
        Some(&format!("csrf={t}&title=Too+slow&likelihood=60%25&min=3&max=8")),
    )
    .await;
    let rid: i64 = sqlx::query_scalar("SELECT id FROM risks").fetch_one(&app.db).await.unwrap();
    for p in [40, 40, 0] {
        send(
            &app,
            "POST",
            &format!("/risks/{rid}"),
            Some(&c),
            Some(&format!("csrf={t}&action=likelihood&likelihood={p}")),
        )
        .await;
    }
    let r = send(&app, "GET", &format!("/tasks/{task}"), Some(&c), None).await;
    assert!(r.body.contains("Likelihood history: 60% → 40% → 0%"), "unchanged values aren't repeated");
    assert!(r.body.contains("class=\"risk retired\""), "0% retires it");
    assert!(
        !send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await.body.contains("Top risks on the path"),
        "retired risks drop out"
    );
    send(&app, "POST", &format!("/risks/{rid}"), Some(&c), Some(&format!("csrf={t}&action=likelihood&likelihood=30")))
        .await;
    assert!(send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await.body.contains("30% · +3–8d"));
    send(&app, "POST", &format!("/risks/{rid}"), Some(&c), Some(&format!("csrf={t}&action=remove"))).await;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM risk_likelihood").fetch_one(&app.db).await.unwrap();
    assert_eq!(n, 0, "history goes with the risk");
}

#[tokio::test]
async fn archive_restore_and_delete() {
    let (app, _) = with_ada(false).await;
    let c = sign_in(&app, "ada", None).await;
    let g = new_goal(&app, &c, "Parent", None).await;
    let other = new_goal(&app, &c, "Other", None).await;
    let only = add_task(&app, &c, g, "Only here", "1", None).await;
    let shared = add_task(&app, &c, g, "Shared", "1", None).await;
    let t = token(&app, &c, &format!("/tasks/{shared}")).await;
    send(
        &app,
        "POST",
        &format!("/nodes/{shared}/parents"),
        Some(&c),
        Some(&format!("csrf={t}&action=add&goal={other}")),
    )
    .await;

    send(&app, "POST", &format!("/nodes/{only}/archive"), Some(&c), Some(&format!("csrf={t}"))).await;
    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await;
    assert!(!r.body.contains(&format!(r#"data-id="{only}""#)), "archived steps leave the path");
    assert!(r.body.contains("Archived steps"));
    send(&app, "POST", &format!("/nodes/{only}/restore"), Some(&c), Some(&format!("csrf={t}"))).await;
    assert!(
        send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await.body.contains(&format!(r#"data-id="{only}""#))
    );

    let r = send(&app, "POST", &format!("/nodes/{g}/delete"), Some(&c), Some(&format!("csrf={t}"))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN, "only archived items can be deleted");
    send(&app, "POST", &format!("/nodes/{g}/archive"), Some(&c), Some(&format!("csrf={t}"))).await;
    // Owners archive; only Administrators delete for good.
    add_user(&app, "kofi", false).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let kt = token(&app, &kofi, "/settings").await;
    let r = send(&app, "POST", &format!("/nodes/{g}/delete"), Some(&kofi), Some(&format!("csrf={kt}"))).await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    assert!(!send(&app, "GET", &format!("/goals/{g}"), Some(&c), None).await.body.contains("Delete permanently"));
    let r = send(&app, "GET", "/settings", Some(&c), None).await;
    assert!(r.body.contains("Archived goals and tasks"));
    assert!(r.body.contains("and 1 step only it holds"));
    assert!(send(&app, "GET", "/goals", Some(&c), None).await.body.contains("<b class=\"small grow\">Archived</b>"));
    let r = send(&app, "POST", &format!("/nodes/{g}/delete"), Some(&c), Some(&format!("csrf={t}"))).await;
    assert_eq!(r.location.as_deref(), Some("/settings?done=deleted#archived-h"));
    let left: Vec<String> = sqlx::query_scalar("SELECT title FROM nodes ORDER BY id").fetch_all(&app.db).await.unwrap();
    assert_eq!(left, vec!["Other", "Shared"], "what only it held is gone; shared work stays");
    let home: Option<i64> = sqlx::query_scalar("SELECT home_parent_id FROM nodes WHERE id = ?")
        .bind(shared)
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(home, Some(other));
}
