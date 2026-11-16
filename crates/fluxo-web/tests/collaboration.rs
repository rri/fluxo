//! Phase 4: invitations, roles, watching, ownership transfers, and Updates on Focus.

mod common;
use common::*;

use axum::http::StatusCode;

async fn token(app: &App, c: &str, path: &str) -> String {
    csrf(&send(app, "GET", path, Some(c), None).await.body)
}

async fn goal(app: &App, c: &str, title: &str) -> i64 {
    let t = token(app, c, "/goals/new").await;
    let r = send(app, "POST", "/goals", Some(c), Some(&format!("csrf={t}&title={}&why=&parent=", enc(title)))).await;
    let id: i64 = r.location.unwrap().trim_start_matches("/goals/").parse().unwrap();
    send(app, "POST", &format!("/goals/{id}/shape"), Some(c), Some(&format!("csrf={t}&coverage=full"))).await;
    id
}

async fn task(app: &App, c: &str, goal: i64, title: &str) -> i64 {
    let t = token(app, c, &format!("/goals/{goal}")).await;
    send(
        app,
        "POST",
        &format!("/goals/{goal}/steps"),
        Some(c),
        Some(&format!("csrf={t}&title={}&kind=task&estimate=5&clarity=high", enc(title))),
    )
    .await;
    sqlx::query_scalar("SELECT MAX(id) FROM nodes").fetch_one(&app.db).await.unwrap()
}

