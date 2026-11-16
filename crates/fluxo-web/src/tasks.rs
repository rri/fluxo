//! A task's page: what and why, estimate and clarity, progress, links and risks.

use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use fluxo_core::access::Access;
use serde::Deserialize;
use serde_json::json;

use crate::goals::PageQuery;
use crate::nodes::{gate, with_access};
use crate::store::{self, Id, Node, World};
use crate::util::date;
use crate::views::{self, Crumb, Option_, RiskView};
use crate::{Result, Shared, initials, now, page};

pub struct Link {
    pub id: Id,
    pub title: String,
    pub href: String,
}

#[derive(Template)]
#[template(path = "task.html")]
struct TaskPage<'a> {
    people: crate::people::PeopleView,
    done: Option<&'static str>,
    history: Vec<crate::updates::Line>,
    theme: &'a str,
    focus_count: usize,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
    node_id: Id,
    crumbs: Vec<Crumb>,
    title: String,
    what: String,
    why: String,
    owner: String,
    estimate: String,
    clarity: String,
    spread: String,
    status: String,
    remaining: String,
    started: String,
    archived: bool,
    can_edit: bool,
    is_owner: bool,
    /// Feeds no goal: show the "not attached" notice.
    unattached: bool,
    needs: Vec<Link>,
    unblocks: Vec<Link>,
    need_nearby: Vec<Option_>,
    need_elsewhere: Vec<Option_>,
    feeds: Vec<views::Feed>,
    parent_options: Vec<Option_>,
    risks: Vec<RiskView>,
    error: Option<&'static str>,
    /// (goal title, href, criticality %) for each goal it feeds.
    critical_in: Vec<(String, String, String)>,
    done_p50: String,
    done_p85: String,
}

fn link(world: &World, user: Id, id: Id) -> Option<Link> {
    let n = world.nodes.get(&id).filter(|n| n.archived_at.is_none())?;
    Some(if world.access(user, id).can_read() {
        Link { id, title: n.title.clone(), href: n.href() }
    } else {
        Link { id, title: "A private step".into(), href: String::new() }
    })
}

pub async fn show(State(app): State<Shared>, jar: CookieJar, Path(id): Path<Id>, Query(q): Query<PageQuery>) -> Result<Response> {
    let (ctx, world, access) = match with_access(&app, &jar, None, id, Access::Read).await? {
        Ok(x) => x,
        Err(r) => return Ok(r),
    };
    let task = &world.nodes[&id];
    if task.is_goal() {
        return Ok(Redirect::to(&task.href()).into_response());
    }
    let names = store::usernames(&app.db).await?;
    let user = ctx.user_id;
    let estimate = task.estimate_days.unwrap_or(0.0);
    let clarity = task.clarity.clone().unwrap_or_else(|| "medium".into());
    let (lo, hi) = match clarity.as_str() {
        "high" => (0.85, 1.3),
        "low" => (0.6, 2.2),
        _ => (0.75, 1.6),
    };
    let base = if task.status == "doing" { task.remaining_days.unwrap_or(estimate) } else { estimate };
    let spread = format!("{}–{}", views::days(base * lo).trim_end_matches('d'), views::days(base * hi).trim_end_matches('d'));

    // "Needs" choices: siblings first, then everything else visible (D16).
    let siblings: std::collections::HashSet<Id> =
        world.parents.get(&id).into_iter().flatten().flat_map(|p| world.children_of(*p)).map(|n| n.id).collect();
    let already: std::collections::HashSet<Id> = world.needs_of(id).iter().copied().collect();
    let candidate = |n: &&Node| {
        n.id != id && n.archived_at.is_none() && !already.contains(&n.id) && world.access(user, n.id).can_read()
    };
    let mut nearby: Vec<&Node> = world.nodes.values().filter(candidate).filter(|n| siblings.contains(&n.id)).collect();
    let mut elsewhere: Vec<&Node> =
        world.nodes.values().filter(candidate).filter(|n| !siblings.contains(&n.id) && !n.is_goal()).collect();
    nearby.sort_by_key(|n| (n.created_at, n.id));
    elsewhere.sort_by(|a, b| views::path_label(&world, a).cmp(&views::path_label(&world, b)));
    let opt = |n: &Node, label: String| Option_ { value: n.id, label, selected: false };

    let current_parents = world.parents.get(&id).cloned().unwrap_or_default();
    let mut parent_options: Vec<&Node> = world
        .nodes
        .values()
        .filter(|n| n.is_goal() && n.archived_at.is_none() && !current_parents.contains(&n.id))
        .filter(|n| world.access(user, n.id).can_contribute())
        .collect();
    parent_options.sort_by(|a, b| views::path_label(&world, a).cmp(&views::path_label(&world, b)));

    let forecasts = crate::forecast::current(&app, &world).await?;
    let critical_in = world
        .parents_of(id)
        .iter()
        .filter(|p| p.archived_at.is_none() && p.achieved_at.is_none() && world.access(user, p.id).can_read())
        .map(|p| {
            let c = forecasts.by_node.get(&p.id).and_then(|f| f.criticality.get(&id)).copied().unwrap_or(0.0);
            (p.title.clone(), p.href(), format!("{:.0}%", c * 100.0))
        })
        .collect();
    let (done_p50, done_p85) = match forecasts.by_node.get(&id).filter(|_| task.status != "done" && !current_parents.is_empty()) {
        Some(f) => {
            let cals = crate::forecast::calendars(&app.db).await?;
            let home_owner = task.home_parent_id.and_then(|h| world.nodes.get(&h)).and_then(|h| h.owner_id);
            let cal = home_owner.or(task.owner_id).and_then(|o| cals.get(&o).cloned()).unwrap_or_default();
            let d = crate::forecast::dated(f, &cal);
            (d.p50, d.p85)
        }
        None => (String::new(), String::new()),
    };
    let people = crate::people::view(&app.db, &world, user, id, access).await?;
    let history = views::history(&app.db, &world, user, id).await?;
    page(TaskPage {
        people,
        history,
        done: views::done_text(q.done.as_deref()),
        focus_count: crate::updates::badge(&app.db, ctx.user_id).await?,
        critical_in,
        done_p50,
        done_p85,
        theme: &ctx.theme,
        nav: "goals",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
        node_id: id,
        crumbs: views::crumbs(&world, user, id),
        title: task.title.clone(),
        what: task.what.clone(),
        why: task.why.clone(),
        owner: views::owner_label(task.owner_id, user, &names),
        estimate: views::days(estimate).trim_end_matches('d').to_string(),
        clarity,
        spread,
        status: task.status.clone(),
        remaining: task.remaining_days.map(|d| views::days(d).trim_end_matches('d').to_string()).unwrap_or_default(),
        started: task.started_at.map(date).unwrap_or_default(),
        archived: task.archived_at.is_some(),
        can_edit: access.can_contribute() && task.archived_at.is_none(),
        is_owner: access.is_owner(),
        unattached: current_parents.is_empty(),
        needs: world.needs_of(id).iter().filter_map(|d| link(&world, user, *d)).collect(),
        unblocks: world.needed_by(id).iter().filter_map(|d| link(&world, user, *d)).collect(),
        need_nearby: nearby.into_iter().map(|n| opt(n, n.title.clone())).collect(),
        need_elsewhere: elsewhere.into_iter().map(|n| opt(n, views::path_label(&world, n))).collect(),
        feeds: views::feeds(&world, user, id),
        parent_options: parent_options.into_iter().map(|n| opt(n, views::path_label(&world, n))).collect(),
        risks: views::risk_views(store::risks(&app.db, id).await?),
        error: views::error_text(q.error.as_deref()),
    })
}

