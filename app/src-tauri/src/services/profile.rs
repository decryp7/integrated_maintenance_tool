//! The active profile on disk: checking a text against a store, saving it with version history,
//! listing and reading versions, loading the saved one.
//!
//! One source of truth: a window's profile is what its store applied. `save_profile` checks the text,
//! re-imports with it as a job, and only after the swap succeeded writes it to disk and makes it the
//! window's profile; a cancelled or failed apply leaves both as they were.

use crate::error::{CmdError, CmdResult};
use crate::profile::ProfileDoc;
use crate::ErrorCode;
use imt_core::{LogStore, Profile};
use serde_json::Value;
use std::path::{Path, PathBuf};

/// The active profile's file in the profile folder.
pub const PROFILE_FILE: &str = "active.imtprofile.toml";
/// Saved profile versions kept for "restore".
pub const PROFILE_HISTORY: usize = 20;

/// The profile folder (none: the profile lives in memory only).
#[derive(Debug, Clone, Default)]
pub struct ProfileFiles {
    dir: Option<PathBuf>,
}

impl ProfileFiles {
    pub fn new(dir: Option<PathBuf>) -> ProfileFiles {
        ProfileFiles { dir }
    }

    pub fn active(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(PROFILE_FILE))
    }

    fn history_dir(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join("history"))
    }

    /// `toml` as a profile that compiles against `store`'s plugins, or why not (field `toml`, with the line).
    pub fn check(store: &dyn LogStore, toml: &str) -> CmdResult<Profile> {
        let profile = Profile::from_toml(toml).map_err(|e| CmdError::invalid(e.message).field("toml").line(e.line))?;
        store.validate_profile(profile.clone()).map_err(|e| {
            let line = imt_core::profile::RuleContext::of(&e).and_then(|r| r.line_in(toml));
            CmdError::invalid(format!("{e:#}")).field("toml").line(line)
        })?;
        Ok(profile)
    }

    /// Makes `toml` the active profile on disk: the previous one goes to the history first, the new one is
    /// written atomically. Returns the file (none without a folder).
    pub fn persist(&self, toml: &str) -> CmdResult<Option<PathBuf>> {
        let Some(path) = self.active() else { return Ok(None) };
        let io = |e: std::io::Error| CmdError::new(ErrorCode::Io, format!("cannot save the profile: {e}"));
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(io)?;
        }
        self.archive(&path);
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, toml).and_then(|_| std::fs::rename(&tmp, &path)).map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io(e)
        })?;
        Ok(Some(path))
    }

    /// Copies the active profile into `history/` (keeping the newest [`PROFILE_HISTORY`]).
    fn archive(&self, active: &Path) {
        let Some(dir) = self.history_dir() else { return };
        if !active.exists() || std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S%3f");
        let _ = std::fs::copy(active, dir.join(format!("{stamp}.imtprofile.toml")));
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        for old in files.iter().rev().skip(PROFILE_HISTORY) {
            let _ = std::fs::remove_file(old);
        }
    }

    /// Previous versions, newest first: `[{ file, name, savedAt, savedAtMs }]`.
    pub fn history(&self) -> Vec<Value> {
        let Some(dir) = self.history_dir() else { return Vec::new() };
        let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
            .map(|r| r.flatten().map(|e| e.path()).collect())
            .unwrap_or_default();
        files.sort();
        files
            .iter()
            .rev()
            .map(|f| {
                let toml = std::fs::read_to_string(f).unwrap_or_default();
                let name = Profile::from_toml(&toml).map(|p| p.name).unwrap_or_else(|_| "(unreadable)".into());
                let stem = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                let saved_at = stem.split('.').next().unwrap_or("");
                serde_json::json!({ "file": stem, "name": name, "savedAt": saved_at, "savedAtMs": stamp_ms(saved_at) })
            })
            .collect()
    }

    /// One previous version (a plain file name of the history).
    pub fn version(&self, file: &str) -> CmdResult<ProfileDoc> {
        let dir = self.history_dir().ok_or_else(|| CmdError::unavailable("no profile folder"))?;
        let version = crate::paths::child(&dir, file).map_err(|_| CmdError::invalid("invalid version"))?;
        let toml = std::fs::read_to_string(version).map_err(|e| CmdError::not_found(format!("cannot read this version: {e}")))?;
        let name = Profile::from_toml(&toml).map_err(|e| CmdError::invalid(e.message))?.name;
        Ok(ProfileDoc { name, path: None, toml })
    }

    /// The saved profile if it still compiles against `store`'s plugins, else the built-in default.
    pub fn load(&self, store: &dyn LogStore) -> (Profile, ProfileDoc) {
        if let Some(path) = self.active() {
            if let Ok(toml) = std::fs::read_to_string(&path) {
                match Profile::from_toml(&toml) {
                    Ok(p) if store.validate_profile(p.clone()).is_ok() => {
                        let doc = ProfileDoc {
                            name: p.name.clone(),
                            path: Some(path.to_string_lossy().into_owned()),
                            toml,
                        };
                        return (p, doc);
                    }
                    _ => log::warn!("ignoring invalid profile {}", path.display()),
                }
            }
        }
        ProfileDoc::builtin()
    }
}

/// Epoch ms of a history stamp (`YYYYMMDD-HHMMSSmmm`, UTC); `None` when it is not one.
pub fn stamp_ms(stamp: &str) -> Option<i64> {
    let (date_time, millis) = stamp.split_at_checked(stamp.len().checked_sub(3)?)?;
    let t = chrono::NaiveDateTime::parse_from_str(date_time, "%Y%m%d-%H%M%S").ok()?;
    Some(t.and_utc().timestamp_millis() + millis.parse::<i64>().ok()?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn saving_keeps_history_atomically_and_versions_are_plain_names() {
        let t = TempDir::new("profile-service");
        let files = ProfileFiles::new(Some(t.0.clone()));
        for name in ["One", "Two", "Three"] {
            files.persist(&format!("name = '{name}'")).unwrap();
        }
        assert_eq!(std::fs::read_to_string(files.active().unwrap()).unwrap(), "name = 'Three'");
        let history = files.history();
        assert_eq!(history.len(), 2);
        assert_eq!(history[0]["name"], "Two");
        assert!(history[0]["savedAtMs"].as_i64().unwrap() > 0);
        assert_eq!(files.version(history[1]["file"].as_str().unwrap()).unwrap().name, "One");
        assert_eq!(files.version("../x").unwrap_err().code, ErrorCode::Invalid);
        assert_eq!(files.version("missing.toml").unwrap_err().code, ErrorCode::NotFound);
        assert!(!t.0.join("active.imtprofile.toml.tmp").exists());
        let none = ProfileFiles::new(None);
        assert_eq!(none.persist("name = 'x'").unwrap(), None);
        assert!(none.history().is_empty());
        assert_eq!(none.version("a").unwrap_err().code, ErrorCode::Unavailable);
        assert_eq!(stamp_ms("bad"), None);
    }
}
