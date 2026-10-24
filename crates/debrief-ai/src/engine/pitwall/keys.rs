//! Where Debrief keeps its Pitwall key. The key is a secret (Pitwall ADR 0007): it lives only in the OS credential
//! store (the shell's adapter), never in a file, a log or the UI. Tests use [`MemoryKeys`], never the real store.

use std::sync::Mutex;

/// Longest key accepted (Pitwall's limit for credentials).
pub const MAX_KEY_LEN: usize = 256;

/// A usable key: 1 to 256 printable ASCII characters after trimming.
pub fn parse_key(raw: &str) -> Option<String> {
    let k = raw.trim();
    (!k.is_empty() && k.len() <= MAX_KEY_LEN && k.bytes().all(|b| b.is_ascii_graphic())).then(|| k.to_string())
}

/// The port of the credential store. Errors are plain sentences without the key.
pub trait KeyStore: Send + Sync {
    fn get(&self) -> Option<String>;
    fn set(&self, key: &str) -> Result<(), String>;
    /// Deleting a missing key succeeds.
    fn delete(&self) -> Result<(), String>;
}

/// An in-memory store (tests and runs without a credential store).
#[derive(Default)]
pub struct MemoryKeys(Mutex<Option<String>>);

impl MemoryKeys {
    pub fn with(key: &str) -> MemoryKeys {
        MemoryKeys(Mutex::new(Some(key.to_string())))
    }
}

impl KeyStore for MemoryKeys {
    fn get(&self) -> Option<String> {
        self.0.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn set(&self, key: &str) -> Result<(), String> {
        let k = parse_key(key).ok_or("the key is not usable")?;
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = Some(k);
        Ok(())
    }

    fn delete(&self) -> Result<(), String> {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = None;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_are_checked_and_kept() {
        assert_eq!(parse_key(" pw_abc \n").as_deref(), Some("pw_abc"));
        assert_eq!(parse_key(""), None);
        assert_eq!(parse_key("has space"), None);
        assert_eq!(parse_key(&"k".repeat(MAX_KEY_LEN + 1)), None);
        let m = MemoryKeys::default();
        assert_eq!(m.get(), None);
        assert!(m.set("bad key").is_err());
        m.set("pw_1").unwrap();
        assert_eq!(m.get().as_deref(), Some("pw_1"));
        m.delete().unwrap();
        m.delete().unwrap();
        assert_eq!(MemoryKeys::with("pw_2").get().as_deref(), Some("pw_2"));
    }
}
