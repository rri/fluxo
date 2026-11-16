//! Database snapshots: `fluxo backup PATH` (used before every deploy) and a nightly copy kept in
//! `<data_dir>/backups`.

use std::path::{Path, PathBuf};
use std::time::Duration;

use sqlx::SqlitePool;

use crate::{Result, now};

/// A consistent copy of the live database at `to` (SQLite `VACUUM INTO`), safe while serving.
pub async fn snapshot(db: &SqlitePool, to: &Path) -> Result<()> {
    if to.exists() {
        return Err(format!("{} already exists", to.display()).into());
    }
    if let Some(dir) = to.parent().filter(|d| !d.as_os_str().is_empty()) {
        std::fs::create_dir_all(dir)?;
    }
    sqlx::query("VACUUM INTO ?").bind(to.to_string_lossy().to_string()).execute(db).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(to, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn nightly_name(unix: i64) -> String {
    let (y, m, d) = crate::util::civil(unix);
    format!("nightly-{y:04}{m:02}{d:02}.db")
}

/// Take today's snapshot if it's missing, then keep only the newest `keep` nightly copies.
pub async fn nightly(db: &SqlitePool, dir: &Path, keep: usize) -> Result<Option<PathBuf>> {
    let path = dir.join(nightly_name(now()));
    let taken = if path.exists() {
        None
    } else {
        snapshot(db, &path).await?;
        Some(path)
    };
    let mut old: Vec<PathBuf> = std::fs::read_dir(dir)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| n.starts_with("nightly-") && n.ends_with(".db"))
        })
        .collect();
    old.sort();
    let excess = old.len().saturating_sub(keep.max(1));
    for p in old.into_iter().take(excess) {
        std::fs::remove_file(p)?;
    }
    Ok(taken)
}

/// Check every hour; take one snapshot a day (UTC).
pub fn spawn_nightly(db: SqlitePool, dir: PathBuf, keep: usize) {
    tokio::spawn(async move {
        loop {
            match nightly(&db, &dir, keep).await {
                Ok(Some(p)) => tracing::info!(path = %p.display(), "nightly snapshot taken"),
                Ok(None) => {}
                Err(e) => tracing::warn!(error = %e.0, "nightly snapshot failed"),
            }
            tokio::time::sleep(Duration::from_secs(3600)).await;
        }
    });
}
