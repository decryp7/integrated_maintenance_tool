//! `.imtproj` project files: which files are open, which were removed, the
//! profile, and the UI's own state (filters, zoom history, layout, bookmarks,
//! notes...), which the engine stores without interpreting.
//!
//! Paths inside the project's folder are stored relative to it, so a project
//! folder can be moved or shared with its logs.

use anyhow::{anyhow, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Component, Path, PathBuf};

/// Current project format version. Older files are read; newer ones are refused.
pub const PROJECT_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ProjectFile {
    pub version: u32,
    pub name: String,
    /// Opened files and folders.
    pub sources: Vec<String>,
    /// Files removed from the workspace (kept out of folder scans).
    #[serde(default)]
    pub removed: Vec<String>,
    /// The profile, as TOML.
    pub profile: String,
    /// UI state, opaque to the engine.
    #[serde(default)]
    pub ui: Value,
}

impl ProjectFile {
    /// Writes the project; absolute paths under the project's folder become relative.
    pub fn save(&self, path: &Path) -> Result<()> {
        let base = base_dir(path)?;
        let mut stored = self.clone();
        stored.version = PROJECT_VERSION;
        stored.sources = self.sources.iter().map(|p| relativize(&base, p)).collect();
        stored.removed = self.removed.iter().map(|p| relativize(&base, p)).collect();
        let json = serde_json::to_string_pretty(&stored)?;
        std::fs::write(path, json).with_context(|| format!("cannot write {}", path.display()))
    }

    /// Reads a project and resolves its paths against the project's folder.
    pub fn load(path: &Path) -> Result<ProjectFile> {
        let text = std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
        let mut p: ProjectFile = serde_json::from_str(&text).with_context(|| format!("{} is not an IMT project", path.display()))?;
        if p.version > PROJECT_VERSION {
            return Err(anyhow!(
                "this project was saved by a newer IMT (format {}); please update IMT",
                p.version
            ));
        }
        let base = base_dir(path)?;
        p.sources = p.sources.iter().map(|s| resolve(&base, s)).collect();
        p.removed = p.removed.iter().map(|s| resolve(&base, s)).collect();
        Ok(p)
    }
}

fn base_dir(project: &Path) -> Result<PathBuf> {
    let parent = project.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new("."));
    std::fs::canonicalize(parent).with_context(|| format!("folder of {} does not exist", project.display()))
}

/// Archive entries ("x.zip!/a.log") keep their entry suffix untouched.
fn split_entry(p: &str) -> (&str, Option<&str>) {
    match p.split_once(crate::archive::ENTRY_SEPARATOR) {
        Some((f, e)) => (f, Some(e)),
        None => (p, None),
    }
}

fn join_entry(file: String, entry: Option<&str>) -> String {
    match entry {
        Some(e) => format!("{file}{}{e}", crate::archive::ENTRY_SEPARATOR),
        None => file,
    }
}

fn relativize(base: &Path, p: &str) -> String {
    let (file, entry) = split_entry(p);
    let rel = Path::new(file).strip_prefix(base).ok().map(|r| {
        if r.as_os_str().is_empty() {
            ".".to_string()
        } else {
            r.to_string_lossy().replace('\\', "/")
        }
    });
    join_entry(rel.unwrap_or_else(|| file.to_string()), entry)
}

fn resolve(base: &Path, p: &str) -> String {
    let (file, entry) = split_entry(p);
    let path = Path::new(file);
    let abs = if path.is_absolute() {
        path.to_path_buf()
    } else {
        normalize(&base.join(path))
    };
    join_entry(abs.to_string_lossy().into_owned(), entry)
}

/// Removes `.` and `..` without touching the file system.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                out.pop();
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn round_trips_with_relative_paths() {
        let dir = std::env::temp_dir().join(format!("imt-project-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("logs")).unwrap();
        let dir = std::fs::canonicalize(dir).unwrap();
        let file = dir.join("incident.imtproj");
        let elsewhere = if cfg!(windows) { "C:\\other\\x.log" } else { "/other/x.log" };
        let p = ProjectFile {
            version: 0,
            name: "Incident".into(),
            sources: vec![
                dir.join("logs").to_string_lossy().into_owned(),
                dir.to_string_lossy().into_owned(),
                elsewhere.into(),
                format!("{}!/web01/app.log", dir.join("logs/bundle.zip").display()),
            ],
            removed: vec![dir.join("logs/old.log").to_string_lossy().into_owned()],
            profile: "name = 'P'".into(),
            ui: json!({ "query": { "text": "timeout" } }),
        };
        p.save(&file).unwrap();
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&file).unwrap()).unwrap();
        assert_eq!(stored["version"], PROJECT_VERSION);
        assert_eq!(stored["sources"], json!(["logs", ".", elsewhere, "logs/bundle.zip!/web01/app.log"]));
        assert_eq!(stored["removed"], json!(["logs/old.log"]));

        let back = ProjectFile::load(&file).unwrap();
        assert_eq!(back.sources[0], dir.join("logs").to_string_lossy());
        assert_eq!(back.sources[1], dir.to_string_lossy());
        assert_eq!(back.sources[2], elsewhere);
        assert!(back.sources[3].ends_with("bundle.zip!/web01/app.log"));
        assert_eq!(back.ui["query"]["text"], "timeout");
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn rejects_newer_and_invalid_files() {
        let dir = std::env::temp_dir().join(format!("imt-project-bad-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let newer = dir.join("newer.imtproj");
        std::fs::write(&newer, r#"{"version": 99, "name": "x", "sources": [], "profile": ""}"#).unwrap();
        assert!(ProjectFile::load(&newer).unwrap_err().to_string().contains("newer IMT"));
        let junk = dir.join("junk.imtproj");
        std::fs::write(&junk, "not json").unwrap();
        assert!(ProjectFile::load(&junk).unwrap_err().to_string().contains("not an IMT project"));
        assert!(ProjectFile::load(&dir.join("missing.imtproj")).is_err());
        let p = ProjectFile {
            version: 1,
            name: "x".into(),
            sources: vec![],
            removed: vec![],
            profile: String::new(),
            ui: Value::Null,
        };
        assert!(p.save(&dir.join("no/such/dir/x.imtproj")).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(normalize(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(normalize(Path::new("./a/../b")), PathBuf::from("b"));
        assert!(base_dir(Path::new("relative.imtproj")).is_ok());
    }
}