#[derive(Deserialize)]
pub struct TaskForm {
    csrf: String,
    title: String,
    #[serde(default)]
    what: String,
    #[serde(default)]
    why: String,
    estimate: String,
    clarity: String,
    status: String,
    #[serde(default)]
    remaining: String,
}

pub async fn update(State(app): State<Shared>, jar: CookieJar, Path(id): Path<Id>, Form(f): Form<TaskForm>) -> Result<Response> {
    let (ctx, world, _) = gate!(app, jar, f.csrf, id, Access::Contribute);
    let task = &world.nodes[&id];
    if task.is_goal() || task.archived_at.is_some() {
        return Ok(Redirect::to(&task.href()).into_response());
    }
    let title = f.title.trim();
    if views::check_title(title).is_some() || f.what.chars().count() > 5000 || f.why.chars().count() > 5000 {
        return Ok(Redirect::to(&format!("/tasks/{id}?error=title#top")).into_response());
    }
    let Some(estimate) = views::parse_days(&f.estimate) else {
        return Ok(Redirect::to(&format!("/tasks/{id}?error=estimate#top")).into_response());
    };
    let clarity = if matches!(f.clarity.as_str(), "low" | "medium" | "high") { f.clarity.as_str() } else { "medium" };
    let status = if matches!(f.status.as_str(), "todo" | "doing" | "done") { f.status.as_str() } else { "todo" };
    let remaining = if status == "doing" { views::parse_days(&f.remaining).or(task.remaining_days).or(Some(estimate)) } else { None };
    let started = match (status, task.started_at) {
        ("todo", _) => None,
        (_, Some(s)) => Some(s),
        _ => Some(now()),
    };
    let (what, why) = (f.what.trim(), f.why.trim());
    sqlx::query!(
        "UPDATE nodes SET title = ?, what = ?, why = ?, estimate_days = ?, clarity = ?, status = ?, remaining_days = ?,
                          started_at = ? WHERE id = ?",
        title,
        what,
        why,
        estimate,
        clarity,
        status,
        remaining,
        started,
        id
    )
    .execute(&app.db)
    .await?;
    if status != task.status {
        store::record(&app.db, id, ctx.user_id, "status_changed", json!({ "from": task.status, "to": status })).await?;
    }
    Ok(Redirect::to(&format!("/tasks/{id}#top")).into_response())
}
