//! Goals: the Goals page, creating goals, a goal's own page, and the goal-only actions (statement,
//! coverage and privacy, marking achieved, adding steps).

use askama::Template;
use axum::extract::{Form, Path, Query, State};
use axum::response::{IntoResponse, Redirect, Response};
use axum_extra::extract::cookie::CookieJar;
use fluxo_core::access::Access;
use serde::Deserialize;
use serde_json::json;

use crate::nodes::{gate, not_found, with_access};
use crate::store::{self, Id, Node, World};
use crate::util::date;
use crate::views::{self, Crumb, Option_, RiskView, StepView};
use crate::{Result, Shared, forbidden, initials, now, page, require_user};

// ---------------------------------------------------------------- Goals page

pub struct GoalCard {
    pub href: String,
    pub title: String,
    pub why: String,
    pub role: &'static str,
    pub private: bool,
    pub achieved: bool,
    pub coverage: String,
    pub steps: usize,
    pub done: usize,
    pub subgoals: Vec<(String, String, String)>, // href, title, meta
    pub tasks: usize,
}

#[derive(Template)]
#[template(path = "goals.html")]
struct GoalsPage<'a> {
    theme: &'a str,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
    goals: Vec<GoalCard>,
    archived: Vec<(String, String)>,
}

fn role_label(a: Access) -> &'static str {
    match a {
        Access::Own => "you own it",
        Access::Contribute => "Contributor",
        Access::Watch => "Watcher",
        _ => "open to everyone",
    }
}

fn card(world: &World, user: Id, g: &Node) -> GoalCard {
    let kids: Vec<&Node> = world.children_of(g.id).into_iter().filter(|k| world.access(user, k.id).can_read()).collect();
    let subgoals = kids
        .iter()
        .filter(|k| k.is_goal())
        .map(|k| {
            let n = world.children_of(k.id).len();
            (k.href(), k.title.clone(), format!("{n} step{}", if n == 1 { "" } else { "s" }))
        })
        .collect();
    GoalCard {
        href: g.href(),
        title: g.title.clone(),
        why: g.why.clone(),
        role: role_label(world.access(user, g.id)),
        private: world.graph.is_private(g.id),
        achieved: g.achieved_at.is_some(),
        coverage: g.coverage.clone().unwrap_or_default(),
        steps: kids.len(),
        done: kids.iter().filter(|k| k.is_done()).count(),
        subgoals,
        tasks: kids.iter().filter(|k| !k.is_goal()).count(),
    }
}

pub async fn index(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let world = World::load(&app.db).await?;
    let mut tops: Vec<&Node> = world
        .nodes
        .values()
        .filter(|n| n.is_goal() && world.parents.get(&n.id).is_none_or(Vec::is_empty))
        .filter(|n| world.access(ctx.user_id, n.id).can_read())
        .collect();
    // Yours first, then the ones you work on, then everything else you can see; oldest first within.
    tops.sort_by_key(|n| (std::cmp::Reverse(world.access(ctx.user_id, n.id)), n.achieved_at.is_some(), n.created_at, n.id));
    let goals = tops.iter().filter(|n| n.archived_at.is_none()).map(|g| card(&world, ctx.user_id, g)).collect();
    let archived = tops.iter().filter(|n| n.archived_at.is_some()).map(|g| (g.href(), g.title.clone())).collect();
    page(GoalsPage {
        theme: &ctx.theme,
        nav: "goals",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
        goals,
        archived,
    })
}

// ---------------------------------------------------------------- new goal

#[derive(Template)]
#[template(path = "goal_new.html")]
struct NewGoalPage<'a> {
    theme: &'a str,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
    parents: Vec<Option_>,
    error: Option<&'a str>,
    title: &'a str,
    why: &'a str,
}

#[derive(Deserialize)]
pub struct NewQuery {
    parent: Option<Id>,
}

/// Goals this person can add a sub-goal to.
fn parent_options(world: &World, user: Id, selected: Option<Id>, exclude: Option<Id>) -> Vec<Option_> {
    let mut v: Vec<&Node> = world
        .nodes
        .values()
        .filter(|n| n.is_goal() && n.archived_at.is_none() && Some(n.id) != exclude)
        .filter(|n| world.access(user, n.id).can_contribute())
        .filter(|n| exclude.is_none_or(|x| !world.graph.would_contain_itself(x, n.id)))
        .collect();
    v.sort_by(|a, b| views::path_label(world, a).cmp(&views::path_label(world, b)));
    v.into_iter().map(|n| Option_ { value: n.id, label: views::path_label(world, n), selected: Some(n.id) == selected }).collect()
}

