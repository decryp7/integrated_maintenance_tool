//! Diff of two snapshots of the same machine: two folders (e.g. two
//! diagnostics-collector runs) whose configuration files are paired by
//! relative path and compared one by one. Files no parser recognises are
//! skipped; symbolic links are not followed.

use super::diff::{diff, IgnoreSet};
use super::dto::{CompareOptions, DiffCounts};
use super::parsers::{read_config, ConfigParsers};
use super::Limits;
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Path, PathBuf};

/// Most configuration files compared per folder.
pub const MAX_FILES: usize = 2000;
/// Deepest folder level searched.
pub const MAX_DIR_DEPTH: usize = 16;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FileStatus {
    Added,
    Removed,
    Changed,
    Unchanged,
    /// One side could not be read or parsed (see `error`).
    Unreadable,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotFile {
    /// Path relative to both folders, `/`-separated.
    pub rel_path: String,
    pub left_path: Option<String>,
    pub right_path: Option<String>,
    pub status: FileStatus,
    pub format: Option<String>,
    pub counts: Option<DiffCounts>,
    pub error: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotDiff {
    pub left: String,
    pub right: String,
    pub files: Vec<SnapshotFile>,
    /// A folder had more than [`MAX_FILES`] configuration files.
    pub truncated: bool,
}

/// Compares the configuration files of two folders.
pub fn diff_folders(parsers: &ConfigParsers, left: &Path, right: &Path, options: &CompareOptions, limits: &Limits) -> Result<SnapshotDiff> {
    IgnoreSet::new(&options.ignore)?;
    for dir in [left, right] {
        if !dir.is_dir() {
            return Err(anyhow!("{} is not a folder", dir.display()));
        }
    }
    let (l, lt) = scan(parsers, left);
    let (r, rt) = scan(parsers, right);
    let mut keys: Vec<&String> = l.keys().chain(r.keys()).collect();
    keys.sort();
    keys.dedup();
    let files = keys
        .into_iter()
        .map(|k| {
            let (lp, rp) = (l.get(k), r.get(k));
            let mut f = SnapshotFile {
                rel_path: k.clone(),
                left_path: lp.map(|p| p.to_string_lossy().into_owned()),
                right_path: rp.map(|p| p.to_string_lossy().into_owned()),
                status: FileStatus::Unreadable,
                format: None,
                counts: None,
                error: None,
            };
            match (lp, rp) {
                (Some(a), Some(b)) => match compare(parsers, a, b, options, limits) {
                    Ok((format, counts)) => {
                        f.status = if counts.differences() > 0 {
                            FileStatus::Changed
                        } else {
                            FileStatus::Unchanged
                        };
                        f.format = Some(format);
                        f.counts = Some(counts);
                    }
                    Err(e) => f.error = Some(e),
                },
                (Some(_), None) => f.status = FileStatus::Removed,
                _ => f.status = FileStatus::Added,
            }
            f
        })
        .collect();
    Ok(SnapshotDiff {
        left: left.to_string_lossy().into_owned(),
        right: right.to_string_lossy().into_owned(),
        files,
        truncated: lt || rt,
    })
}

fn compare(
    parsers: &ConfigParsers,
    a: &Path,
    b: &Path,
    options: &CompareOptions,
    limits: &Limits,
) -> std::result::Result<(String, DiffCounts), String> {
    let left = read_config(parsers, a, limits).map_err(|e| format!("left: {e}"))?;
    let right = read_config(parsers, b, limits).map_err(|e| format!("right: {e}"))?;
    let d = diff(&left, &right, options).map_err(|e| format!("{e:#}"))?;
    Ok((right.format, d.counts))
}

/// Configuration files under `root` by relative path; true when there were too many.
fn scan(parsers: &ConfigParsers, root: &Path) -> (BTreeMap<String, PathBuf>, bool) {
    let mut out = BTreeMap::new();
    let mut stack = vec![(root.to_path_buf(), 0usize)];
    let mut truncated = false;
    while let Some((dir, depth)) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        let mut entries: Vec<_> = entries.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            match entry.file_type() {
                Ok(t) if t.is_dir() && depth < MAX_DIR_DEPTH => stack.push((path, depth + 1)),
                Ok(t) if t.is_file() => {
                    let name = entry.file_name().to_string_lossy().into_owned();
                    if !parsers.recognises(&name, &head(&path)) {
                        continue;
                    }
                    if out.len() >= MAX_FILES {
                        truncated = true;
                        continue;
                    }
                    let rel = path.strip_prefix(root).unwrap_or(&path).to_string_lossy().replace('\\', "/");
                    out.insert(rel, path);
                }
                _ => {}
            }
        }
    }
    (out, truncated)
}

