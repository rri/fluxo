//! Reading fluxo offline (G20): the service worker, the page it shows for pages that weren't
//! saved, and the list of pages it keeps on the device.
//!
//! Nothing here changes data. Writing stays online only: offline, pages are read-only copies.

use std::sync::LazyLock;

use axum::Json;
use axum::extract::State;
use axum::http::header;
use axum::response::{Html, IntoResponse, Response};
use axum_extra::extract::cookie::CookieJar;
use fluxo_core::access::Access;
use sha2::{Digest, Sha256};

use crate::store::World;
use crate::{ASSETS, Result, Shared, error_page, require_user};

/// At most this many goal and task pages are kept on a device, besides Focus and Goals.
const MAX_ITEMS: usize = 300;

const WORKER: &str = include_str!("../static/sw.js");

/// Changes whenever the worker or any static file does, so browsers install the new worker and
/// replace their saved copies of the static files.
static BUILD: LazyLock<String> = LazyLock::new(|| {
    let mut h = Sha256::new();
    h.update(WORKER.as_bytes());
    for (name, body, _) in ASSETS {
        h.update(name.as_bytes());
        h.update(body);
    }
    h.finalize().iter().take(6).map(|b| format!("{b:02x}")).collect()
});

/// `/sw.js`, served from the root so it may look after every page.
pub async fn service_worker() -> Response {
    let body = WORKER.replace("__BUILD__", &BUILD);
    ([(header::CONTENT_TYPE, "text/javascript; charset=utf-8")], body).into_response()
}

/// What the worker shows for a page it hasn't saved while there's no connection.
pub async fn offline_page() -> Html<String> {
    Html(error_page(
        "Not saved on this device",
        "You’re offline, and this page wasn’t saved for reading offline. Focus, Goals, the goals \
         and tasks you own, contribute to or watch, and pages you opened in the last 30 days are.",
    ))
}

/// The pages to keep on the signed-in person's device: Focus, Goals, and every goal and task they
/// own, contribute to or watch, including everything beneath those (as Updates count them).
/// Archived items and ones they can't read are left out.
pub async fn pages(State(app): State<Shared>, jar: CookieJar) -> Result<Response> {
    let ctx = match require_user(&app, &jar).await? {
        Ok(c) => c,
        Err(r) => return Ok(r),
    };
    let world = World::load(&app.db).await?;
    let mut items: Vec<_> = world
        .nodes
        .values()
        .filter(|n| n.archived_at.is_none())
        .filter(|n| world.role(ctx.user_id, n.id) >= Access::Watch && world.access(ctx.user_id, n.id).can_read())
        .collect();
    items.sort_by_key(|n| (!n.is_goal(), n.id));
    let pages: Vec<String> = ["/".to_string(), "/goals".to_string()]
        .into_iter()
        .chain(items.iter().take(MAX_ITEMS).map(|n| n.href()))
        .collect();
    Ok(Json(serde_json::json!({ "pages": pages })).into_response())
}
