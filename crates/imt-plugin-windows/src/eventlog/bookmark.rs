//! Where each channel was last read, persisted so a restarted follower
//! resumes after the last event it wrote: no gaps, no duplicates.
//!
//! A position is the channel's `EventRecordID`, which only grows. The
//! Windows API takes it as bookmark XML ([`bookmark_xml`]), so the follower
//! needs no opaque handles to persist. The file is versioned; a missing,
//! corrupt or newer file is ignored (the follower then starts afresh).

use super::query::Channel;
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

/// Version of the bookmark file this build writes and reads.
pub const VERSION: u32 = 1;
/// Largest bookmark file read (anything bigger is not ours).
const MAX_FILE_BYTES: u64 = 1 << 20;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct File {
    version: u32,
    /// Channel name to the last written `EventRecordID`.
    channels: BTreeMap<String, u64>,
}

/// The last written record per channel, backed by a JSON file.
#[derive(Debug, Clone)]
pub struct BookmarkStore {
    path: PathBuf,
    data: File,
}

impl BookmarkStore {
    /// Loads `path`, or starts empty when it is missing or unusable.
    pub fn load(path: impl Into<PathBuf>) -> BookmarkStore {
        let path = path.into();
        let data = read(&path).unwrap_or_default();
        BookmarkStore { path, data }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The last written record of `channel`.
    pub fn get(&self, channel: &Channel) -> Option<u64> {
        self.data.channels.get(channel.as_str()).copied()
    }

    /// Records progress: the last record written for `channel`. (Record ids
    /// only grow, except after a log was cleared, so the latest write wins.)
    pub fn set(&mut self, channel: &Channel, record_id: u64) {
        self.data.channels.insert(channel.as_str().to_string(), record_id);
    }

    /// Forgets `channel` (e.g. after its log was cleared).
    pub fn forget(&mut self, channel: &Channel) {
        self.data.channels.remove(channel.as_str());
    }

    /// Writes the file atomically (temporary file, then rename).
    pub fn save(&self) -> Result<()> {
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir).with_context(|| format!("cannot create {}", dir.display()))?;
        }
        let file = File {
            version: VERSION,
            channels: self.data.channels.clone(),
        };
        let text = serde_json::to_vec_pretty(&file)?;
        let tmp = self.path.with_extension("json.tmp");
        std::fs::write(&tmp, text).with_context(|| format!("cannot write {}", tmp.display()))?;
        if let Err(e) = std::fs::rename(&tmp, &self.path) {
            let _ = std::fs::remove_file(&tmp);
            return Err(e).with_context(|| format!("cannot replace {}", self.path.display()));
        }
        Ok(())
    }
}

fn read(path: &Path) -> Option<File> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_FILE_BYTES {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    let file: File = serde_json::from_slice(&bytes).ok()?;
    (file.version == VERSION).then_some(file)
}

/// The Event Log bookmark XML for "after record `record_id` of `channel`".
pub fn bookmark_xml(channel: &Channel, record_id: u64) -> String {
    format!(
        "<BookmarkList><Bookmark Channel='{}' RecordId='{record_id}' IsCurrent='true'/></BookmarkList>",
        escape_attr(channel.as_str())
    )
}

fn escape_attr(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '\'' => out.push_str("&apos;"),
            '"' => out.push_str("&quot;"),
            c => out.push(c),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("imt-evtbm-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn saves_loads_and_keeps_the_latest_position() {
        let dir = temp("round");
        let path = dir.join("nested").join("bookmarks.json");
        let system = Channel::parse("System").unwrap();
        let app = Channel::parse("Application").unwrap();
        let mut s = BookmarkStore::load(&path);
        assert_eq!(s.path(), path);
        assert_eq!(s.get(&system), None);
        s.set(&system, 7);
        s.set(&system, 10);
        s.set(&app, 3);
        assert_eq!(s.get(&system), Some(10));
        s.save().unwrap();
        let back = BookmarkStore::load(&path);
        assert_eq!((back.get(&system), back.get(&app)), (Some(10), Some(3)));
        let mut back = back;
        back.forget(&app);
        assert_eq!(back.get(&app), None);
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("\"version\": 1"), "{text}");
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn corrupt_newer_or_huge_files_mean_a_fresh_start() {
        let dir = temp("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let system = Channel::parse("System").unwrap();
        let path = dir.join("b.json");
        for text in [
            "not json".to_string(),
            r#"{"version": 2, "channels": {"System": 5}}"#.to_string(),
            r#"{"version": 1, "channels": {"System": -5}}"#.to_string(),
            format!("{{\"version\": 1, \"pad\": \"{}\"}}", "x".repeat(MAX_FILE_BYTES as usize)),
        ] {
            std::fs::write(&path, text).unwrap();
            assert_eq!(BookmarkStore::load(&path).get(&system), None);
        }
        std::fs::write(&path, r#"{"version": 1, "channels": {"System": 5}}"#).unwrap();
        assert_eq!(BookmarkStore::load(&path).get(&system), Some(5));
        // A folder in the way of the file: saving reports it, nothing panics.
        let blocked = BookmarkStore::load(dir.clone());
        assert!(blocked.save().is_err());
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn bookmark_xml_names_channel_and_record() {
        let c = Channel::parse("Microsoft-Windows-TaskScheduler/Operational").unwrap();
        assert_eq!(
            bookmark_xml(&c, 42),
            "<BookmarkList><Bookmark Channel='Microsoft-Windows-TaskScheduler/Operational' RecordId='42' IsCurrent='true'/></BookmarkList>"
        );
        assert_eq!(escape_attr("a&b<c>'\""), "a&amp;b&lt;c&gt;&apos;&quot;");
    }
}
