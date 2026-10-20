//! The recent files and recent projects, owned by Rust: an entry is recorded only after a granted open
//! or save succeeded ([`crate::paths`]), so reopening one from the list (the welcome page, the Recent
//! projects dialog, the tray) is the user's earlier choice and is granted again on use.
//!
//! The lists live in `recent.json` in the config folder (versioned; a missing or corrupt file reads as
//! empty) and are app-wide: every window shows the same lists. A change is announced with
//! [`RECENT_EVENT`] and refreshes the tray menu.
//!
//! Commands: `recent_list`, `recent_remove`, `recent_clear`.

use crate::CmdResult;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Event to every window: the lists changed (payload: [`RecentLists`]).
pub const RECENT_EVENT: &str = "imt://recent-changed";
/// Recent files and folders kept.
pub const MAX_FILES: usize = 8;
/// Recent projects kept.
pub const MAX_PROJECTS: usize = 10;
const VERSION: u32 = 1;
const FILE: &str = "recent.json";
const MAX_NAME: usize = 120;

/// A project in the recent list.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: String,
    pub name: String,
}

/// Both lists, newest first (camelCase on the wire; mirrored as `RecentLists` in api.types.ts).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecentLists {
    pub files: Vec<String>,
    pub projects: Vec<RecentProject>,
}

/// Which list a command means.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum RecentKind {
    Files,
    Projects,
}

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    #[serde(flatten)]
    lists: RecentLists,
}

/// Managed state: the lists and where they are kept.
pub struct Recents {
    file: Option<PathBuf>,
    lists: Mutex<RecentLists>,
}

