//! Which packages Debrief installs (ADR 0027):
//!
//! - signed by a **trusted key** (Debrief's built-in first-party key, or one the user added in Settings) → installed
//!   normally;
//! - **unsigned**, or signed by a key that is not trusted → only with **developer mode** on, with a warning;
//! - a signature that does not match (tampered) → never ([`crate::Package::open`] already refuses it).
//!
//! The decision is a pure function of the package's [`Signature`] and the [`TrustSettings`]; the app stores the
//! settings and asks.

use crate::archive::Signature;
use crate::keys::PublicKey;
use crate::PackageError;
use serde::{Deserialize, Serialize};

/// Debrief's first-party publisher keys (name, public key), trusted on every installation and not removable.
pub const FIRST_PARTY_KEYS: &[(&str, &str)] = &[(
    "Debrief (first-party)",
    "debrief-plugin-public-key ed25519 c7eda0a755f4e9ee1d47609b7b2447ff6e1b4e8507072ca16583227f2f1e074c",
)];

/// The format of [`TrustSettings`] on disk.
pub const TRUST_VERSION: u32 = 1;
/// Most keys a user may trust.
pub const MAX_TRUSTED_KEYS: usize = 64;

/// A key the user trusts, as stored.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StoredKey {
    /// Who the key belongs to, as the user named it.
    pub name: String,
    /// The public key, text form.
    pub public_key: String,
}

/// The user's trust settings: developer mode and the keys they added.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct TrustSettings {
    /// Unsigned packages and packages signed by unknown keys may be installed (with a warning).
    pub developer_mode: bool,
    /// Publisher keys the user trusts (besides [`FIRST_PARTY_KEYS`]).
    pub keys: Vec<StoredKey>,
}

/// A trusted key.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrustedKey {
    /// Who it belongs to.
    pub name: String,
    /// The key.
    pub key: PublicKey,
    /// One of Debrief's own keys (cannot be removed).
    pub first_party: bool,
}

/// How far a package that may be installed is trusted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// Signed by a trusted key.
    Trusted {
        /// The key's name.
        publisher_key: String,
    },
    /// Installable only because developer mode is on.
    Developer {
        /// What the user is warned about.
        warning: String,
    },
}

/// The signature status shown before installing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SignatureStatus {
    /// Signed by a trusted key (its name).
    Trusted(String),
    /// Signed by a key that is not trusted (its id).
    UnknownKey(String),
    /// Not signed.
    Unsigned,
}

impl SignatureStatus {
    /// The stable code (`trusted`, `unknownKey`, `unsigned`).
    pub fn code(&self) -> &'static str {
        match self {
            SignatureStatus::Trusted(_) => "trusted",
            SignatureStatus::UnknownKey(_) => "unknownKey",
            SignatureStatus::Unsigned => "unsigned",
        }
    }
}

impl TrustSettings {
    /// The first-party keys and the user's valid keys (an invalid stored key is skipped, not fatal).
    pub fn trusted_keys(&self) -> Vec<TrustedKey> {
        let mut out: Vec<TrustedKey> = FIRST_PARTY_KEYS
            .iter()
            .filter_map(|(name, text)| {
                PublicKey::parse(text).ok().map(|key| TrustedKey {
                    name: name.to_string(),
                    key,
                    first_party: true,
                })
            })
            .collect();
        for k in &self.keys {
            if let Ok(key) = PublicKey::parse(&k.public_key) {
                if !out.iter().any(|t| t.key == key) {
                    out.push(TrustedKey {
                        name: k.name.clone(),
                        key,
                        first_party: false,
                    });
                }
            }
        }
        out
    }

    /// Adds a key (checked: valid, named, not already trusted, within [`MAX_TRUSTED_KEYS`]). Returns its id.
    pub fn add_key(&mut self, name: &str, public_key: &str) -> Result<String, PackageError> {
        let name = name.trim();
        if name.is_empty() || name.chars().count() > 120 || name.chars().any(char::is_control) {
            return Err(PackageError::InvalidKey("name the key's publisher (1–120 characters)".into()));
        }
        let key = PublicKey::parse(public_key)?;
        if self.trusted_keys().iter().any(|t| t.key == key) {
            return Err(PackageError::InvalidKey(format!("key {} is already trusted", key.id())));
        }
        if self.keys.len() >= MAX_TRUSTED_KEYS {
            return Err(PackageError::InvalidKey(format!("at most {MAX_TRUSTED_KEYS} keys can be trusted")));
        }
        self.keys.push(StoredKey {
            name: name.to_string(),
            public_key: key.to_text(),
        });
        Ok(key.id())
    }

    /// Removes the user's key with id `key_id`; first-party keys cannot be removed. Returns whether one was.
    pub fn remove_key(&mut self, key_id: &str) -> bool {
        let before = self.keys.len();
        self.keys
            .retain(|k| PublicKey::parse(&k.public_key).map(|p| p.id() != key_id).unwrap_or(false));
        self.keys.len() != before
    }