pub async fn new_page(State(app): State<Shared>, jar: CookieJar, Query(q): Query<NewQuery>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let world = World::load(&app.db).await?;
    page(NewGoalPage {
        theme: &ctx.theme,
        nav: "goals",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
        parents: parent_options(&world, ctx.user_id, q.parent, None),
        error: None,
        title: "",
        why: "",
    })
}

#[derive(Deserialize)]
pub struct NewGoalForm {
    csrf: String,
    title: String,
    why: String,
    #[serde(default)]
    parent: String,
}

pub(crate) async fn insert_node(
    app: &crate::AppState,
    user: Id,
    kind: &str,
    title: &str,
    why: &str,
    parent: Option<Id>,
    task: Option<(f64, &str)>,
    coverage: Option<&str>,
) -> Result<Id> {
    let t = now();
    let (estimate, clarity) = task.map(|(e, c)| (Some(e), Some(c))).unwrap_or((None, None));
    let mut tx = app.db.begin().await?;
    let id = sqlx::query_scalar!(
        r#"INSERT INTO nodes (kind, title, why, owner_id, home_parent_id, coverage, clarity, estimate_days,
                              statement_changed_at, created_by, created_at)
           VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?) RETURNING id AS "id!""#,
        kind,
        title,
        why,
        user,
        parent,
        coverage,
        clarity,
        estimate,
        t,
        user,
        t
    )
    .fetch_one(&mut *tx)
    .await?;
    if let Some(p) = parent {
        sqlx::query!("INSERT INTO node_parents (child_id, parent_id, added_at) VALUES (?, ?, ?)", id, p, t)
            .execute(&mut *tx)
            .await?;
    }
    tx.commit().await?;
    store::record(&app.db, id, user, "created", json!({ "kind": kind, "parent": parent })).await?;
    Ok(id)
}

pub async fn create(State(app): State<Shared>, jar: CookieJar, Form(f): Form<NewGoalForm>) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    if !ctx.csrf_ok(&f.csrf) {
        return Ok(forbidden());
    }
    let world = World::load(&app.db).await?;
    let parent: Option<Id> = f.parent.trim().parse().ok();
    if let Some(p) = parent {
        if !world.nodes.get(&p).is_some_and(|n| n.is_goal()) || !world.access(ctx.user_id, p).can_contribute() {
            return Ok(forbidden());
        }
    }
    let (title, why) = (f.title.trim(), f.why.trim());
    let error = views::check_title(title).or_else(|| (why.chars().count() > 2000).then_some("Keep the why under 2,000 characters."));
    if let Some(error) = error {
        return page(NewGoalPage {
            theme: &ctx.theme,
            nav: "goals",
            csrf: &ctx.session.csrf_token,
            username: &ctx.username,
            initials: initials(&ctx.username),
            parents: parent_options(&world, ctx.user_id, parent, None),
            error: Some(error),
            title,
            why,
        });
    }
    let id = insert_node(&app, ctx.user_id, "goal", title, why, parent, None, Some("low")).await?;
    Ok(Redirect::to(&format!("/goals/{id}")).into_response())
}

// ---------------------------------------------------------------- a goal's page

#[derive(Template)]
#[template(path = "goal.html")]
struct GoalPage<'a> {
    theme: &'a str,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
    id: Id,
    node_id: Id,
    crumbs: Vec<Crumb>,
    title: String,
    why: String,
    is_sub: bool,
    owner: String,
    coverage: String,
    private: bool,
    private_here: bool,
    achieved: Option<String>,
    archived: bool,
    stamp: String,
    steps: Vec<StepView>,
    anchor_deps: String,
    suggest_achieved: bool,
    can_edit: bool,
    is_owner: bool,
    sibling_options: Vec<Option_>,
    feeds: Vec<views::Feed>,
    parent_options: Vec<Option_>,
    risks: Vec<RiskView>,
    path_risks: Vec<views::PathRisk>,
    archived_steps: Vec<(Id, String, String)>,
    error: Option<&'static str>,
}

