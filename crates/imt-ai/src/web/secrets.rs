//! Secrets live in the operating system's keychain (Windows Credential Manager, macOS Keychain, Secret
//! Service on Linux), never in a file IMT writes. Values are write-only for the UI: `get` is for the engine.

use crate::error::{AiError, Result};
use std::collections::HashMap;
use std::sync::Mutex;

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>>;
    fn set(&self, name: &str, value: &str) -> Result<()>;
    fn delete(&self, name: &str) -> Result<()>;
}

/// For tests and for sessions where no keychain is available.
#[derive(Default)]
pub struct MemorySecretStore {
    values: Mutex<HashMap<String, String>>,
}

impl MemorySecretStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl SecretStore for MemorySecretStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        Ok(self.values.lock().unwrap_or_else(|p| p.into_inner()).get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.values
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .insert(name.into(), value.into());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<()> {
        self.values.lock().unwrap_or_else(|p| p.into_inner()).remove(name);
        Ok(())
    }
}

pub struct KeyringSecretStore {
    service: String,
}

impl KeyringSecretStore {
    pub fn new(service: impl Into<String>) -> Self {
        KeyringSecretStore { service: service.into() }
    }

    fn entry(&self, name: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(&self.service, name).map_err(|e| AiError::Refused(format!("the system keychain is not available: {e}")))
    }
}

impl SecretStore for KeyringSecretStore {
    fn get(&self, name: &str) -> Result<Option<String>> {
        match self.entry(name)?.get_password() {
            Ok(v) => Ok(Some(v)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(AiError::Refused(format!("cannot read from the system keychain: {e}"))),
        }
    }
    fn set(&self, name: &str, value: &str) -> Result<()> {
        self.entry(name)?
            .set_password(value)
            .map_err(|e| AiError::Refused(format!("cannot save to the system keychain: {e}")))
    }
    fn delete(&self, name: &str) -> Result<()> {
        match self.entry(name)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(AiError::Refused(format!("cannot remove from the system keychain: {e}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_roundtrip() {
        let s = MemorySecretStore::new();
        assert_eq!(s.get("k").unwrap(), None);
        s.set("k", "v").unwrap();
        assert_eq!(s.get("k").unwrap().as_deref(), Some("v"));
        s.delete("k").unwrap();
        s.delete("k").unwrap();
        assert_eq!(s.get("k").unwrap(), None);
    }

    /// Needs a real keychain (may prompt on a developer machine): `cargo test -- --ignored keychain`.
    #[test]
    #[ignore]
    fn keychain_roundtrip() {
        let s = KeyringSecretStore::new("imt-ai-test");
        s.set("probe", "secret").unwrap();
        assert_eq!(s.get("probe").unwrap().as_deref(), Some("secret"));
        s.delete("probe").unwrap();
        assert_eq!(s.get("probe").unwrap(), None);
    }
}