/// The number on the Focus button, or 0.
async fn badge(app: &App, c: &str) -> usize {
    let body = send(app, "GET", "/goals", Some(c), None).await.body;
    body.find(r#"<span class="count""#)
        .map(|i| {
            let s = &body[i..];
            let s = &s[s.find('>').unwrap() + 1..];
            s[..s.find('<').unwrap()].parse().unwrap()
        })
        .unwrap_or(0)
}

async fn focus(app: &App, c: &str) -> String {
    send(app, "GET", "/", Some(c), None).await.body
}

#[tokio::test]
async fn invite_accept_and_contribute() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let g = goal(&app, &ada, "Launch").await;
    let t = token(&app, &ada, &format!("/goals/{g}")).await;

    let r = send(
        &app,
        "POST",
        &format!("/nodes/{g}/people"),
        Some(&ada),
        Some(&format!("csrf={t}&action=invite&username=nobody&role=contributor")),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some(format!("/goals/{g}?error=no-user#people").as_str()));
    let r = send(
        &app,
        "POST",
        &format!("/nodes/{g}/people"),
        Some(&ada),
        Some(&format!("csrf={t}&action=invite&username=Kofi&role=contributor")),
    )
    .await;
    assert_eq!(r.location.as_deref(), Some(format!("/goals/{g}?done=invited#people").as_str()));
    assert!(send(&app, "GET", &format!("/goals/{g}"), Some(&ada), None).await.body.contains("Invited · Contributor"));

    // kofi sees it waiting, and can't edit until he accepts.
    assert_eq!(badge(&app, &kofi).await, 1);
    let f = focus(&app, &kofi).await;
    assert!(f.contains("Waiting on you"));
    assert!(f.contains("invited you as a Contributor to <b>Launch</b>"));
    let kt = token(&app, &kofi, &format!("/goals/{g}")).await;
    let r = send(
        &app,
        "POST",
        &format!("/goals/{g}/steps"),
        Some(&kofi),
        Some(&format!("csrf={kt}&title=X&kind=task&estimate=1")),
    )
    .await;
    assert_eq!(r.status, StatusCode::FORBIDDEN);
    let r = send(&app, "GET", &format!("/invitations/{g}"), Some(&kofi), None).await;
    assert!(r.body.contains("Accept invitation"));
    let r =
        send(&app, "POST", &format!("/invitations/{g}"), Some(&kofi), Some(&format!("csrf={kt}&answer=accept"))).await;
    assert_eq!(r.location.as_deref(), Some(format!("/goals/{g}").as_str()));
    let r = send(
        &app,
        "POST",
        &format!("/goals/{g}/steps"),
        Some(&kofi),
        Some(&format!("csrf={kt}&title=Helpful&kind=task&estimate=1")),
    )
    .await;
    assert_eq!(r.status, StatusCode::SEE_OTHER);
    assert_eq!(badge(&app, &kofi).await, 0, "nothing waiting any more");

    // ada hears that kofi joined; reading clears the badge.
    let f = focus(&app, &ada).await;
    assert!(f.contains(r#"kofi joined <a href="/goals/"#));
    assert!(f.contains("as a Contributor"));
    assert!(badge(&app, &ada).await >= 1);
    let upto = field(&f, r#"name="upto" value=""#);
    send(&app, "POST", "/updates/read", Some(&ada), Some(&format!("csrf={t}&upto={upto}"))).await;
    assert_eq!(badge(&app, &ada).await, 0);
    assert!(focus(&app, &ada).await.contains("All read"));

    // Roles shown, inherited ones marked.
    let sub_task = task(&app, &ada, g, "Under launch").await;
    let r = send(&app, "GET", &format!("/tasks/{sub_task}"), Some(&ada), None).await;
    assert!(r.body.contains("kofi <span class=\"handle\">· from Launch</span>"));
}

#[tokio::test]
async fn declined_and_expired_invitations() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let g = goal(&app, &ada, "G").await;
    let t = token(&app, &ada, &format!("/goals/{g}")).await;
    send(
        &app,
        "POST",
        &format!("/nodes/{g}/people"),
        Some(&ada),
        Some(&format!("csrf={t}&action=invite&username=kofi&role=watcher")),
    )
    .await;
    let kt = token(&app, &kofi, "/").await;
    send(&app, "POST", &format!("/invitations/{g}"), Some(&kofi), Some(&format!("csrf={kt}&answer=decline"))).await;
    assert!(!focus(&app, &kofi).await.contains("Waiting on you"));
    assert!(!send(&app, "GET", &format!("/goals/{g}"), Some(&ada), None).await.body.contains("Invited ·"));

    send(
        &app,
        "POST",
        &format!("/nodes/{g}/people"),
        Some(&ada),
        Some(&format!("csrf={t}&action=invite&username=kofi&role=watcher")),
    )
    .await;
    sqlx::query("UPDATE memberships SET invited_at = invited_at - 15 * 86400").execute(&app.db).await.unwrap();
    assert!(!focus(&app, &kofi).await.contains("Waiting on you"), "expired after 14 days");
    assert!(send(&app, "GET", &format!("/invitations/{g}"), Some(&kofi), None).await.body.contains("has expired"));
    send(&app, "POST", &format!("/invitations/{g}"), Some(&kofi), Some(&format!("csrf={kt}&answer=accept"))).await;
    let n: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM memberships WHERE status = 'active'")
        .fetch_one(&app.db)
        .await
        .unwrap();
    assert_eq!(n, 0);
}

#[tokio::test]
async fn watchers_hear_about_outlook_changes_only() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let g = goal(&app, &ada, "Public goal").await;
    let job = task(&app, &ada, g, "Job").await;
    send(&app, "GET", &format!("/goals/{g}"), Some(&ada), None).await; // first forecast, sets the baseline

    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&kofi), None).await;
    assert!(r.body.contains(">Watch</button>"));
    let kt = csrf(&r.body);
    send(&app, "POST", &format!("/nodes/{g}/people"), Some(&kofi), Some(&format!("csrf={kt}&action=watch"))).await;
    assert!(send(&app, "GET", &format!("/goals/{g}"), Some(&kofi), None).await.body.contains("Stop watching"));
    assert!(focus(&app, &ada).await.contains("kofi started watching"));

    let t = token(&app, &ada, &format!("/tasks/{job}")).await;
    // Not an Update: an estimate tweak.
    send(
        &app,
        "POST",
        &format!("/tasks/{job}"),
        Some(&ada),
        Some(&format!("csrf={t}&title=Job&estimate=5.5&clarity=high&status=todo")),
    )
    .await;
    // Updates: a risk added, raised; finishing the task. Not: a risk lowered.
    send(
        &app,
        "POST",
        &format!("/nodes/{job}/risks"),
        Some(&ada),
        Some(&format!("csrf={t}&title=Vendor+slip&likelihood=30&min=10&max=10")),
    )
    .await;
    let rid: i64 = sqlx::query_scalar("SELECT id FROM risks").fetch_one(&app.db).await.unwrap();
    send(
        &app,
        "POST",
        &format!("/risks/{rid}"),
        Some(&ada),
        Some(&format!("csrf={t}&action=likelihood&likelihood=10")),
    )
    .await;
    send(
        &app,
        "POST",
        &format!("/risks/{rid}"),
        Some(&ada),
        Some(&format!("csrf={t}&action=likelihood&likelihood=90")),
    )
    .await;
    let f = focus(&app, &kofi).await;
    assert!(f.contains("ada added a risk to"));
    assert!(f.contains("ada raised a risk on"));
    assert!(f.contains(" from 10% to 90%"));
    assert!(!f.contains("from 30% to 10%"), "lowering a risk isn't news");
    assert!(f.contains("The finish of"), "the near-certain 10-day risk moved the forecast");
    assert!(f.contains(" moved: P85 "));
    send(
        &app,
        "POST",
        &format!("/tasks/{job}"),
        Some(&ada),
        Some(&format!("csrf={t}&title=Job&estimate=5.5&clarity=high&status=done")),
    )
    .await;
    let f = focus(&app, &kofi).await;
    assert!(f.contains("ada finished"));
    assert!(!f.contains("estimate"), "no estimate noise");
    // Your own actions never notify you.
    assert!(!focus(&app, &ada).await.contains("ada finished"));

    // Stop watching: nothing more.
    let kt = token(&app, &kofi, &format!("/goals/{g}")).await;
    send(&app, "POST", &format!("/nodes/{g}/people"), Some(&kofi), Some(&format!("csrf={kt}&action=leave"))).await;
    let t = token(&app, &ada, &format!("/goals/{g}")).await;
    send(&app, "POST", &format!("/goals/{g}/achieved"), Some(&ada), Some(&format!("csrf={t}&achieved=1"))).await;
    assert!(!focus(&app, &kofi).await.contains("marked"));
}