#[derive(Deserialize)]
pub struct PageQuery {
    pub(crate) error: Option<String>,
}

pub async fn show(State(app): State<Shared>, jar: CookieJar, Path(id): Path<Id>, Query(q): Query<PageQuery>) -> Result<Response> {
    let (ctx, world, access) = match with_access(&app, &jar, None, id, Access::Read).await? {
        Ok(x) => x,
        Err(r) => return Ok(r),
    };
    let goal = &world.nodes[&id];
    if !goal.is_goal() {
        return Ok(Redirect::to(&goal.href()).into_response());
    }
    let names = store::usernames(&app.db).await?;
    let steps = views::steps(&world, ctx.user_id, id);
    let needed_inside: std::collections::HashSet<Id> =
        steps.iter().flat_map(|s| s.deps.split(' ').filter_map(|d| d.parse::<Id>().ok())).collect();
    let anchor_deps = steps.iter().filter(|s| !needed_inside.contains(&s.id)).map(|s| s.id.to_string()).collect::<Vec<_>>().join(" ");
    let all_done = !steps.is_empty() && steps.iter().all(|s| s.done);
    let stamp = if goal.statement_changed_at > goal.created_at + 60 {
        format!("Set {} · statement changed {}", date(goal.created_at), date(goal.statement_changed_at))
    } else {
        format!("Set {} · statement unchanged since", date(goal.created_at))
    };
    let risks = views::risk_views(store::risks(&app.db, id).await?);
    let path_risks = views::path_risks(&app.db, &world, ctx.user_id, id).await?;
    let error = views::error_text(q.error.as_deref());
    page(GoalPage {
        theme: &ctx.theme,
        nav: "goals",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
        id,
        node_id: id,
        crumbs: views::crumbs(&world, ctx.user_id, id),
        title: goal.title.clone(),
        why: goal.why.clone(),
        is_sub: world.parents.get(&id).is_some_and(|p| !p.is_empty()),
        owner: views::owner_label(goal.owner_id, ctx.user_id, &names),
        coverage: goal.coverage.clone().unwrap_or_default(),
        private: world.graph.is_private(id),
        private_here: goal.private,
        achieved: goal.achieved_at.map(date),
        archived: goal.archived_at.is_some(),
        stamp,
        anchor_deps,
        suggest_achieved: all_done && goal.achieved_at.is_none() && access.is_owner(),
        steps,
        can_edit: access.can_contribute() && goal.archived_at.is_none(),
        is_owner: access.is_owner(),
        sibling_options: views::step_options(&world, ctx.user_id, id),
        feeds: views::feeds(&world, ctx.user_id, id),
        parent_options: parent_options(&world, ctx.user_id, None, Some(id))
            .into_iter()
            .filter(|o| !world.parents.get(&id).is_some_and(|p| p.contains(&o.value)))
            .collect(),
        risks,
        path_risks,
        archived_steps: if access.is_owner() {
            world.archived_children_of(id).iter().map(|n| (n.id, n.title.clone(), n.href())).collect()
        } else {
            Vec::new()
        },
        error,
    })
}

// ---------------------------------------------------------------- goal actions

#[derive(Deserialize)]
pub struct StepForm {
    csrf: String,
    title: String,
    kind: String,
    #[serde(default)]
    estimate: String,
    #[serde(default)]
    clarity: String,
    #[serde(default)]
    coverage: String,
    #[serde(default)]
    needs: String,
}

