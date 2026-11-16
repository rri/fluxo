//! The daily check for dates slipping with no recorded progress (decided 2026-11-16).
//!
//! Forecasts count working days from the next working day, so if nothing changes, a goal's P85
//! *date* drifts later as time passes. Once a day the server recalculates every goal; when a P85
//! date has slipped 3 or more working days past the last one reported, followers get an Update.
//! Plan changes that move dates are reported by `forecast_moved` and reset this baseline.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use fluxo_core::calendar::{self, Calendar};
use sqlx::SqlitePool;

use crate::store::{Id, World};
use crate::{AppState, Result, now};

pub const SLIP_DAYS: i64 = 3;
const CHECK_EVERY: Duration = Duration::from_secs(6 * 3600);

fn owner_calendar(world: &World, goal: Id, cals: &HashMap<Id, Calendar>) -> Calendar {
    world.nodes.get(&goal).and_then(|n| n.owner_id).and_then(|o| cals.get(&o).cloned()).unwrap_or_default()
}

/// Record `p85_days` (as a date today) as the baseline for `goal`.
pub async fn reset_mark(
    db: &SqlitePool,
    world: &World,
    goal: Id,
    p85_days: f64,
    cals: &HashMap<Id, Calendar>,
) -> Result<()> {
    let cal = owner_calendar(world, goal, cals);
    let (t, date) = (now(), cal.finish_date(now(), p85_days).to_string());
    sqlx::query!(
        "INSERT INTO slip_marks (goal_id, p85_date, at) VALUES (?, ?, ?)
         ON CONFLICT (goal_id) DO UPDATE SET p85_date = excluded.p85_date, at = excluded.at",
        goal,
        date,
        t
    )
    .execute(db)
    .await?;
    Ok(())
}

/// One pass: recalculate, then report goals whose P85 date slipped `SLIP_DAYS`+ working days.
pub async fn check(app: &AppState) -> Result<usize> {
    let world = World::load(&app.db).await?;
    let f = crate::forecast::current(app, &world).await?;
    let cals = crate::forecast::calendars(&app.db).await?;
    let marks: HashMap<Id, String> = sqlx::query!("SELECT goal_id, p85_date FROM slip_marks")
        .fetch_all(&app.db)
        .await?
        .into_iter()
        .map(|r| (r.goal_id, r.p85_date))
        .collect();
    let mut reported = 0;
    for (id, nf) in &f.by_node {
        let Some(n) = world.nodes.get(id) else { continue };
        let forecastable = n.is_goal()
            && n.archived_at.is_none()
            && n.achieved_at.is_none()
            && n.coverage.as_deref() != Some("low")
            && (!world.children_of(*id).is_empty() || n.size.is_some());
        if !forecastable {
            continue;
        }
        let cal = owner_calendar(&world, *id, &cals);
        let now_date = cal.finish_date(now(), nf.p85);
        let Some(Ok(before)) = marks.get(id).map(|d| d.parse::<jiff::civil::Date>()) else {
            reset_mark(&app.db, &world, *id, nf.p85, &cals).await?;
            continue;
        };
        if cal.working_days_between(before, now_date) >= SLIP_DAYS {
            let year = cal.today(now()).year();
            let detail = serde_json::json!({
                "from": before.to_string(), "to": now_date.to_string(),
                "from_label": calendar::label(before, year), "to_label": calendar::label(now_date, year),
            })
            .to_string();
            let t = now();
            sqlx::query!(
                "INSERT INTO events (node_id, actor_id, kind, detail, at) VALUES (?, NULL, 'forecast_slipped', ?, ?)",
                id,
                detail,
                t
            )
            .execute(&app.db)
            .await?;
            reset_mark(&app.db, &world, *id, nf.p85, &cals).await?;
            reported += 1;
        }
    }
    Ok(reported)
}

/// Run [`check`] a few times a day for as long as the server runs.
pub fn spawn(app: Arc<AppState>) {
    tokio::spawn(async move {
        loop {
            match check(&app).await {
                Ok(n) if n > 0 => tracing::info!(goals = n, "reported slipping dates"),
                Ok(_) => {}
                Err(e) => tracing::warn!(error = %e.0, "slip check failed"),
            }
            tokio::time::sleep(CHECK_EVERY).await;
        }
    });
}
