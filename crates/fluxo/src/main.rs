//! The fluxo server.
//!
//! ```text
//! fluxo [serve]            run the server (the default)
//! fluxo backup PATH        copy the database to PATH (a consistent snapshot; safe while running)
//! fluxo --version          print the version and the commit it was built from
//! ```
//!
//! Configuration comes from a TOML file (decided 2026-11-16): `$FLUXO_CONFIG`, else
//! `/etc/fluxo/fluxo.toml` if it exists, else built-in defaults for local use. Environment
//! variables override individual settings:
//!
//! | Setting           | Environment             | Default                    |
//! |-------------------|-------------------------|----------------------------|
//! | `listen`          | `FLUXO_LISTEN`          | `127.0.0.1:8080`           |
//! | `public_url`      | `FLUXO_PUBLIC_URL`      | none (links use the Host)  |
//! | `data_dir`        | `FLUXO_DATA_DIR`        | `./data`                   |
//! | `key_file`        | `FLUXO_KEY_FILE`        | `<data_dir>/secret.key`    |
//! | `secure_cookies`  | `FLUXO_SECURE_COOKIES`  | `true`                     |
//! | `trusted_proxies` | `FLUXO_TRUSTED_PROXIES` | `["127.0.0.1", "::1"]`     |
//! | `backup_keep`     | `FLUXO_BACKUP_KEEP`     | `7` nightly snapshots      |
//! | `setup_token`     | `FLUXO_SETUP_TOKEN`     | random each start          |
//!
//! If the key file doesn't exist and wasn't named explicitly, a new key is created on first start.
//! Back it up separately from the database: without it, 2FA secrets can't be read.

use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::Arc;
use std::time::Duration;

use fluxo_web::auth::secrets::SecretBox;
use fluxo_web::{AppState, MIGRATOR, router};
use serde::Deserialize;
use sqlx::SqlitePool;
use sqlx::sqlite::{SqliteConnectOptions, SqliteJournalMode, SqlitePoolOptions};

type BoxError = Box<dyn std::error::Error>;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const COMMIT: Option<&str> = option_env!("FLUXO_COMMIT");

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct Config {
    listen: Option<String>,
    public_url: Option<String>,
    data_dir: Option<PathBuf>,
    key_file: Option<PathBuf>,
    secure_cookies: Option<bool>,
    trusted_proxies: Option<Vec<IpAddr>>,
    backup_keep: Option<usize>,
    setup_token: Option<String>,
}

struct Settings {
    listen: String,
    public_url: Option<String>,
    data_dir: PathBuf,
    key_file: Option<PathBuf>,
    secure_cookies: bool,
    trusted_proxies: Vec<IpAddr>,
    backup_keep: usize,
    setup_token: Option<String>,
}

fn env(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| !v.is_empty())
}

fn load_settings() -> Result<Settings, BoxError> {
    let path = env("FLUXO_CONFIG").map(PathBuf::from).or_else(|| {
        let p = PathBuf::from("/etc/fluxo/fluxo.toml");
        p.exists().then_some(p)
    });
    let c: Config = match &path {
        Some(p) => toml::from_str(&std::fs::read_to_string(p).map_err(|e| format!("can't read {}: {e}", p.display()))?)
            .map_err(|e| format!("{}: {e}", p.display()))?,
        None => Config::default(),
    };
    let flag = |s: String| s != "false" && s != "0";
    Ok(Settings {
        listen: env("FLUXO_LISTEN").or(c.listen).unwrap_or_else(|| "127.0.0.1:8080".into()),
        public_url: env("FLUXO_PUBLIC_URL").or(c.public_url).map(|u| u.trim_end_matches('/').to_string()),
        data_dir: env("FLUXO_DATA_DIR").map(PathBuf::from).or(c.data_dir).unwrap_or_else(|| "data".into()),
        key_file: env("FLUXO_KEY_FILE").map(PathBuf::from).or(c.key_file),
        secure_cookies: env("FLUXO_SECURE_COOKIES").map(flag).or(c.secure_cookies).unwrap_or(true),
        trusted_proxies: match env("FLUXO_TRUSTED_PROXIES") {
            Some(v) => v.split(',').map(|s| s.trim().parse()).collect::<Result<_, _>>()?,
            None => c
                .trusted_proxies
                .unwrap_or_else(|| vec![IpAddr::from([127, 0, 0, 1]), IpAddr::from([0u16, 0, 0, 0, 0, 0, 0, 1])]),
        },
        backup_keep: env("FLUXO_BACKUP_KEEP").map(|v| v.parse()).transpose()?.or(c.backup_keep).unwrap_or(7),
        setup_token: env("FLUXO_SETUP_TOKEN").or(c.setup_token),
    })
}

