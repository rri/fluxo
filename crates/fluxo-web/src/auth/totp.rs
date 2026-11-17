//! Authenticator (TOTP) codes: RFC 6238, SHA-1, 6 digits, 30-second steps, one step of clock
//! drift allowed either way (D7). A code is accepted at most once: the caller stores the step
//! returned by [`check`] and passes it back as `last_step` next time.

use rand::Rng;
use totp_rs::{Algorithm, TOTP};

pub const STEP: u64 = 30;
const SKEW: u64 = 1;

/// A new random 160-bit secret, the size RFC 4226 recommends.
pub fn generate_secret() -> Vec<u8> {
    let mut s = vec![0u8; 20];
    rand::rng().fill_bytes(&mut s);
    s
}

fn totp(secret: &[u8]) -> TOTP {
    labelled(secret, "")
}

fn labelled(secret: &[u8], account: &str) -> TOTP {
    TOTP::new(Algorithm::SHA1, 6, SKEW as u8, STEP, secret.to_vec(), Some("fluxo".into()), account.into())
        .expect("6 digits, a 160-bit secret and a username without ':' are valid")
}

/// What an authenticator app needs to add the account: a QR code (PNG, base64) and the same
/// secret as base32 text for typing in by hand, grouped in fours for readability.
pub fn provisioning(secret: &[u8], username: &str) -> (String, String) {
    let t = labelled(secret, username);
    let qr = t.get_qr_base64().expect("an otpauth URL this short always fits a QR code");
    let b32 = t.get_secret_base32();
    let grouped = b32.as_bytes().chunks(4).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join(" ");
    (qr, grouped)
}

/// The code an authenticator shows at `unix_time` (for tests).
pub fn code_at(secret: &[u8], unix_time: u64) -> String {
    totp(secret).generate(unix_time)
}

/// If `code` is valid at `unix_time` and newer than `last_step`, return the step it belongs to.
pub fn check(secret: &[u8], code: &str, unix_time: u64, last_step: Option<u64>) -> Option<u64> {
    let code = code.trim();
    if code.len() != 6 || !code.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    let t = totp(secret);
    let now = unix_time / STEP;
    let mut found = None;
    for step in now.saturating_sub(SKEW)..=now + SKEW {
        // Compare every candidate so timing doesn't depend on which step matched.
        if constant_time_eq(t.generate(step * STEP).as_bytes(), code.as_bytes()) {
            found = Some(step);
        }
    }
    found.filter(|&s| last_step.is_none_or(|last| s > last))
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepts_current_and_adjacent_steps_once() {
        let s = generate_secret();
        let now = 1_800_000_000;
        let code = code_at(&s, now);
        let step = check(&s, &code, now, None).expect("current code is valid");
        assert_eq!(step, now / STEP);
        assert_eq!(check(&s, &code, now, Some(step)), None, "a code can't be replayed");
        assert!(check(&s, &code_at(&s, now - STEP), now, None).is_some(), "30 s clock drift is tolerated");
        assert!(check(&s, &code_at(&s, now - 3 * STEP), now, None).is_none(), "old codes expire");
        assert!(check(&s, "12345", now, None).is_none());
        assert!(check(&s, "abcdef", now, None).is_none());
    }

    #[test]
    fn matches_rfc_6238_test_vector() {
        // RFC 6238 Appendix B, SHA-1, T = 59 s, last 6 digits of 94287082.
        assert_eq!(code_at(b"12345678901234567890", 59), "287082");
    }
}
