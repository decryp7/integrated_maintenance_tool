//! What the update checks keep between runs (`<config>/ai/updates.json`): per kind the last check, the
//! failures in a row and when to retry, and the versions the user skipped or was already told about.
//! Versioned; a missing, damaged or newer file gives an empty record (checks then simply run again).

use super::policy::{backoff_secs, UpdateKind};
use crate::error::{AiError, Result};
use serde::{Deserialize, Serialize};
use std::path::Path;

pub const RECORD_VERSION: u32 = 1;
/// Skipped or announced keys kept per kind (the oldest go first).
const MAX_KEYS: usize = 64;
const MAX_KEY_LEN: usize = 160;
const MAX_ERROR_CHARS: usize = 600;

/// Update keys are short ASCII words (`runtime:vulkan@b11700`, `model:qwen3.5-9b-q4km@0123456789ab`).
pub fn is_update_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= MAX_KEY_LEN
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, ':' | '@' | '.' | '-' | '_'))
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct KindRecord {
    /// Epoch seconds of the last check, successful or not.
    pub last_check: Option<i64>,
    pub last_success: Option<i64>,
    /// Why the last check or update failed (shown in Settings; never a notification).
    pub last_error: Option<String>,
    /// The last failure suggests downloading by hand (certificate interception, proxy sign-in, blocked host).
    pub manual_suggested: bool,
    pub failures: u32,
    /// Epoch seconds of the next attempt after a failure.
    pub next_retry: Option<i64>,
    /// "Skip this version".
    pub skipped: Vec<String>,
    /// Already announced ("Later" removes a key so the next check announces it again).
    pub announced: Vec<String>,
}

fn push_bounded(list: &mut Vec<String>, key: &str) -> bool {
    if list.iter().any(|k| k == key) {
        return false;
    }
    list.push(key.to_string());
    let excess = list.len().saturating_sub(MAX_KEYS);
    list.drain(..excess);
    true
}

fn bounded_error(e: &str) -> String {
    let mut s: String = e.chars().filter(|c| !c.is_control()).take(MAX_ERROR_CHARS).collect();
    if e.chars().count() > MAX_ERROR_CHARS {
        s.push('…');
    }
    s
}

impl KindRecord {
    pub fn checked(&mut self, now: i64) {
        self.last_check = Some(now);
        self.last_success = Some(now);
        self.failures = 0;
        self.next_retry = None;
        self.last_error = None;
        self.manual_suggested = false;
    }

    /// A check or update failed: remember why and retry after the backoff.
    pub fn failed(&mut self, now: i64, error: &str, manual_suggested: bool, interval_secs: u64) {
        self.last_check = Some(now);
        self.failures = self.failures.saturating_add(1);
        self.last_error = Some(bounded_error(error));
        self.manual_suggested = manual_suggested;
        self.next_retry = Some(now.saturating_add(backoff_secs(self.failures, interval_secs) as i64));
    }

    pub fn is_skipped(&self, key: &str) -> bool {
        self.skipped.iter().any(|k| k == key)
    }

    pub fn skip(&mut self, key: &str) {
        push_bounded(&mut self.skipped, key);
    }

    pub fn is_announced(&self, key: &str) -> bool {
        self.announced.iter().any(|k| k == key)
    }

    /// Marks `key` announced; false when it already was.
    pub fn announce(&mut self, key: &str) -> bool {
        push_bounded(&mut self.announced, key)
    }

    /// "Later": announce it again at the next check.
    pub fn later(&mut self, key: &str) {
        self.announced.retain(|k| k != key);
    }

