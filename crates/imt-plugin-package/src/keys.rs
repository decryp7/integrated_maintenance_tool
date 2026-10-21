//! Publisher keys (Ed25519). A publisher signs the package digest ([`crate::digest`]) with its secret key; IMT
//! verifies with the public key in the package's `signature` file and trusts the package when that key is one of
//! its trusted keys ([`crate::trust`]).
//!
//! Text forms (one line each, so they can be pasted into Settings or kept in a file):
//!
//! - public key: `imt-plugin-public-key ed25519 <64 hex digits>`;
//! - secret key: `imt-plugin-secret-key ed25519 <64 hex digits of the seed>` (keep it private);
//! - key id: the first 16 hex digits of the SHA-256 of the public key.

use crate::PackageError;
use sha2::{Digest, Sha256};

const PUBLIC_PREFIX: &str = "imt-plugin-public-key ed25519 ";
const SECRET_PREFIX: &str = "imt-plugin-secret-key ed25519 ";

/// Lowercase hexadecimal.
pub fn to_hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Bytes from hexadecimal of exactly `N` bytes.
pub fn from_hex<const N: usize>(s: &str) -> Option<[u8; N]> {
    let s = s.trim();
    if s.len() != N * 2 || !s.is_ascii() {
        return None;
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

/// A publisher's public key.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PublicKey(pub [u8; 32]);

impl PublicKey {
    /// Reads the text form, or 64 bare hex digits.
    pub fn parse(text: &str) -> Result<PublicKey, PackageError> {
        let t = text.trim();
        let hex = t.strip_prefix(PUBLIC_PREFIX).unwrap_or(t);
        from_hex::<32>(hex).map(PublicKey).ok_or_else(|| {
            PackageError::InvalidKey("not a public key: expected \"imt-plugin-public-key ed25519 \" and 64 hexadecimal digits".into())
        })
    }

    /// The text form.
    pub fn to_text(&self) -> String {
        format!("{PUBLIC_PREFIX}{}", to_hex(&self.0))
    }

    /// The key's id: the first 16 hex digits of the SHA-256 of the key.
    pub fn id(&self) -> String {
        to_hex(&Sha256::digest(self.0)[..8])
    }

    /// Whether `signature` is this key's signature of `message`.
    pub fn verifies(&self, message: &[u8], signature: &[u8; 64]) -> bool {
        let key = ed25519_compact::PublicKey::new(self.0);
        key.verify(message, &ed25519_compact::Signature::new(*signature)).is_ok()
    }
}

/// A publisher's secret key (the seed of an Ed25519 key pair). Its `Debug` hides the bytes.
#[derive(Clone, PartialEq, Eq)]
pub struct SecretKey([u8; 32]);

impl std::fmt::Debug for SecretKey {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SecretKey({})", self.public().id())
    }
}

impl SecretKey {
    /// A new random key.
    pub fn generate() -> Result<SecretKey, PackageError> {
        let mut seed = [0u8; 32];
        getrandom::fill(&mut seed).map_err(|e| PackageError::InvalidKey(format!("no random numbers available: {e}")))?;
        Ok(SecretKey(seed))
    }

    /// Reads the text form.
    pub fn parse(text: &str) -> Result<SecretKey, PackageError> {
        text.trim()
            .strip_prefix(SECRET_PREFIX)
            .and_then(from_hex::<32>)
            .map(SecretKey)
            .ok_or_else(|| {
                PackageError::InvalidKey("not a secret key: expected \"imt-plugin-secret-key ed25519 \" and 64 hexadecimal digits".into())
            })
    }

    /// The text form (write it to a private file).
    pub fn to_text(&self) -> String {
        format!("{SECRET_PREFIX}{}", to_hex(&self.0))
    }

    fn pair(&self) -> ed25519_compact::KeyPair {
        ed25519_compact::KeyPair::from_seed(ed25519_compact::Seed::new(self.0))
    }

    /// The matching public key.
    pub fn public(&self) -> PublicKey {
        PublicKey(*self.pair().pk)
    }

    /// The signature of `message` (deterministic).
    pub fn sign(&self, message: &[u8]) -> [u8; 64] {
        *self.pair().sk.sign(message, None)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_sign_verify_and_roundtrip_through_text() {
        let sk = SecretKey::generate().unwrap();
        let pk = sk.public();
        assert_eq!(SecretKey::parse(&sk.to_text()).unwrap(), sk);
        assert_eq!(PublicKey::parse(&pk.to_text()).unwrap(), pk);
        assert_eq!(PublicKey::parse(&to_hex(&pk.0)).unwrap(), pk);
        assert_eq!(pk.id().len(), 16);
        let sig = sk.sign(b"digest");
        assert!(pk.verifies(b"digest", &sig));
        assert!(!pk.verifies(b"digest!", &sig));
        let mut bad = sig;
        bad[0] ^= 1;
        assert!(!pk.verifies(b"digest", &bad));
        let other = SecretKey::generate().unwrap().public();
        assert!(!other.verifies(b"digest", &sig));
        assert_ne!(other.id(), pk.id());
        assert!(!format!("{sk:?}").contains(&to_hex(&sk.0)));
    }

    #[test]
    fn malformed_keys_are_refused() {
        for t in [
            "",
            "imt-plugin-public-key ed25519 zz",
            &"a".repeat(63),
            "imt-plugin-secret-key ed25519 00",
        ] {
            assert_eq!(PublicKey::parse(t).unwrap_err().code(), "invalidKey", "{t}");
        }
        assert!(SecretKey::parse(&to_hex(&[1u8; 32])).is_err(), "a secret key needs its prefix");
        assert_eq!(from_hex::<2>("0aff"), Some([0x0a, 0xff]));
        assert_eq!(from_hex::<2>("0afg"), None);
        assert_eq!(from_hex::<2>("é0"), None);
    }
}
