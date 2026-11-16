//! Export (D23): everything the person can see, as JSON: goals and tasks with their links, risks
//! with likelihood history, and each item's history.

use axum::extract::State;
use axum::http::header;
use axum::response::{IntoResponse, Response};
use axum_extra::extract::cookie::CookieJar;
use serde_json::{Value, json};

use crate::store::{self, World};
use crate::{Result, Shared, now, require_user};

pub async fn download(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let world = World::load(&app.db).await?;
    let names = store::usernames(&app.db).await?;
    let name = |u: Option<i64>| u.and_then(|u| names.get(&u).cloned());
    let mut ids: Vec<i64> =
        world.nodes.keys().copied().filter(|id| world.access(ctx.user_id, *id).can_read()).collect();
    ids.sort_unstable();
    let visible: std::collections::HashSet<i64> = ids.iter().copied().collect();
    let events = crate::updates::recent(&app.db, 0).await?;
    let mut items = Vec::new();
    for id in &ids {
        let n = &world.nodes[id];
        let risks: Vec<Value> = store::risks(&app.db, *id)
            .await?
            .into_iter()
            .map(|r| json!({ "id": r.id, "title": r.title, "likelihood_pct": r.likelihood, "likelihood_history_pct": r.history, "impact_days": [r.min, r.max] }))
            .collect();
        let history: Vec<Value> = events
            .iter()
            .filter(|e| e.node == *id)
            .map(|e| json!({ "at": e.at, "by": name(e.actor), "kind": e.kind, "detail": e.detail }))
            .collect();
        items.push(json!({
            "id": n.id,
            "kind": n.kind,
            "title": n.title,
            "what": n.what,
            "why": n.why,
            "owner": name(n.owner_id),
            "home_goal": n.home_parent_id.filter(|p| visible.contains(p)),
            "feeds": world.parents.get(id).map(|ps| ps.iter().filter(|p| visible.contains(p)).collect::<Vec<_>>()).unwrap_or_default(),
            "needs": world.needs_of(*id).iter().filter(|d| visible.contains(d)).collect::<Vec<_>>(),
            "coverage": n.coverage,
            "rough_size_days": n.size.map(|s| [s.0, s.1]),
            "clarity": n.clarity,
            "estimate_days": n.estimate_days,
            "remaining_days": n.remaining_days,
            "status": n.status,
            "achieved_at": n.achieved_at,
            "private": n.private,
            "archived_at": n.archived_at,
            "created_at": n.created_at,
            "risks": risks,
            "history": history,
        }));
    }
    let doc = json!({
        "format": "fluxo-export-1",
        "exported_at": now(),
        "exported_by": ctx.username,
        "note": "Everything this person could see when exporting. Times are Unix seconds (UTC); durations are working days.",
        "items": items,
    });
    let (y, m, d) = crate::util::civil(now());
    let filename = format!("fluxo-export-{}-{y:04}{m:02}{d:02}.json", ctx.username);
    Ok((
        [
            (header::CONTENT_TYPE, "application/json".to_string()),
            (header::CONTENT_DISPOSITION, format!("attachment; filename=\"{filename}\"")),
        ],
        serde_json::to_string_pretty(&doc)?,
    )
        .into_response())
}