impl Recents {
    /// The lists kept in `dir` (none: in memory only), read once.
    pub fn load(dir: Option<&Path>) -> Recents {
        let file = dir.map(|d| d.join(FILE));
        let lists = file.as_deref().map(read).unwrap_or_default();
        Recents {
            file,
            lists: Mutex::new(lists),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, RecentLists> {
        self.lists.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn list(&self) -> RecentLists {
        self.lock().clone()
    }

    /// Whether `path` (as the webview sends it) is a recent file or folder.
    pub fn has_file(&self, path: &str) -> bool {
        self.lock().files.iter().any(|f| same(f, path))
    }

    /// Whether `path` is a recent project.
    pub fn has_project(&self, path: &str) -> bool {
        self.lock().projects.iter().any(|p| same(&p.path, path))
    }

    /// Puts opened files and folders at the front (deduplicated, capped); whether the list changed.
    pub fn record_files(&self, paths: &[String]) -> bool {
        self.change(|l| {
            let mut next: Vec<String> = paths.iter().filter(|p| keepable(p)).take(MAX_FILES).cloned().collect();
            next.extend(l.files.iter().filter(|f| !paths.iter().any(|p| same(p, f))).cloned());
            next.truncate(MAX_FILES);
            l.files = next;
        })
    }

    /// Puts a project at the front (deduplicated by path, capped).
    pub fn record_project(&self, path: &str, name: &str) -> bool {
        if !keepable(path) {
            return false;
        }
        let name: String = name.chars().filter(|c| !c.is_control()).take(MAX_NAME).collect();
        let name = if name.trim().is_empty() { file_stem(path) } else { name };
        self.change(|l| {
            l.projects.retain(|p| !same(&p.path, path));
            l.projects.insert(
                0,
                RecentProject {
                    path: path.to_string(),
                    name,
                },
            );
            l.projects.truncate(MAX_PROJECTS);
        })
    }

    pub fn remove(&self, kind: RecentKind, path: &str) -> bool {
        self.change(|l| match kind {
            RecentKind::Files => l.files.retain(|f| !same(f, path)),
            RecentKind::Projects => l.projects.retain(|p| !same(&p.path, path)),
        })
    }

    pub fn clear(&self, kind: RecentKind) -> bool {
        self.change(|l| match kind {
            RecentKind::Files => l.files.clear(),
            RecentKind::Projects => l.projects.clear(),
        })
    }

    /// Applies `f`; when the lists changed, writes them (outside the lock) and says so.
    fn change(&self, f: impl FnOnce(&mut RecentLists)) -> bool {
        let snapshot = {
            let mut lists = self.lock();
            let before = lists.clone();
            f(&mut lists);
            if *lists == before {
                return false;
            }
            lists.clone()
        };
        if let Some(file) = &self.file {
            if let Err(e) = write(file, &snapshot) {
                log::warn!("could not save the recent lists: {e}");
            }
        }
        true
    }
}

/// Paths compare as written, ignoring case on Windows and macOS (case-insensitive file systems).
fn same(a: &str, b: &str) -> bool {
    if cfg!(any(windows, target_os = "macos")) {
        a.eq_ignore_ascii_case(b)
    } else {
        a == b
    }
}

fn keepable(path: &str) -> bool {
    path.len() <= crate::paths::MAX_PATH_LEN && !path.contains('\0') && Path::new(path).is_absolute()
}

fn file_stem(path: &str) -> String {
    Path::new(path)
        .file_stem()
        .map(|s| s.to_string_lossy().chars().take(MAX_NAME).collect())
        .unwrap_or_default()
}

fn read(file: &Path) -> RecentLists {
    let Ok(text) = std::fs::read_to_string(file) else {
        return RecentLists::default();
    };
    match serde_json::from_str::<Stored>(&text) {
        Ok(s) if s.version == VERSION => {
            let mut lists = s.lists;
            lists.files.retain(|f| keepable(f));
            lists.files.truncate(MAX_FILES);
            lists.projects.retain(|p| keepable(&p.path));
            lists.projects.truncate(MAX_PROJECTS);
            lists
        }
        _ => RecentLists::default(),
    }
}

fn write(file: &Path, lists: &RecentLists) -> std::io::Result<()> {
    let stored = Stored {
        version: VERSION,
        lists: lists.clone(),
    };
    let text = serde_json::to_string_pretty(&stored).map_err(std::io::Error::other)?;
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let tmp = file.with_extension("json.tmp");
    std::fs::write(&tmp, text)
        .and_then(|_| std::fs::rename(&tmp, file))
        .inspect_err(|_| {
            let _ = std::fs::remove_file(&tmp);
        })
}

/// The app's lists (created on first use, read from the config folder).
pub fn recents<R: Runtime, M: Manager<R>>(manager: &M) -> tauri::State<'_, Recents> {
    if manager.try_state::<Recents>().is_none() {
        let dir = crate::app_dirs::config(manager).ok();
        manager.manage(Recents::load(dir.as_deref()));
    }
    manager.state::<Recents>()
}

/// Tells every window and the tray that the lists changed.
fn changed<R: Runtime>(app: &AppHandle<R>) {
    let lists = recents(app).list();
    let _ = app.emit(RECENT_EVENT, lists);
    crate::tray::refresh(app);
}

/// Records files and folders opened in a window (they were granted).
pub fn opened_files<R: Runtime>(app: &AppHandle<R>, paths: &[String]) {
    if recents(app).record_files(paths) {
        changed(app);
    }
}

/// Records a project opened or saved (its file was granted).
pub fn opened_project<R: Runtime>(app: &AppHandle<R>, path: &str, name: &str) {
    if recents(app).record_project(path, name) {
        changed(app);
    }
}

#[tauri::command]
pub async fn recent_list<R: Runtime>(app: AppHandle<R>) -> RecentLists {
    recents(&app).list()
}

#[tauri::command]
pub async fn recent_remove<R: Runtime>(app: AppHandle<R>, kind: RecentKind, path: String) -> CmdResult<()> {
    if recents(&app).remove(kind, &path) {
        changed(&app);
    }
    Ok(())
}

#[tauri::command]
pub async fn recent_clear<R: Runtime>(app: AppHandle<R>, kind: RecentKind) -> CmdResult<()> {
    if recents(&app).clear(kind) {
        changed(&app);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn abs(name: &str) -> String {
        std::env::temp_dir().join(name).to_string_lossy().into_owned()
    }

    #[test]
    fn lists_are_deduplicated_capped_and_kept() {
        let t = TempDir::new("recent");
        let r = Recents::load(Some(&t.0));
        assert_eq!(r.list(), RecentLists::default());
        assert!(r.record_files(&[abs("a"), abs("b")]));
        assert!(!r.record_files(&[abs("a"), abs("b")]), "unchanged");
        assert!(r.record_files(&[abs("b")]));
        assert_eq!(r.list().files, vec![abs("b"), abs("a")]);
        assert!(!r.record_files(&["relative".into()]), "only absolute paths are kept");
        let many: Vec<String> = (0..20).map(|i| abs(&format!("f{i}"))).collect();
        r.record_files(&many);
        assert_eq!(r.list().files.len(), MAX_FILES);
        for i in 0..15 {
            r.record_project(&abs(&format!("p{i}.imtproj")), &format!("P{i}"));
        }
        let projects = r.list().projects;
        assert_eq!(projects.len(), MAX_PROJECTS);
        assert_eq!(projects[0].name, "P14");
        r.record_project(&abs("p14.imtproj"), "  ");
        assert_eq!(r.list().projects[0].name, "p14", "an empty name is the file's");
        assert!(r.has_project(&abs("p14.imtproj")) && r.has_file(&abs("f0")));
        assert!(!r.has_project(&abs("other.imtproj")));
        // Kept across loads.
        let again = Recents::load(Some(&t.0));
        assert_eq!(again.list(), r.list());
        assert!(again.remove(RecentKind::Projects, &abs("p14.imtproj")));
        assert!(!again.has_project(&abs("p14.imtproj")));
        assert!(again.clear(RecentKind::Files));
        assert!(!again.clear(RecentKind::Files));
        assert!(Recents::load(Some(&t.0)).list().files.is_empty());
    }

    #[test]
    fn a_corrupt_or_newer_file_reads_as_empty() {
        let t = TempDir::new("recent-corrupt");
        std::fs::write(t.0.join(FILE), "{ not json").unwrap();
        assert_eq!(Recents::load(Some(&t.0)).list(), RecentLists::default());
        std::fs::write(t.0.join(FILE), r#"{"version":99,"files":["/x"],"projects":[]}"#).unwrap();
        assert_eq!(Recents::load(Some(&t.0)).list(), RecentLists::default());
        assert_eq!(Recents::load(None).list(), RecentLists::default());
    }
}