pub async fn add_step(State(app): State<Shared>, jar: CookieJar, Path(id): Path<Id>, Form(f): Form<StepForm>) -> Result<Response> {
    let (ctx, world, _) = gate!(app, jar, f.csrf, id, Access::Contribute);
    let goal = &world.nodes[&id];
    if !goal.is_goal() || goal.archived_at.is_some() {
        return Ok(not_found());
    }
    let title = f.title.trim();
    if views::check_title(title).is_some() {
        return Ok(Redirect::to(&format!("/goals/{id}?error=title#add")).into_response());
    }
    let new_id = if f.kind == "goal" {
        let coverage = if matches!(f.coverage.as_str(), "low" | "partial" | "full") { f.coverage.as_str() } else { "low" };
        insert_node(&app, ctx.user_id, "goal", title, "", Some(id), None, Some(coverage)).await?
    } else {
        let Some(estimate) = views::parse_days(&f.estimate) else {
            return Ok(Redirect::to(&format!("/goals/{id}?error=estimate#add")).into_response());
        };
        let clarity = if matches!(f.clarity.as_str(), "low" | "medium" | "high") { f.clarity.as_str() } else { "medium" };
        insert_node(&app, ctx.user_id, "task", title, "", Some(id), Some((estimate, clarity)), None).await?
    };
    if let Ok(needs) = f.needs.trim().parse::<Id>() {
        let world = World::load(&app.db).await?;
        if world.access(ctx.user_id, needs).can_read() && !world.would_loop(None, Some((new_id, needs))) {
            let t = now();
            sqlx::query!("INSERT INTO dependencies (node_id, needs_id, added_at) VALUES (?, ?, ?)", new_id, needs, t)
                .execute(&app.db)
                .await?;
        }
    }
    Ok(Redirect::to(&format!("/goals/{id}#path")).into_response())
}

#[derive(Deserialize)]
pub struct StatementForm {
    csrf: String,
    title: String,
    why: String,
}

pub async fn statement(State(app): State<Shared>, jar: CookieJar, Path(id): Path<Id>, Form(f): Form<StatementForm>) -> Result<Response> {
    let (ctx, world, _) = gate!(app, jar, f.csrf, id, Access::Own);
    let goal = &world.nodes[&id];
    let (title, why) = (f.title.trim(), f.why.trim());
    if views::check_title(title).is_some() || why.chars().count() > 2000 {
        return Ok(Redirect::to(&format!("/goals/{id}?error=title#edit")).into_response());
    }
    if title != goal.title || why != goal.why {
        let t = now();
        sqlx::query!("UPDATE nodes SET title = ?, why = ?, statement_changed_at = ? WHERE id = ?", title, why, t, id)
            .execute(&app.db)
            .await?;
        store::record(
            &app.db,
            id,
            ctx.user_id,
            "statement_changed",
            json!({ "from": { "title": goal.title, "why": goal.why }, "to": { "title": title, "why": why } }),
        )
        .await?;
    }
    Ok(Redirect::to(&format!("/goals/{id}#top")).into_response())
}

#[derive(Deserialize)]
pub struct ShapeForm {
    csrf: String,
    coverage: String,
    #[serde(default)]
    private: Option<String>,
}

pub async fn shape(State(app): State<Shared>, jar: CookieJar, Path(id): Path<Id>, Form(f): Form<ShapeForm>) -> Result<Response> {
    let (ctx, world, _) = gate!(app, jar, f.csrf, id, Access::Own);
    if !world.nodes[&id].is_goal() || !matches!(f.coverage.as_str(), "low" | "partial" | "full") {
        return Ok(Redirect::to(&format!("/goals/{id}#edit")).into_response());
    }
    let private = f.private.is_some() as i64;
    sqlx::query!("UPDATE nodes SET coverage = ?, private = ? WHERE id = ?", f.coverage, private, id).execute(&app.db).await?;
    store::record(&app.db, id, ctx.user_id, "shape_changed", json!({ "coverage": f.coverage, "private": private == 1 })).await?;
    Ok(Redirect::to(&format!("/goals/{id}#edit")).into_response())
}

#[derive(Deserialize)]
pub struct AchievedForm {
    csrf: String,
    achieved: String,
}

pub async fn achieved(State(app): State<Shared>, jar: CookieJar, Path(id): Path<Id>, Form(f): Form<AchievedForm>) -> Result<Response> {
    let (ctx, _, _) = gate!(app, jar, f.csrf, id, Access::Own);
    let when = (f.achieved == "1").then(now);
    sqlx::query!("UPDATE nodes SET achieved_at = ? WHERE id = ? AND kind = 'goal'", when, id).execute(&app.db).await?;
    store::record(&app.db, id, ctx.user_id, if when.is_some() { "achieved" } else { "reopened" }, json!({})).await?;
    Ok(Redirect::to(&format!("/goals/{id}#top")).into_response())
}
