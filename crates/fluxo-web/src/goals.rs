//! Goals: the Goals page, creating goals, a goal's own page, and the goal-only actions (statement,
//! coverage and privacy, marking achieved, adding steps).

use std::collections::HashMap;

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
    /// "P50 Feb 12 · P85 Mar 3", "nebulous", or empty.
    pub forecast: String,
    pub critical_now: String,
}

#[derive(Template)]
#[template(path = "goals.html")]
struct GoalsPage<'a> {
    theme: &'a str,
    focus_count: usize,
    nav: &'a str,
    csrf: &'a str,
    username: &'a str,
    initials: String,
    goals: Vec<GoalCard>,
    unattached: Vec<(String, String, String)>, // href, title, meta
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

fn card(world: &World, user: Id, g: &Node, f: &crate::forecast::Forecasts, cals: &HashMap<Id, fluxo_core::calendar::Calendar>) -> GoalCard {
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
        forecast: match (g.achieved_at, g.coverage.as_deref(), f.by_node.get(&g.id)) {
            (Some(_), _, _) => String::new(),
            (_, Some("low"), _) => "nebulous".into(),
            (_, _, Some(nf)) if !kids.is_empty() => {
                let cal = g.owner_id.and_then(|o| cals.get(&o).cloned()).unwrap_or_default();
                let d = crate::forecast::dated(nf, &cal);
                format!("P50 {} · P85 {}", d.p50, d.p85)
            }
            _ => String::new(),
        },
        critical_now: f
            .by_node
            .get(&g.id)
            .and_then(|nf| {
                // Most critical first; among equals, one that can start now (everything it needs
                // is done), then the earliest added.
                let ready = |k: &Node| world.needs_of(k.id).iter().all(|d| world.nodes.get(d).is_none_or(|n| n.is_done()));
                kids.iter()
                    .filter(|k| !k.is_done())
                    .filter_map(|k| nf.criticality.get(&k.id).map(|c| (*c, ready(k), std::cmp::Reverse(k.created_at), k)))
                    .max_by(|a, b| a.0.total_cmp(&b.0).then(a.1.cmp(&b.1)).then(a.2.cmp(&b.2)))
                    .map(|(.., k)| k.title.clone())
            })
            .unwrap_or_default(),
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
    let forecasts = crate::forecast::current(&app, &world).await?;
    let cals = crate::forecast::calendars(&app.db).await?;
    let goals = tops.iter().filter(|n| n.archived_at.is_none()).map(|g| card(&world, ctx.user_id, g, &forecasts, &cals)).collect();
    // Tasks attached to no goal that this person can work on, so they're easy to find and place.
    let loose = |n: &&Node| !n.is_goal() && world.parents.get(&n.id).is_none_or(Vec::is_empty);
    let mut unattached: Vec<&Node> = world
        .nodes
        .values()
        .filter(loose)
        .filter(|n| n.archived_at.is_none() && world.access(ctx.user_id, n.id).can_contribute())
        .collect();
    unattached.sort_by_key(|n| (n.created_at, n.id));
    let unattached = unattached
        .into_iter()
        .map(|n| (n.href(), n.title.clone(), n.estimate_days.map(views::days).unwrap_or_default()))
        .collect();
    let mut archived: Vec<(String, String)> =
        tops.iter().filter(|n| n.archived_at.is_some()).map(|g| (g.href(), g.title.clone())).collect();
    archived.extend(
        world
            .nodes
            .values()
            .filter(loose)
            .filter(|n| n.archived_at.is_some() && world.access(ctx.user_id, n.id).is_owner())
            .map(|n| (n.href(), n.title.clone())),
    );
    page(GoalsPage {
        focus_count: crate::updates::badge(&app.db, ctx.user_id).await?,
        theme: &ctx.theme,
        nav: "goals",
        csrf: &ctx.session.csrf_token,
        username: &ctx.username,
        initials: initials(&ctx.username),
        goals,
        unattached,
        archived,
    })
}

// ---------------------------------------------------------------- new goal

#[derive(Template)]
#[template(path = "goal_new.html")]
struct NewGoalPage<'a> {
    theme: &'a str,
    focus_count: usize,
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
        focus_count: crate::updates::badge(&app.db, ctx.user_id).await?,
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
            focus_count: crate::updates::badge(&app.db, ctx.user_id).await?,
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
    people: crate::people::PeopleView,
    done: Option<&'static str>,
    history: Vec<crate::updates::Line>,
    theme: &'a str,
    focus_count: usize,
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
    band: Option<Band>,
    gap: String,
    /// Goals beneath (or this one) with no steps and no rough size.
    unsized_goals: Vec<Crumb>,
    /// This goal's own rough size, for the form shown while it has no steps.
    size_min: String,
    size_max: String,
}

/// The forecast at the top of a goal page.
pub struct Band {
    pub nebulous: bool,
    pub p50: String,
    pub p85: String,
    pub spread: String,
    pub exposure: String,
    pub runs: String,
    pub updated: String,
    pub chart: String,
    pub calendar_owner: String,
}

