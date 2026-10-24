//! The kept research profile (ADR 0058): an opt-in browser profile folder under Debrief's data folder (never the
//! user's own browser profile), so cookies and consent choices survive between researches and engines ask less often.
//! Off by default: every browser then starts with a throwaway profile.
//!
//! Plain file checks, no browser: whether the folder can be used (not damaged, not over its size limit), its size, and
//! clearing it. Who uses it and when is [`super::sessions::Renderers`]'s business.

use std::path::{Path, PathBuf};

/// The profile's folder name under Debrief's data folder.
pub const FOLDER: &str = "research-profile";
/// The default size limit, and the limits of the setting.
pub const DEFAULT_MAX_BYTES: u64 = 500 << 20;
pub const MIN_MAX_BYTES: u64 = 50 << 20;
pub const MAX_MAX_BYTES: u64 = 5 << 30;
/// The browser's disk cache inside the kept profile (it would otherwise fill the size limit quickly).
pub const DISK_CACHE_BYTES: u64 = 50 << 20;
/// Files a size count looks at, at most (a profile is a few thousand files).
const MAX_FILES: usize = 200_000;
/// The files whose damage makes the browser misbehave: JSON documents it reads at start.
const JSON_FILES: &[&str] = &["Local State", "Default/Preferences"];

/// The kept profile's folder for a data folder.
pub fn folder(data_dir: &Path) -> PathBuf {
    data_dir.join(FOLDER)
}

/// Whether the kept profile can be used now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Check {
    /// Missing (it is created) or fine.
    Usable,
    /// Larger than the limit.
    TooLarge { bytes: u64, max: u64 },
    /// A file the browser reads at start is damaged.
    Damaged(String),
}

impl Check {
    /// What the user is told when a throwaway profile is used instead (one plain sentence and the next step).
    pub fn message(&self) -> Option<String> {
        match self {
            Check::Usable => None,
            Check::TooLarge { bytes, max } => Some(format!(
                "The research profile is {} (the limit is {}), so this research used a throwaway profile. Clear it in Settings ▸ Web browser, or raise the limit.",
                mb(*bytes),
                mb(*max)
            )),
            Check::Damaged(why) => Some(format!(
                "The research profile is damaged ({why}), so this research used a throwaway profile. Clear it in Settings ▸ Web browser to start a fresh one."
            )),
        }
    }
}

fn mb(bytes: u64) -> String {
    format!("{} MB", bytes.div_ceil(1 << 20))
}

/// Checks the kept profile at `dir` against `max` bytes.
pub fn check(dir: &Path, max: u64) -> Check {
    if !dir.exists() {
        return Check::Usable;
    }
    if !dir.is_dir() {
        return Check::Damaged("it is not a folder".into());
    }
    for name in JSON_FILES {
        let path = dir.join(name);
        let Ok(text) = std::fs::read(&path) else { continue };
        if !serde_json::from_slice::<serde_json::Value>(&text).is_ok_and(|v| v.is_object()) {
            return Check::Damaged(format!("{name} cannot be read"));
        }
    }
    let bytes = size(dir);
    if bytes > max {
        return Check::TooLarge { bytes, max };
    }
    Check::Usable
}

/// The size of the files under `dir` (bounded: at most [`MAX_FILES`] files are counted; links are not followed).
pub fn size(dir: &Path) -> u64 {
    let mut total = 0u64;
    let mut files = 0usize;
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&d) else { continue };
        for e in entries.flatten() {
            let Ok(meta) = e.path().symlink_metadata() else { continue };
            if meta.is_dir() {
                stack.push(e.path());
            } else {
                total = total.saturating_add(meta.len());
                files += 1;
                if files >= MAX_FILES {
                    return total;
                }
            }
        }
    }
    total
}

/// Removes the kept profile (cookies, consent, history of research pages). Missing is fine.
pub fn clear(dir: &Path) -> Result<(), String> {
    match std::fs::remove_dir_all(dir) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(format!(
            "the research profile could not be removed ({e}); close the research browser and try again"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_or_fine_profile_is_usable() {
        let d = tempfile::tempdir().unwrap();
        let p = folder(d.path());
        assert_eq!(check(&p, DEFAULT_MAX_BYTES), Check::Usable, "missing: it is created");
        std::fs::create_dir_all(p.join("Default")).unwrap();
        std::fs::write(p.join("Local State"), "{}").unwrap();
        std::fs::write(p.join("Default/Preferences"), r#"{"a":1}"#).unwrap();
        assert_eq!(check(&p, DEFAULT_MAX_BYTES), Check::Usable);
        assert_eq!(Check::Usable.message(), None);
    }

    #[test]
    fn damaged_or_too_large_profiles_say_why_and_what_to_do() {
        let d = tempfile::tempdir().unwrap();
        let p = folder(d.path());
        std::fs::create_dir_all(p.join("Default")).unwrap();
        std::fs::write(p.join("Default/Preferences"), "{not json").unwrap();
        let c = check(&p, DEFAULT_MAX_BYTES);
        assert_eq!(c, Check::Damaged("Default/Preferences cannot be read".into()));
        assert!(c.message().unwrap().contains("Clear it in Settings ▸ Web browser"));
        std::fs::write(p.join("Default/Preferences"), "{}").unwrap();
        std::fs::write(p.join("Default/big"), vec![0u8; 4096]).unwrap();
        let c = check(&p, 1000);
        assert!(matches!(c, Check::TooLarge { bytes, max: 1000 } if bytes >= 4096), "{c:?}");
        assert!(c.message().unwrap().contains("throwaway profile"));
        let file = d.path().join("file");
        std::fs::write(&file, "x").unwrap();
        assert!(matches!(check(&file, DEFAULT_MAX_BYTES), Check::Damaged(_)));
    }

    #[test]
    fn clearing_removes_the_folder_and_a_missing_one_is_fine() {
        let d = tempfile::tempdir().unwrap();
        let p = folder(d.path());
        std::fs::create_dir_all(p.join("Default")).unwrap();
        std::fs::write(p.join("Default/Cookies"), "c").unwrap();
        assert!(size(&p) >= 1);
        clear(&p).unwrap();
        assert!(!p.exists());
        clear(&p).unwrap();
        assert_eq!(size(&p), 0);
    }
}
