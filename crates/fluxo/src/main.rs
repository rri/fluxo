//! The fluxo server.
//!
//! Configuration is read from environment variables:
//!
//! | Variable                | Default                   | Meaning                                            |
//! |-------------------------|---------------------------|----------------------------------------------------|
//! | `FLUXO_ADDR`            | `127.0.0.1:8080`          | Address to listen on (Caddy proxies to it, D12)    |
//! | `FLUXO_DATA_DIR`        | `./data`                  | Folder holding `fluxo.db`                          |
//! | `FLUXO_KEY_FILE`        | `$FLUXO_DATA_DIR/secret.key` | 32-byte key that encrypts 2FA secrets (D10)     |
//! | `FLUXO_SECURE_COOKIES`  | `true`                    | Set `false` only to test over plain HTTP off localhost |
//! | `RUST_LOG`              | `info`                    | Log filter                                         |
//!
//! If `FLUXO_KEY_FILE` isn't set and the default key file doesn't exist, a new key is created there
//! on first start. Back it up separately from the database: without it, 2FA secrets can't be read.

use std::path::PathBuf;
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use fluxo_web::auth::secrets::SecretBox;
use fluxo_web::{AppState, MIGRATOR, router};
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

type BoxError = Box<dyn std::error::Error>;

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn load_key(data_dir: &std::path::Path) -> Result<Vec<u8>, BoxError> {
    if let Some(path) = env("FLUXO_KEY_FILE") {
        let key = std::fs::read(&path).map_err(|e| format!("can't read FLUXO_KEY_FILE {path}: {e}"))?;
        return Ok(key);
    }
    let path = data_dir.join("secret.key");
    if path.exists() {
        return Ok(std::fs::read(&path)?);
    }
    let key = SecretBox::generate_key();
    write_private(&path, &key)?;
    tracing::warn!(path = %path.display(), "created a new encryption key; back it up separately from the database");
    Ok(key.to_vec())
}

#[cfg(unix)]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &std::path::Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();

    let addr = env("FLUXO_ADDR").unwrap_or_else(|| "127.0.0.1:8080".into());
    let data_dir = PathBuf::from(env("FLUXO_DATA_DIR").unwrap_or_else(|| "data".into()));
    let secure_cookies = env("FLUXO_SECURE_COOKIES").is_none_or(|v| v != "false" && v != "0");
    std::fs::create_dir_all(&data_dir)?;

    let secrets = SecretBox::new(&load_key(&data_dir)?)?;
    let db_path = data_dir.join("fluxo.db");
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", db_path.display()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let db = SqlitePoolOptions::new().max_connections(8).connect_with(options).await?;
    MIGRATOR.run(&db).await?;

    let state = Arc::new(AppState::new(db, secrets, secure_cookies));
    fluxo_web::slips::spawn(state.clone()); // daily "dates slipping" check
    let app = router(state);
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    tracing::info!(%addr, db = %db_path.display(), "fluxo is listening");
    axum::serve(listener, app)
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