#[derive(Deserialize)]
pub struct PageQuery {
    pub(crate) error: Option<String>,
    pub(crate) done: Option<String>,
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
    let forecasts = crate::forecast::current(&app, &world).await?;
    let fc = forecasts.by_node.get(&id).filter(|_| goal.achieved_at.is_none() && goal.archived_at.is_none());
    let steps = views::steps(&world, ctx.user_id, id, fc.map(|f| &f.criticality));
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
    let (path_risks, exposure) = views::path_risks(&app.db, &world, ctx.user_id, id).await?;
    let nebulous = goal.coverage.as_deref() == Some("low");
    let empty = world.children_of(id).is_empty();
    let unsized_goals: Vec<Crumb> = views::unsized_below(&world, ctx.user_id, id).into_iter().filter(|c| c.href != goal.href()).collect();
    let band = match fc.filter(|_| !empty || goal.size.is_some()) {
        Some(f) => {
            let cals = crate::forecast::calendars(&app.db).await?;
            let cal = goal.owner_id.and_then(|o| cals.get(&o).cloned()).unwrap_or_default();
            let d = crate::forecast::dated(f, &cal);
            Some(Band {
                nebulous,
                p50: d.p50,
                p85: d.p85,
                spread: d.spread_days,
                exposure: views::days(exposure),
                runs: format!("{} runs", crate::forecast::RUNS.to_string().as_bytes().rchunks(3).rev().map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(",")),
                updated: crate::util::ago(forecasts.computed_at, crate::now()).to_lowercase().replace("active now", "just now"),
                chart: views::histogram_svg(&f.bins, f.max, f.p50, f.p85),
                calendar_owner: match views::owner_label(goal.owner_id, ctx.user_id, &names).as_str() {
                    "you" => "your".to_string(),
                    "no one yet" => "the default (Mon–Fri, UTC)".to_string(),
                    name => format!("{name}’s"),
                },
            })
        }
        None => None,
    };
    // Coverage short of full adds unknown work after the known steps (D19).
    let gap = match (fc, goal.coverage.as_deref()) {
        (Some(f), Some(c @ ("partial" | "low"))) if !steps.is_empty() => {
            let share = if c == "partial" { 0.25 } else { 0.75 };
            let known = f.p50 / (1.0 + share / 2.0);
            format!("up to {}", views::days((known * share).max(0.5)))
        }
        _ => String::new(),
    };
    let error = views::error_text(q.error.as_deref());
    let people = crate::people::view(&app.db, &world, ctx.user_id, id, access).await?;
    let history = views::history(&app.db, &world, ctx.user_id, id).await?;
    page(GoalPage {
        people,
        history,
        done: views::done_text(q.done.as_deref()),
        focus_count: crate::updates::badge(&app.db, ctx.user_id).await?,
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
        band,
        gap,
        unsized_goals,
        size_min: goal.size.map(|s| views::days(s.0).trim_end_matches('d').to_string()).unwrap_or_default(),
        size_max: goal.size.map(|s| views::days(s.1).trim_end_matches('d').to_string()).unwrap_or_default(),
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
    #[serde(default)]
    size_min: String,
    #[serde(default)]
    size_max: String,
}

/// An optional rough size: both ends blank means none; otherwise both must be valid, smallest first.
fn parse_size(min: &str, max: &str) -> Result<Option<(f64, f64)>, ()> {
    if min.trim().is_empty() && max.trim().is_empty() {
        return Ok(None);
    }
    match (views::parse_days(min), views::parse_days(max)) {
        (Some(a), Some(b)) if a <= b => Ok(Some((a, b))),
        _ => Err(()),
    }
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
        let Ok(size) = parse_size(&f.size_min, &f.size_max) else {
            return Ok(Redirect::to(&format!("/goals/{id}?error=size#add")).into_response());
        };
        let new = insert_node(&app, ctx.user_id, "goal", title, "", Some(id), None, Some(coverage)).await?;
        if let Some((lo, hi)) = size {
            sqlx::query!("UPDATE nodes SET size_min_days = ?, size_max_days = ? WHERE id = ?", lo, hi, new).execute(&app.db).await?;
        }
        new
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

#[derive(Deserialize)]
pub struct SizeForm {
    csrf: String,
    #[serde(default)]
    min: String,
    #[serde(default)]
    max: String,
}

/// Set or clear a goal's rough size (a path estimate, so Contributors can change it).
pub async fn size(State(app): State<Shared>, jar: CookieJar, Path(id): Path<Id>, Form(f): Form<SizeForm>) -> Result<Response> {
    let (ctx, world, _) = gate!(app, jar, f.csrf, id, Access::Contribute);
    if !world.nodes[&id].is_goal() {
        return Ok(not_found());
    }
    let Ok(size) = parse_size(&f.min, &f.max) else {
        return Ok(Redirect::to(&format!("/goals/{id}?error=size#path")).into_response());
    };
    let (lo, hi) = (size.map(|s| s.0), size.map(|s| s.1));
    sqlx::query!("UPDATE nodes SET size_min_days = ?, size_max_days = ? WHERE id = ?", lo, hi, id).execute(&app.db).await?;
    store::record(&app.db, id, ctx.user_id, "size_changed", json!({ "min": lo, "max": hi })).await?;
    Ok(Redirect::to(&format!("/goals/{id}#path")).into_response())
}