    /// What the user sees about `signature`.
    pub fn status(&self, signature: &Signature) -> SignatureStatus {
        match signature {
            Signature::Unsigned => SignatureStatus::Unsigned,
            Signature::Signed { key } => match self.trusted_keys().into_iter().find(|t| t.key == *key) {
                Some(t) => SignatureStatus::Trusted(t.name),
                None => SignatureStatus::UnknownKey(key.id()),
            },
        }
    }

    /// Whether a package signed as `signature` may be installed, and how far it is trusted.
    pub fn decide(&self, signature: &Signature) -> Result<Trust, PackageError> {
        match self.status(signature) {
            SignatureStatus::Trusted(name) => Ok(Trust::Trusted { publisher_key: name }),
            SignatureStatus::Unsigned if self.developer_mode => Ok(Trust::Developer {
                warning: "This plugin is not signed: nobody vouches for who made it or that it was not changed. Install it only if you built it yourself or trust where it came from.".into(),
            }),
            SignatureStatus::UnknownKey(id) if self.developer_mode => Ok(Trust::Developer {
                warning: format!("This plugin is signed with key {id}, which you have not trusted. Install it only if you trust where it came from, or add the publisher's key in Settings."),
            }),
            SignatureStatus::Unsigned => Err(PackageError::Unsigned),
            SignatureStatus::UnknownKey(key_id) => Err(PackageError::UnknownKey { key_id }),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::keys::SecretKey;

    #[test]
    fn the_first_party_key_is_valid_and_always_trusted() {
        let keys = TrustSettings::default().trusted_keys();
        assert_eq!(keys.len(), FIRST_PARTY_KEYS.len(), "every first-party key parses");
        assert!(keys.iter().all(|k| k.first_party));
        let first = keys[0].key;
        let s = TrustSettings::default();
        assert_eq!(
            s.decide(&Signature::Signed { key: first }).unwrap(),
            Trust::Trusted {
                publisher_key: FIRST_PARTY_KEYS[0].0.into()
            }
        );
        let mut s = s;
        assert!(!s.remove_key(&first.id()), "first-party keys cannot be removed");
        assert!(s.add_key("again", &first.to_text()).is_err());
    }

    #[test]
    fn unsigned_and_unknown_keys_need_developer_mode() {
        let key = SecretKey::generate().unwrap().public();
        let signed = Signature::Signed { key };
        let mut s = TrustSettings::default();
        assert_eq!(s.decide(&Signature::Unsigned).unwrap_err(), PackageError::Unsigned);
        assert_eq!(s.decide(&signed).unwrap_err().code(), "unknownKey");
        assert_eq!(s.status(&signed), SignatureStatus::UnknownKey(key.id()));
        s.developer_mode = true;
        assert!(matches!(s.decide(&Signature::Unsigned).unwrap(), Trust::Developer { warning } if warning.contains("not signed")));
        assert!(matches!(s.decide(&signed).unwrap(), Trust::Developer { warning } if warning.contains(&key.id())));
        // Trusting the key makes it a normal install, with or without developer mode.
        s.developer_mode = false;
        assert_eq!(s.add_key("  Example Ltd ", &key.to_text()).unwrap(), key.id());
        assert_eq!(
            s.decide(&signed).unwrap(),
            Trust::Trusted {
                publisher_key: "Example Ltd".into()
            }
        );
        assert_eq!(s.status(&signed).code(), "trusted");
        assert!(s
            .add_key("Example", &key.to_text())
            .unwrap_err()
            .to_string()
            .contains("already trusted"));
        assert!(s.remove_key(&key.id()) && !s.remove_key(&key.id()));
        assert_eq!(s.decide(&signed).unwrap_err().code(), "unknownKey");
    }

    #[test]
    fn stored_keys_are_checked_and_bad_ones_skipped() {
        let mut s = TrustSettings::default();
        assert!(s.add_key("", &SecretKey::generate().unwrap().public().to_text()).is_err());
        assert!(s.add_key("x", "not a key").is_err());
        s.keys.push(StoredKey {
            name: "broken".into(),
            public_key: "garbage".into(),
        });
        assert_eq!(s.trusted_keys().len(), FIRST_PARTY_KEYS.len());
        for i in 0..MAX_TRUSTED_KEYS - 1 {
            s.add_key(&format!("k{i}"), &SecretKey::generate().unwrap().public().to_text())
                .unwrap();
        }
        assert!(s
            .add_key("one too many", &SecretKey::generate().unwrap().public().to_text())
            .is_err());
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(serde_json::from_str::<TrustSettings>(&json).unwrap(), s);
        assert_eq!(serde_json::from_str::<TrustSettings>("{}").unwrap(), TrustSettings::default());
    }
}