#[tokio::test]
async fn ownership_transfer_needs_acceptance() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let g = goal(&app, &ada, "Hosting").await;
    let t = token(&app, &ada, &format!("/goals/{g}")).await;
    send(
        &app,
        "POST",
        &format!("/nodes/{g}/transfer"),
        Some(&ada),
        Some(&format!("csrf={t}&action=request&username=kofi&note=You+know+infra+best")),
    )
    .await;
    assert!(
        send(&app, "GET", &format!("/goals/{g}"), Some(&ada), None)
            .await
            .body
            .contains("Waiting for <b>kofi</b> to accept")
    );
    let f = focus(&app, &kofi).await;
    assert!(f.contains("wants you to own <b>Hosting</b>"));
    let tid: i64 = sqlx::query_scalar("SELECT id FROM transfers").fetch_one(&app.db).await.unwrap();
    let r = send(&app, "GET", &format!("/transfers/{tid}"), Some(&kofi), None).await;
    assert!(r.body.contains("“You know infra best”"));
    assert_eq!(
        send(&app, "GET", &format!("/transfers/{tid}"), Some(&ada), None).await.status,
        StatusCode::NOT_FOUND,
        "only for the recipient"
    );
    let kt = csrf(&r.body);
    send(&app, "POST", &format!("/transfers/{tid}"), Some(&kofi), Some(&format!("csrf={kt}&answer=accept"))).await;

    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&kofi), None).await;
    assert!(r.body.contains("Owner: you"));
    assert!(r.body.contains(r#"id="edit-h""#), "kofi gets Goal settings");
    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&ada), None).await;
    assert!(r.body.contains("Owner: kofi"));
    assert!(!r.body.contains(r#"id="edit-h""#), "ada is now a Contributor");
    assert!(r.body.contains("+ Add a step"));
    assert!(focus(&app, &ada).await.contains("kofi now owns"));
}

#[tokio::test]
async fn private_goals_are_not_watchable_but_invited_people_can_see_them() {
    let (app, _) = with_ada(false).await;
    add_user(&app, "kofi", false).await;
    let ada = sign_in(&app, "ada", None).await;
    let kofi = sign_in(&app, "kofi", None).await;
    let g = goal(&app, &ada, "Secret").await;
    let t = token(&app, &ada, &format!("/goals/{g}")).await;
    send(&app, "POST", &format!("/goals/{g}/shape"), Some(&ada), Some(&format!("csrf={t}&coverage=full&private=1")))
        .await;
    assert_eq!(send(&app, "GET", &format!("/goals/{g}"), Some(&kofi), None).await.status, StatusCode::NOT_FOUND);
    send(
        &app,
        "POST",
        &format!("/nodes/{g}/people"),
        Some(&ada),
        Some(&format!("csrf={t}&action=invite&username=kofi&role=watcher")),
    )
    .await;
    assert!(
        send(&app, "GET", &format!("/invitations/{g}"), Some(&kofi), None).await.body.contains("even if it’s private")
    );
    let kt = token(&app, &kofi, "/").await;
    send(&app, "POST", &format!("/invitations/{g}"), Some(&kofi), Some(&format!("csrf={kt}&answer=accept"))).await;
    let r = send(&app, "GET", &format!("/goals/{g}"), Some(&kofi), None).await;
    assert_eq!(r.status, StatusCode::OK);
    assert!(!r.body.contains(">Watch</button>"));
    let r =
        send(&app, "POST", &format!("/nodes/{g}/people"), Some(&kofi), Some(&format!("csrf={kt}&action=leave"))).await;
    assert_eq!(r.location.as_deref(), Some("/goals"), "leaving a private goal leaves nothing to see");
}
