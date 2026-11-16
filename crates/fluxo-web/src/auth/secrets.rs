//! Encryption of secrets stored in the database (D10).
//!
//! Authenticator secrets are sealed with XChaCha20-Poly1305 under a 32-byte key that lives outside
//! the database (`FLUXO_KEY_FILE`). The stored value is `nonce (24 bytes) || ciphertext`. A label
//! such as `totp:42` is bound in as associated data, so a sealed value copied to another user's
//! row fails to open.

use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use rand::Rng;

const NONCE: usize = 24;

#[derive(Clone)]
pub struct SecretBox {
    cipher: XChaCha20Poly1305,
}

#[derive(Debug, thiserror::Error)]
pub enum SecretError {
    #[error("the key must be exactly 32 bytes")]
    BadKey,
    #[error("sealed value is corrupt, or was sealed with a different key or label")]
    Open,
}

impl SecretBox {
    pub fn new(key: &[u8]) -> Result<Self, SecretError> {
        let cipher = XChaCha20Poly1305::new_from_slice(key).map_err(|_| SecretError::BadKey)?;
        Ok(SecretBox { cipher })
    }

    /// A fresh random key, for first-run setup and tests.
    pub fn generate_key() -> [u8; 32] {
        let mut key = [0u8; 32];
        rand::rng().fill_bytes(&mut key);
        key
    }

    pub fn seal(&self, label: &str, plaintext: &[u8]) -> Vec<u8> {
        let mut nonce = [0u8; NONCE];
        rand::rng().fill_bytes(&mut nonce);
        let sealed = self
            .cipher
            .encrypt(&XNonce::from(nonce), Payload { msg: plaintext, aad: label.as_bytes() })
            .expect("encryption with a valid key does not fail");
        let mut out = nonce.to_vec();
        out.extend(sealed);
        out
    }

    pub fn open(&self, label: &str, stored: &[u8]) -> Result<Vec<u8>, SecretError> {
        if stored.len() < NONCE {
            return Err(SecretError::Open);
        }
        let (nonce, sealed) = stored.split_at(NONCE);
        let nonce: [u8; NONCE] = nonce.try_into().expect("split at NONCE");
        self.cipher
            .decrypt(&XNonce::from(nonce), Payload { msg: sealed, aad: label.as_bytes() })
            .map_err(|_| SecretError::Open)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seal_open_and_tamper() {
        let b = SecretBox::new(&SecretBox::generate_key()).unwrap();
        let s = b.seal("totp:1", b"secret");
        assert_eq!(b.open("totp:1", &s).unwrap(), b"secret");
        assert!(b.open("totp:2", &s).is_err(), "label is bound in");
        let mut t = s.clone();
        *t.last_mut().unwrap() ^= 1;
        assert!(b.open("totp:1", &t).is_err(), "tampering is detected");
        let other = SecretBox::new(&SecretBox::generate_key()).unwrap();
        assert!(other.open("totp:1", &s).is_err(), "another key can't open it");
        assert!(SecretBox::new(&[0u8; 16]).is_err());
    }
}
