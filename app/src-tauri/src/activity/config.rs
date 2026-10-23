//! What the activity log records (the Diagnostics settings) and the facts every log line carries.

use serde::{Deserialize, Serialize};
use std::hash::{BuildHasher, Hasher};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::OnceLock;

/// How much detail activity entries carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum Detail {
    /// Query and search text as a fingerprint and length; file paths as the file name and a hashed folder.
    #[default]
    Normal,
    /// Full query text and file paths (still without secrets, file contents or AI/web text).
    Detailed,
}

/// The `activity` part of `diagnostics.json`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ActivitySettings {
    pub detail: Detail,
    /// High-frequency read-only commands (page, timeline, facet, chart, ...) are logged at Debug, not Info.
    pub queries_at_debug: bool,
}

impl Default for ActivitySettings {
    fn default() -> Self {
        ActivitySettings {
            detail: Detail::Normal,
            queries_at_debug: true,
        }
    }
}

/// The settings in effect, readable from any thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ActivityConfig {
    pub detail: Detail,
    pub queries_at_debug: bool,
}

static DETAIL: AtomicU8 = AtomicU8::new(0);
static QUERIES_AT_DEBUG: AtomicBool = AtomicBool::new(true);

/// Applies the settings now (at startup and when they change).
pub fn set_config(s: ActivitySettings) {
    DETAIL.store(u8::from(s.detail == Detail::Detailed), Ordering::Relaxed);
    QUERIES_AT_DEBUG.store(s.queries_at_debug, Ordering::Relaxed);
}

pub fn config() -> ActivityConfig {
    ActivityConfig {
        detail: if DETAIL.load(Ordering::Relaxed) == 1 {
            Detail::Detailed
        } else {
            Detail::Normal
        },
        queries_at_debug: QUERIES_AT_DEBUG.load(Ordering::Relaxed),
    }
}

/// A random id for this run of Debrief, on every log line and report (matches a report to its log).
pub fn session_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let random = std::collections::hash_map::RandomState::new().build_hasher().finish();
        format!("{:016x}", random ^ u64::from(std::process::id()).rotate_left(32))
    })
}

/// The commit this binary was built from (`build.rs` records it), or `"unknown"`.
pub fn build_commit() -> &'static str {
    match option_env!("DEBRIEF_BUILD_COMMIT") {
        Some(c) if !c.is_empty() => c,
        _ => "unknown",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_default_and_parse() {
        let d = ActivitySettings::default();
        assert_eq!((d.detail, d.queries_at_debug), (Detail::Normal, true));
        let s: ActivitySettings = serde_json::from_str(r#"{"detail":"detailed"}"#).unwrap();
        assert_eq!((s.detail, s.queries_at_debug), (Detail::Detailed, true));
        assert!(serde_json::from_str::<ActivitySettings>(r#"{"detail":"everything"}"#).is_err());
    }

    #[test]
    fn config_round_trips_and_ids_are_stable() {
        let _g = crate::activity::logger::test_lock();
        set_config(ActivitySettings {
            detail: Detail::Detailed,
            queries_at_debug: false,
        });
        assert_eq!(
            config(),
            ActivityConfig {
                detail: Detail::Detailed,
                queries_at_debug: false
            }
        );
        set_config(ActivitySettings::default());
        assert_eq!(config().detail, Detail::Normal);
        assert_eq!(session_id(), session_id());
        assert_eq!(session_id().len(), 16);
        assert!(!build_commit().is_empty());
    }
}
