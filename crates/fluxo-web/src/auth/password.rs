//! Password hashing with argon2id (D6).

use argon2::Argon2;
use argon2::password_hash::{PasswordHasher, PasswordVerifier, phc::PasswordHash};
use std::sync::LazyLock;

/// Minimum password length accepted at sign-up and on change.
pub const MIN_LENGTH: usize = 12;

pub fn hash(password: &str) -> String {
    Argon2::default()
        .hash_password(password.as_bytes())
        .expect("argon2 hashing with a random salt does not fail")
        .to_string()
}

/// Check `password` against a stored PHC string. Malformed hashes never verify.
pub fn verify(password: &str, stored: &str) -> bool {
    match PasswordHash::new(stored) {
        Ok(parsed) => Argon2::default().verify_password(password.as_bytes(), &parsed).is_ok(),
        Err(_) => false,
    }
}

/// A real hash to verify against when the username doesn't exist, so a wrong username takes
/// as long as a wrong password and response times don't reveal which accounts exist.
static DUMMY: LazyLock<String> = LazyLock::new(|| hash("fluxo dummy password for timing"));

pub fn burn_time(password: &str) {
    let _ = verify(password, &DUMMY);
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip() {
        let h = super::hash("correct horse battery staple");
        assert!(h.starts_with("$argon2id$"));
        assert!(super::verify("correct horse battery staple", &h));
        assert!(!super::verify("wrong", &h));
        assert!(!super::verify("anything", "not a hash"));
    }
}
