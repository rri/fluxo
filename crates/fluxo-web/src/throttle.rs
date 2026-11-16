//! Slowing down password and recovery-code guessing: after too many failures in a window, for a
//! username or from an address, sign-in is refused until the window passes. Successful sign-ins
//! don't clear the count, so an attacker can't reset it by signing in to their own account.

use std::net::{IpAddr, SocketAddr};

use axum::extract::ConnectInfo;
use axum::http::HeaderMap;
use sqlx::SqlitePool;

use crate::{Result, now};

pub const WINDOW_SECS: i64 = 15 * 60;
/// Per username: generous for a person mistyping, far too few for guessing.
pub const PER_USERNAME: i64 = 10;
/// Per address: higher, since offices and phone networks share addresses.
pub const PER_IP: i64 = 30;

/// The visitor's address, worked out once per request by [`remember_ip`].
#[derive(Clone, Debug)]
pub struct ClientIp(pub String);

/// Middleware: note the visitor's address for handlers that need it.
pub async fn remember_ip(
    axum::extract::State(app): axum::extract::State<crate::Shared>,
    mut req: axum::extract::Request,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let ip = client_ip(req.extensions().get::<ConnectInfo<SocketAddr>>(), req.headers(), &app.trusted_proxies);
    req.extensions_mut().insert(ClientIp(ip));
    next.run(req).await
}

/// The visitor's address: the connection's, or the one Caddy reports when the connection comes
/// from a trusted proxy.
pub fn client_ip(conn: Option<&ConnectInfo<SocketAddr>>, headers: &HeaderMap, trusted: &[IpAddr]) -> String {
    let peer = conn.map(|c| c.0.ip());
    let forwarded = headers
        .get("x-forwarded-for")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(',').next_back())
        .and_then(|v| v.trim().parse::<IpAddr>().ok());
    match (peer, forwarded) {
        (Some(p), Some(f)) if trusted.contains(&p) => bucket(f),
        (Some(p), _) => bucket(p),
        (None, _) => "unknown".into(),
    }
}

/// IPv6 visitors usually control a whole /64, so they're counted by it, not one address.
fn bucket(ip: IpAddr) -> String {
    match ip {
        IpAddr::V4(v4) => v4.to_string(),
        IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
            Some(v4) => v4.to_string(),
            None => {
                let s = v6.segments();
                format!("{:x}:{:x}:{:x}:{:x}::/64", s[0], s[1], s[2], s[3])
            }
        },
    }
}

/// The outcome of checking a password someone typed to confirm an action.
#[derive(Debug, PartialEq, Eq)]
pub enum Check {
    Ok,
    Wrong,
    /// Too many recent failures for this account or address; nothing was checked.
    Blocked,
}

/// Check a password typed to confirm a sensitive action (or to sign in), counting failures
/// towards the same limits as sign-in. The hashing runs off the async threads.
pub async fn confirm_password(db: &SqlitePool, username: &str, ip: &str, stored: &str, given: &str) -> Result<Check> {
    if blocked(db, username, ip).await? {
        return Ok(Check::Blocked);
    }
    let (stored, given) = (stored.to_string(), given.to_string());
    let ok = tokio::task::spawn_blocking(move || crate::auth::password::verify(&given, &stored)).await?;
    if ok {
        return Ok(Check::Ok);
    }
    failed(db, username, ip).await?;
    Ok(Check::Wrong)
}

/// Is sign-in currently blocked for this username or address?
pub async fn blocked(db: &SqlitePool, username: &str, ip: &str) -> Result<bool> {
    let since = now() - WINDOW_SECS;
    let by_name =
        sqlx::query_scalar!("SELECT COUNT(*) FROM sign_in_failures WHERE username = ? AND at > ?", username, since)
            .fetch_one(db)
            .await?;
    let by_ip = sqlx::query_scalar!("SELECT COUNT(*) FROM sign_in_failures WHERE ip = ? AND at > ?", ip, since)
        .fetch_one(db)
        .await?;
    Ok(by_name >= PER_USERNAME || by_ip >= PER_IP)
}

pub async fn failed(db: &SqlitePool, username: &str, ip: &str) -> Result<()> {
    let t = now();
    let old = t - 86_400;
    sqlx::query!("DELETE FROM sign_in_failures WHERE at < ?", old).execute(db).await?;
    sqlx::query!("INSERT INTO sign_in_failures (username, ip, at) VALUES (?, ?, ?)", username, ip, t)
        .execute(db)
        .await?;
    Ok(())
}

pub const MESSAGE: &str =
    "Too many unsuccessful attempts. Wait 15 minutes and try again, or ask an Administrator for help.";

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addresses_are_bucketed() {
        assert_eq!(bucket("203.0.113.9".parse().unwrap()), "203.0.113.9");
        assert_eq!(bucket("2001:db8:1:2:aaaa::1".parse().unwrap()), "2001:db8:1:2::/64");
        assert_eq!(bucket("2001:db8:1:2:bbbb::7".parse().unwrap()), "2001:db8:1:2::/64");
        assert_eq!(bucket("::ffff:203.0.113.9".parse().unwrap()), "203.0.113.9");
    }
}