fn load_key(s: &Settings) -> Result<Vec<u8>, BoxError> {
    if let Some(path) = &s.key_file {
        return Ok(std::fs::read(path).map_err(|e| format!("can't read key_file {}: {e}", path.display()))?);
    }
    let path = s.data_dir.join("secret.key");
    if path.exists() {
        return Ok(std::fs::read(&path)?);
    }
    let key = SecretBox::generate_key();
    write_private(&path, &key)?;
    tracing::warn!(path = %path.display(), "created a new encryption key; back it up separately from the database");
    Ok(key.to_vec())
}

#[cfg(unix)]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(path)?;
    f.write_all(bytes)
}

#[cfg(not(unix))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::write(path, bytes)
}

async fn open_db(data_dir: &Path) -> Result<SqlitePool, BoxError> {
    std::fs::create_dir_all(data_dir)?;
    let options = SqliteConnectOptions::from_str(&format!("sqlite://{}", data_dir.join("fluxo.db").display()))?
        .create_if_missing(true)
        .journal_mode(SqliteJournalMode::Wal)
        .foreign_keys(true)
        .busy_timeout(Duration::from_secs(5));
    let db = SqlitePoolOptions::new().max_connections(8).connect_with(options).await?;
    MIGRATOR.run(&db).await?;
    Ok(db)
}

#[tokio::main]
async fn main() -> Result<(), BoxError> {
    tracing_subscriber::fmt()
        .with_env_filter(tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()))
        .init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--version" | "-V" | "version") => {
            println!("fluxo {VERSION} ({})", COMMIT.unwrap_or("local build"));
            Ok(())
        }
        Some("backup") => {
            let Some(to) = args.get(1) else { return Err("usage: fluxo backup PATH".into()) };
            let s = load_settings()?;
            let db = open_db(&s.data_dir).await?;
            fluxo_web::backup::snapshot(&db, Path::new(to)).await.map_err(|e| e.0)?;
            println!("{to}");
            Ok(())
        }
        None | Some("serve") => serve().await,
        Some(other) => {
            Err(format!("unknown command {other:?}; try `fluxo serve`, `fluxo backup PATH` or `fluxo --version`")
                .into())
        }
    }
}

async fn serve() -> Result<(), BoxError> {
    let s = load_settings()?;
    let db = open_db(&s.data_dir).await?;
    let secrets = SecretBox::new(&load_key(&s)?)?;
    let mut state = AppState::new(db.clone(), secrets, s.secure_cookies);
    state.public_url = s.public_url.clone();
    state.trusted_proxies = s.trusted_proxies.clone();
    if let Some(t) = &s.setup_token {
        state.setup_token = t.clone();
    }
    let state = Arc::new(state);

    // Until the first Administrator exists, only someone who can read this log can create one.
    if sqlx::query_scalar::<_, Option<i64>>("SELECT setup_completed_at FROM instance WHERE id = 1")
        .fetch_one(&db)
        .await?
        .is_none()
    {
        let base = s.public_url.clone().unwrap_or_else(|| format!("http://{}", s.listen));
        tracing::warn!("Create the Administrator account: {base}/setup?token={}", state.setup_token);
    }

    fluxo_web::slips::spawn(state.clone()); // daily "dates slipping" check
    fluxo_web::session::spawn_purge(db.clone()); // idle sessions and old sign-in failures
    fluxo_web::backup::spawn_nightly(db, s.data_dir.join("backups"), s.backup_keep);

    let app = router(state);
    let listener = tokio::net::TcpListener::bind(&s.listen).await?;
    tracing::info!(listen = %s.listen, data = %s.data_dir.display(), "fluxo {VERSION} is listening");
    axum::serve(listener, app.into_make_service_with_connect_info::<SocketAddr>())
        .with_graceful_shutdown(async {
            let _ = tokio::signal::ctrl_c().await;
        })
        .await?;
    Ok(())
}