    /// Forgets announcements of updates that are no longer offered (installed, or the catalog moved on).
    pub fn retain_announced(&mut self, current: &[String]) {
        self.announced.retain(|k| current.contains(k));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct UpdatesRecord {
    pub version: u32,
    pub runtime: KindRecord,
    pub model: KindRecord,
}

impl Default for UpdatesRecord {
    fn default() -> Self {
        UpdatesRecord {
            version: RECORD_VERSION,
            runtime: KindRecord::default(),
            model: KindRecord::default(),
        }
    }
}

impl UpdatesRecord {
    pub fn kind(&self, k: UpdateKind) -> &KindRecord {
        match k {
            UpdateKind::Runtime => &self.runtime,
            UpdateKind::Model => &self.model,
        }
    }

    pub fn kind_mut(&mut self, k: UpdateKind) -> &mut KindRecord {
        match k {
            UpdateKind::Runtime => &mut self.runtime,
            UpdateKind::Model => &mut self.model,
        }
    }

    /// Loads the record; a missing file is an empty record, a damaged or newer one an empty record plus a warning.
    pub fn load(path: &Path) -> (UpdatesRecord, Option<String>) {
        let text = match std::fs::read_to_string(path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (UpdatesRecord::default(), None),
            Err(e) => return (UpdatesRecord::default(), Some(format!("cannot read the AI update record ({e})"))),
        };
        match serde_json::from_str::<UpdatesRecord>(&text) {
            Ok(r) if r.version <= RECORD_VERSION => (r.sanitized(), None),
            Ok(_) => (
                UpdatesRecord::default(),
                Some("the AI update record was saved by a newer IMT; starting afresh".into()),
            ),
            Err(_) => (
                UpdatesRecord::default(),
                Some("the AI update record is damaged; starting afresh".into()),
            ),
        }
    }

    /// Drops what a hand-edited file could make unreasonable (bad keys, too many of them, long errors).
    fn sanitized(mut self) -> UpdatesRecord {
        self.version = RECORD_VERSION;
        for k in UpdateKind::ALL {
            let r = self.kind_mut(k);
            for list in [&mut r.skipped, &mut r.announced] {
                list.retain(|key| is_update_key(key));
                let excess = list.len().saturating_sub(MAX_KEYS);
                list.drain(..excess);
            }
            r.last_error = r.last_error.as_deref().map(bounded_error);
        }
        self
    }

    /// Writes atomically (temp file, then rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let tmp = path.with_extension("json.tmp");
        let text = serde_json::to_string_pretty(self).map_err(|e| AiError::Io(e.to_string()))?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failures_back_off_and_success_resets() {
        let mut r = KindRecord::default();
        r.failed(1000, "offline", false, 86_400);
        assert_eq!((r.failures, r.next_retry, r.last_check), (1, Some(1300), Some(1000)));
        r.failed(1300, "offline", true, 86_400);
        assert_eq!((r.failures, r.next_retry), (2, Some(1900)));
        assert!(r.manual_suggested);
        r.checked(2000);
        assert_eq!((r.failures, r.next_retry, r.last_error.as_deref()), (0, None, None));
        assert_eq!((r.last_check, r.last_success), (Some(2000), Some(2000)));
        assert!(!r.manual_suggested);
        r.failed(3000, &format!("x\n{}", "y".repeat(5000)), false, 60);
        let e = r.last_error.unwrap();
        assert!(!e.contains('\n') && e.ends_with('…') && e.chars().count() == MAX_ERROR_CHARS + 1);
    }

    #[test]
    fn skip_announce_and_later() {
        let mut r = KindRecord::default();
        assert!(r.announce("a") && !r.announce("a"));
        assert!(r.is_announced("a"));
        r.later("a");
        assert!(!r.is_announced("a") && r.announce("a"));
        r.skip("b");
        r.skip("b");
        assert_eq!(r.skipped, ["b"]);
        assert!(r.is_skipped("b") && !r.is_skipped("a"));
        r.announce("c");
        r.retain_announced(&["c".to_string()]);
        assert_eq!(r.announced, ["c"]);
        for i in 0..100 {
            r.skip(&format!("k{i}"));
        }
        assert_eq!(r.skipped.len(), MAX_KEYS);
        assert!(r.is_skipped("k99") && !r.is_skipped("b"), "the oldest go first");
    }

    #[test]
    fn keys_are_plain_words() {
        assert!(is_update_key("runtime:cuda-13@b11700") && is_update_key("model:qwen3.5-9b_q4@0123456789ab"));
        for bad in ["", "a b", "a/b", "<script>", &"k".repeat(161)] {
            assert!(!is_update_key(bad), "{bad}");
        }
    }

    #[test]
    fn the_record_survives_a_roundtrip_and_tolerates_damage() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ai/updates.json");
        assert_eq!(UpdatesRecord::load(&p), (UpdatesRecord::default(), None));
        let mut r = UpdatesRecord::default();
        r.kind_mut(UpdateKind::Model).skip("model:x@1");
        r.kind_mut(UpdateKind::Runtime).failed(5, "offline", false, 86_400);
        r.save(&p).unwrap();
        let (back, warning) = UpdatesRecord::load(&p);
        assert_eq!((back.clone(), warning), (r, None));
        assert!(back.kind(UpdateKind::Model).is_skipped("model:x@1"));
        assert_eq!(back.kind(UpdateKind::Runtime).failures, 1);
        std::fs::write(&p, "{ nope").unwrap();
        assert!(UpdatesRecord::load(&p).1.unwrap().contains("damaged"));
        std::fs::write(&p, r#"{"version": 9}"#).unwrap();
        assert!(UpdatesRecord::load(&p).1.unwrap().contains("newer"));
        assert!(UpdatesRecord::load(dir.path()).1.unwrap().contains("cannot read"));
        std::fs::write(
            &p,
            r#"{"version": 1, "model": {"skipped": ["ok:1", "bad key"], "lastError": "e\u0007rr"}}"#,
        )
        .unwrap();
        let (r, w) = UpdatesRecord::load(&p);
        assert!(w.is_none());
        assert_eq!(r.model.skipped, ["ok:1"]);
        assert_eq!(r.model.last_error.as_deref(), Some("err"));
    }
}