/// The first 4 KiB of a file as text ("" when unreadable or binary).
fn head(path: &Path) -> String {
    let mut buf = Vec::with_capacity(4096);
    let read = std::fs::File::open(path).and_then(|f| f.take(4096).read_to_end(&mut buf));
    if read.is_err() {
        return String::new();
    }
    crate::importers::decode(&buf).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);
    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn tmp(tag: &str) -> Dir {
        let d = std::env::temp_dir().join(format!("debrief-snapshot-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        Dir(d)
    }

    fn write(dir: &Path, rel: &str, text: &str) {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, text).unwrap();
    }

    #[test]
    fn pairs_files_by_relative_path() {
        let root = tmp("pairs");
        let (a, b) = (root.0.join("a"), root.0.join("b"));
        write(&a, "app/settings.json", r#"{"x": 1}"#);
        write(&b, "app/settings.json", r#"{"x": 2}"#);
        write(&a, "same.ini", "[s]\nk=v\n");
        write(&b, "same.ini", "[S]\nK=v\n");
        write(&a, "old.yaml", "a: 1\n");
        write(&b, "new.toml", "a = 1\n");
        write(&a, "broken.json", "{");
        write(&b, "broken.json", "{}");
        write(&a, "notes.txt", "not config");
        // Deeper than the search goes: not compared.
        write(&a, &format!("{}deep.json", "d/".repeat(MAX_DIR_DEPTH + 1)), "{}");
        write(&b, "notes.txt", "still not config");
        let d = diff_folders(&ConfigParsers::builtin(), &a, &b, &CompareOptions::default(), &Limits::default()).unwrap();
        let shown: Vec<(&str, FileStatus)> = d.files.iter().map(|f| (f.rel_path.as_str(), f.status)).collect();
        assert_eq!(
            shown,
            [
                ("app/settings.json", FileStatus::Changed),
                ("broken.json", FileStatus::Unreadable),
                ("new.toml", FileStatus::Added),
                ("old.yaml", FileStatus::Removed),
                ("same.ini", FileStatus::Unchanged),
            ]
        );
        assert_eq!(d.files[0].counts.unwrap().changed, 1);
        assert_eq!(d.files[0].format.as_deref(), Some("json"));
        assert!(d.files[1].error.as_ref().unwrap().starts_with("left: JSON"));
        assert!(!d.truncated);
    }

    #[test]
    fn rejects_files_and_bad_options() {
        let root = tmp("bad");
        write(&root.0, "f.json", "{}");
        let p = ConfigParsers::builtin();
        let file = root.0.join("f.json");
        let o = CompareOptions::default();
        assert!(diff_folders(&p, &file, &root.0, &o, &Limits::default())
            .unwrap_err()
            .to_string()
            .contains("is not a folder"));
        let bad = CompareOptions {
            ignore: vec!["re:(".into()],
            case_insensitive: false,
        };
        assert!(diff_folders(&p, &root.0, &root.0, &bad, &Limits::default()).is_err());
        assert_eq!(head(&root.0.join("missing")), "");
    }

    #[test]
    fn caps_the_number_of_files() {
        let root = tmp("cap");
        let a = root.0.join("a");
        for i in 0..MAX_FILES + 1 {
            write(&a, &format!("f{i}.json"), "{}");
        }
        let d = diff_folders(&ConfigParsers::builtin(), &a, &a, &CompareOptions::default(), &Limits::default()).unwrap();
        assert!(d.truncated);
        assert_eq!(d.files.len(), MAX_FILES);
    }
}
