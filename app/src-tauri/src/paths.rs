//! Every path the webview names is checked here, once: it must be one the user chose. The proof is a
//! grant in [`PathGrants`], filled only on the Rust side, by the native file dialogs ([`pick_paths`]),
//! drag and drop onto a window, the command line and a second launch, and the window's own project file.
//! A command asks [`WindowGrants`] (a command argument resolved from the calling window, never from the
//! webview) for a [`GrantedPath`] to read or write, and gets either the canonical path or a message that
//! tells the user to choose the file again.
//!
//! Grants are per window (a pop-out panel shares its host's), bounded ([`MAX_GRANTS`] per window, the
//! oldest go first) and expire after [`GRANT_TTL`] (the window's project file does not). Paths are
//! compared canonically: `..`, `.` and links resolve before the check, so no spelling reaches another
//! file. A read grant on a folder covers what is inside it.
//!
//! The pure helpers ([`parse`], [`output_file`], [`confined`]) are the only path validators of the shell.

use crate::{err, CmdError, CmdResult, ErrorCode};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::ipc::{CommandArg, CommandItem, InvokeError};
use tauri::{AppHandle, Manager, Runtime};

/// Grants kept per window; the oldest are dropped first.
pub const MAX_GRANTS: usize = 64;
/// How long a chosen path stays usable.
pub const GRANT_TTL: Duration = Duration::from_secs(60 * 60);
/// Longest path accepted from the webview.
pub const MAX_PATH_LEN: usize = 4096;

/// What a command does with a path.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Access {
    Read,
    Write,
}

/// Where a grant came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GrantSource {
    /// A native open or save dialog.
    Dialog,
    /// Files dropped on the window.
    Drop,
    /// The command line, a second launch or the OS ("Open with").
    CommandLine,
    /// The window's project file (opened or saved): kept while the window lives.
    Project,
    /// An entry of the recent lists ([`crate::recent`]): recorded only after a granted open or save.
    Recent,
    /// A file or folder IMT itself produced or chose (an unpacked bundle, a tool's output, the demo
    /// samples, the autosaved session, a capture named by an opened project).
    Output,
}

/// A path the user chose, checked: canonical (for a write, its folder is) and granted for `access`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GrantedPath {
    path: PathBuf,
    access: Access,
}

impl GrantedPath {
    pub fn path(&self) -> &Path {
        &self.path
    }

    #[cfg(test)]
    pub fn access(&self) -> Access {
        self.access
    }
}

#[derive(Debug, Clone)]
struct Grant {
    /// Canonical; for a write, the canonical folder joined with the file name.
    path: PathBuf,
    access: Access,
    /// A folder: what is inside it is covered too (reads only).
    tree: bool,
    expires: Option<Instant>,
}

impl Grant {
    fn covers(&self, path: &Path, access: Access, now: Instant) -> bool {
        if self.expires.is_some_and(|e| now >= e) || self.access != access {
            return false;
        }
        // Only folders opened for reading are trees (and test grants, see `allow_under`).
        self.path == path || (self.tree && path.starts_with(&self.path))
    }
}

/// The grants of every window (managed state; cheap to clone).
#[derive(Clone, Default)]
pub struct PathGrants {
    windows: Arc<Mutex<HashMap<String, Vec<Grant>>>>,
}

impl PathGrants {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<String, Vec<Grant>>> {
        self.windows.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Records that the user chose `path` in `window`. A read grant needs an existing file or folder; a
    /// write grant an existing folder. Fails (with a reason) for anything else.
    pub fn grant(&self, window: &str, path: &Path, access: Access, source: GrantSource) -> CmdResult<GrantedPath> {
        self.grant_at(window, path, access, source, Instant::now())
    }

    fn grant_at(&self, window: &str, path: &Path, access: Access, source: GrantSource, now: Instant) -> CmdResult<GrantedPath> {
        let (canonical, tree) = match access {
            Access::Read => {
                let c = canonical(path)?;
                let tree = c.is_dir();
                (c, tree)
            }
            Access::Write => (output_target(path)?, false),
        };
        let expires = (source != GrantSource::Project).then(|| now + GRANT_TTL);
        let mut all = self.lock();
        let list = all.entry(window.to_string()).or_default();
        list.retain(|g| g.expires.is_none_or(|e| now < e) && !(g.path == canonical && g.access == access));
        if source == GrantSource::Project {
            // One project file per window.
            list.retain(|g| g.expires.is_some());
        }
        list.push(Grant {
            path: canonical.clone(),
            access,
            tree,
            expires,
        });
        while list.len() > MAX_GRANTS {
            let oldest = list.iter().position(|g| g.expires.is_some()).unwrap_or(0);
            list.remove(oldest);
        }
        Ok(GrantedPath { path: canonical, access })
    }

    /// Grants every path that resolves, for reading (dropped files, the command line); returns how many.
    pub fn grant_all<S: AsRef<Path>>(&self, window: &str, paths: &[S], source: GrantSource) -> usize {
        paths
            .iter()
            .filter(|p| self.grant(window, p.as_ref(), Access::Read, source).is_ok())
            .count()
    }

    /// `raw` (from the webview) as a granted path of `window`, or why not. `extensions` (lower case,
    /// without the dot; empty: any) are what the command reads or writes.
    pub fn check(&self, window: &str, raw: &str, access: Access, extensions: &[&str]) -> CmdResult<GrantedPath> {
        self.check_at(window, raw, access, extensions, Instant::now())
    }

    fn check_at(&self, window: &str, raw: &str, access: Access, extensions: &[&str], now: Instant) -> CmdResult<GrantedPath> {
        let path = parse(raw)?;
        let resolved = match access {
            Access::Read => canonical(path)?,
            Access::Write => output_target(path)?,
        };
        has_extension(&resolved, extensions)?;
        let granted = self
            .lock()
            .get(window)
            .is_some_and(|list| list.iter().any(|g| g.covers(&resolved, access, now)));
        if !granted {
            return Err(CmdError::not_granted(match access {
                Access::Read => format!("IMT only reads files you choose: choose {} again with Open", file_name(&resolved)),
                Access::Write => format!(
                    "IMT only writes where you choose: choose where to save {} again",
                    file_name(&resolved)
                ),
            }));
        }
        Ok(GrantedPath { path: resolved, access })
    }

    /// Drops a window's grants (it closed).
    pub fn forget(&self, window: &str) {
        self.lock().remove(window);
    }
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "the file".into())
}

/// The grants of an app (created on first use).
pub fn grants<R: Runtime, M: Manager<R>>(manager: &M) -> PathGrants {
    if let Some(g) = manager.try_state::<PathGrants>() {
        return g.inner().clone();
    }
    manager.manage(PathGrants::default());
    manager.try_state::<PathGrants>().map(|g| g.inner().clone()).unwrap_or_default()
}

/// The grant key of a window: a pop-out panel uses its host's.
fn key(label: &str) -> String {
    crate::panel_windows::resolve(label).to_string()
}

/// Grants `path` to `window` (see [`PathGrants::grant`]).
pub fn grant<R: Runtime, M: Manager<R>>(
    manager: &M,
    window: &str,
    path: &Path,
    access: Access,
    source: GrantSource,
) -> CmdResult<GrantedPath> {
    grants(manager).grant(&key(window), path, access, source)
}

/// Grants every resolvable path to `window` for reading.
pub fn grant_read_all<R: Runtime, M: Manager<R>, S: AsRef<Path>>(manager: &M, window: &str, paths: &[S], source: GrantSource) -> usize {
    grants(manager).grant_all(&key(window), paths, source)
}

/// Tests only: lets `window` read and write anything under `dir` (as if every file there was chosen).
#[cfg(test)]
pub fn allow_under<R: Runtime, M: Manager<R>>(manager: &M, window: &str, dir: &Path) {
    let dir = canonical(dir).expect("an existing folder");
    let all = grants(manager);
    let mut map = all.lock();
    let list = map.entry(key(window)).or_default();
    for access in [Access::Read, Access::Write] {
        list.push(Grant {
            path: dir.clone(),
            access,
            tree: true,
            expires: None,
        });
    }
}

/// The calling window's grants: a command argument resolved from the invoking webview's window.
pub struct WindowGrants {
    grants: PathGrants,
    window: String,
}

impl WindowGrants {
    pub fn of<R: Runtime, M: Manager<R>>(manager: &M, label: &str) -> WindowGrants {
        WindowGrants {
            grants: grants(manager),
            window: key(label),
        }
    }

    /// An existing file or folder the user chose, to read.
    pub fn read(&self, raw: &str, extensions: &[&str]) -> CmdResult<GrantedPath> {
        self.grants.check(&self.window, raw, Access::Read, extensions)
    }

    /// An existing file the user chose, to read.
    pub fn read_file(&self, raw: &str, extensions: &[&str]) -> CmdResult<GrantedPath> {
        let g = self.read(raw, extensions)?;
        if !g.path.is_file() {
            return Err(CmdError::invalid(format!("{} is not a file", g.path.display())));
        }
        Ok(g)
    }

    /// An existing folder the user chose, to read.
    pub fn read_dir(&self, raw: &str) -> CmdResult<GrantedPath> {
        let g = self.read(raw, &[])?;
        if !g.path.is_dir() {
            return Err(CmdError::invalid(format!("{} is not a folder", g.path.display())));
        }
        Ok(g)
    }

    /// An existing file or folder the user chose, or one the caller vouches for (`listed`: an entry of
    /// the recent lists), which is granted again now.
    pub fn read_or_listed(&self, raw: &str, extensions: &[&str], listed: impl FnOnce(&str) -> bool) -> CmdResult<GrantedPath> {
        match self.read(raw, extensions) {
            Ok(g) => Ok(g),
            Err(e) if listed(raw) => {
                self.grant(parse(raw)?, Access::Read, GrantSource::Recent).map_err(|_| e)?;
                self.read(raw, extensions)
            }
            Err(e) => Err(e),
        }
    }

    /// A file the user chose to read, or to write (a new file that does not exist yet).
    pub fn read_or_write(&self, raw: &str, extensions: &[&str]) -> CmdResult<GrantedPath> {
        self.read(raw, extensions).or_else(|e| self.write(raw, extensions).map_err(|_| e))
    }

    /// A file the user chose to write (it may not exist yet).
    pub fn write(&self, raw: &str, extensions: &[&str]) -> CmdResult<GrantedPath> {
        self.grants.check(&self.window, raw, Access::Write, extensions)
    }

    /// Grants a path to this window (the project file it opened or saved).
    pub fn grant(&self, path: &Path, access: Access, source: GrantSource) -> CmdResult<GrantedPath> {
        self.grants.grant(&self.window, path, access, source)
    }
}

impl<'de, R: Runtime> CommandArg<'de, R> for WindowGrants {
    fn from_command(command: CommandItem<'de, R>) -> Result<Self, InvokeError> {
        let webview = command.message.webview();
        let label = webview.window().label().to_string();
        Ok(WindowGrants::of(&webview, &label))
    }
}

// ---- the validators ----

/// A path from the webview: not empty, not too long, no NUL, absolute.
pub fn parse(raw: &str) -> CmdResult<&Path> {
    if raw.trim().is_empty() {
        return Err(CmdError::invalid("choose a file"));
    }
    if raw.len() > MAX_PATH_LEN || raw.contains('\0') {
        return Err(CmdError::invalid("this is not a valid path"));
    }
    let p = Path::new(raw);
    if !p.is_absolute() {
        return Err(CmdError::invalid(format!(
            "the path must be absolute (choose the file in the dialog): {raw}"
        )));
    }
    Ok(p)
}

/// An existing file or folder, canonicalised.
fn canonical(path: &Path) -> CmdResult<PathBuf> {
    std::fs::canonicalize(path).map(plain).map_err(|e| {
        let code = if e.kind() == std::io::ErrorKind::NotFound {
            ErrorCode::NotFound
        } else {
            ErrorCode::Io
        };
        CmdError::new(code, format!("cannot open {}: {e}", path.display()))
    })
}

/// A canonical Windows path without its verbatim prefix when it has an ordinary spelling (`C:` drives and
/// `UNC` shares), so messages and results show the path the user knows. Elsewhere: unchanged.
pub fn plain(path: PathBuf) -> PathBuf {
    let Some(text) = path.to_str() else { return path };
    let rest = match text.strip_prefix(r"\\?\") {
        Some(r) => r,
        None => return path,
    };
    if let Some(share) = rest.strip_prefix(r"UNC\") {
        return PathBuf::from(format!(r"\\{share}"));
    }
    let b = rest.as_bytes();
    if b.len() >= 3 && b[0].is_ascii_alphabetic() && b[1] == b':' && b[2] == b'\\' {
        return PathBuf::from(rest);
    }
    path
}

/// Where a file will be written: its folder canonicalised (it must exist) joined with its name, which
/// must not name a folder or `..`.
fn output_target(path: &Path) -> CmdResult<PathBuf> {
    let name = match path.components().next_back() {
        Some(Component::Normal(n)) => n.to_owned(),
        _ => return Err(CmdError::invalid("choose a file name")),
    };
    let parent = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .ok_or_else(|| CmdError::invalid("choose a folder"))?;
    let dir = std::fs::canonicalize(parent).map_err(|_| CmdError::not_found(format!("the folder {} does not exist", parent.display())))?;
    let target = plain(dir).join(name);
    if target.is_dir() {
        return Err(CmdError::invalid(format!("{} is a folder", target.display())));
    }
    Ok(target)
}

fn has_extension(path: &Path, extensions: &[&str]) -> CmdResult<()> {
    if extensions.is_empty() || path.is_dir() {
        return Ok(());
    }
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if extensions.contains(&ext.as_str()) {
        Ok(())
    } else {
        Err(CmdError::invalid(format!("the file must end in .{}", extensions.join(" or ."))))
    }
}

/// A file to write, without a grant (paths IMT picks itself, or typed into a setting): absolute, one of
/// `extensions`, in an existing folder (canonicalised), not a folder.
pub fn output_file(raw: &str, extensions: &[&str]) -> CmdResult<PathBuf> {
    let target = output_target(parse(raw)?)?;
    has_extension(&target, extensions)?;
    Ok(target)
}

/// `name` (from the webview) as a file directly in `root`: one plain name (no separators, `.`, `..` or
/// NUL), not too long.
pub fn child(root: &Path, name: &str) -> CmdResult<PathBuf> {
    let mut parts = Path::new(name).components();
    match (parts.next(), parts.next()) {
        (Some(Component::Normal(n)), None) if name.len() <= 255 && !name.contains(['\0', '/', '\\']) => Ok(root.join(n)),
        _ => Err(CmdError::invalid(format!(
            "{} is not a file name",
            name.chars().take(80).collect::<String>()
        ))),
    }
}

/// `item` as it may be removed: its parent must resolve (canonically) inside `root`, which is itself
/// canonical, and its name must be a plain file or folder name. The returned path is built from the
/// resolved parent, so a link in the parent chain cannot point the removal elsewhere.
pub fn confined(root: &Path, item: &Path) -> Result<PathBuf, String> {
    let outside = || format!("{} is outside the folder IMT clears", item.display());
    if item.components().any(|c| matches!(c, Component::ParentDir | Component::CurDir)) {
        return Err(outside());
    }
    let (Some(name), Some(parent)) = (item.file_name(), item.parent()) else {
        return Err(outside());
    };
    let parent = std::fs::canonicalize(parent).map_err(|_| outside())?;
    if !parent.starts_with(root) {
        return Err(outside());
    }
    Ok(parent.join(name))
}

// ---- the dialogs ----

/// What a dialog asks for.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PickKind {
    OpenFile,
    OpenFiles,
    OpenFolder,
    OpenFolders,
    Save,
}

/// A file type offered in a dialog.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct PickFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

/// `pick_paths`' request (camelCase on the wire; mirrored as `PickRequest` in api.types.ts).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PickRequest {
    pub kind: PickKind,
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    #[cfg_attr(test, ts(as = "Option<Vec<PickFilter>>", optional))]
    pub filters: Vec<PickFilter>,
    /// Suggested file name of a save dialog (a name, not a path).
    #[serde(default)]
    #[cfg_attr(test, ts(optional))]
    pub default_name: Option<String>,
}

const MAX_TITLE: usize = 200;
const MAX_FILTERS: usize = 16;
const MAX_EXTENSIONS: usize = 16;

impl PickRequest {
    /// Checks the request (bounded text, extensions of letters and digits, a plain default name).
    pub fn validated(self) -> CmdResult<PickRequest> {
        if self.title.chars().count() > MAX_TITLE || self.filters.len() > MAX_FILTERS {
            return Err("invalid dialog request".into());
        }
        for f in &self.filters {
            let ext_ok = |e: &String| !e.is_empty() && e.len() <= 16 && e.chars().all(|c| c.is_ascii_alphanumeric());
            if f.name.chars().count() > MAX_TITLE || f.extensions.len() > MAX_EXTENSIONS || !f.extensions.iter().all(ext_ok) {
                return Err("invalid file type in a dialog request".into());
            }
        }
        if let Some(n) = &self.default_name {
            let plain = Path::new(n).components().count() == 1 && matches!(Path::new(n).components().next(), Some(Component::Normal(_)));
            if n.len() > 255 || n.contains('\0') || !plain {
                return Err("the suggested file name must be a name, not a path".into());
            }
        }
        Ok(self)
    }

    fn access(&self) -> Access {
        if self.kind == PickKind::Save {
            Access::Write
        } else {
            Access::Read
        }
    }
}

/// Shows a native open or save dialog for the calling window and grants what the user chose to it.
/// Returns the chosen paths (empty when cancelled). The only way the webview gets a path it may use.
#[tauri::command]
pub async fn pick_paths<R: Runtime>(app: AppHandle<R>, window: tauri::WebviewWindow<R>, request: PickRequest) -> CmdResult<Vec<String>> {
    let request = request.validated()?;
    let chosen = show(&app, &window, &request).await?;
    let access = request.access();
    let mut granted = Vec::with_capacity(chosen.len());
    for path in chosen {
        match grant(&app, window.label(), &path, access, GrantSource::Dialog) {
            Ok(_) => granted.push(path.to_string_lossy().into_owned()),
            Err(e) => log::warn!(
                "a chosen path could not be granted: {}",
                crate::activity::privacy::redact(&e.message)
            ),
        }
    }
    Ok(granted)
}

/// Runs the dialog; the answer comes back through a callback (the dialog runs on the main thread).
async fn show<R: Runtime>(app: &AppHandle<R>, window: &tauri::WebviewWindow<R>, request: &PickRequest) -> CmdResult<Vec<PathBuf>> {
    use tauri_plugin_dialog::{DialogExt, FilePath};
    let mut dialog = app.dialog().file().set_parent(window);
    if !request.title.is_empty() {
        dialog = dialog.set_title(&request.title);
    }
    for f in &request.filters {
        let exts: Vec<&str> = f.extensions.iter().map(String::as_str).collect();
        dialog = dialog.add_filter(&f.name, &exts);
    }
    if let Some(n) = &request.default_name {
        dialog = dialog.set_file_name(n);
    }
    let (tx, rx) = std::sync::mpsc::channel::<Vec<FilePath>>();
    let one = {
        let tx = tx.clone();
        move |p: Option<FilePath>| {
            let _ = tx.send(p.into_iter().collect());
        }
    };
    let many = move |p: Option<Vec<FilePath>>| {
        let _ = tx.send(p.unwrap_or_default());
    };
    match request.kind {
        PickKind::OpenFile => dialog.pick_file(one),
        PickKind::OpenFolder => dialog.pick_folder(one),
        PickKind::Save => dialog.save_file(one),
        PickKind::OpenFiles => dialog.pick_files(many),
        PickKind::OpenFolders => dialog.pick_folders(many),
    }
    let chosen = tauri::async_runtime::spawn_blocking(move || rx.recv().unwrap_or_default())
        .await
        .map_err(err)?;
    Ok(chosen.into_iter().filter_map(|p| p.into_path().ok()).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    fn s(p: &Path) -> String {
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn only_granted_paths_pass_and_messages_say_what_to_do() {
        let t = TempDir::new("grants");
        let file = t.0.join("in.json");
        std::fs::write(&file, "{}").unwrap();
        let g = PathGrants::default();
        let e = g.check("main", &s(&file), Access::Read, &["json"]).unwrap_err();
        assert!(e.contains("only reads files you choose") && e.contains("in.json"), "{e}");
        g.grant("main", &file, Access::Read, GrantSource::Dialog).unwrap();
        let ok = g.check("main", &s(&file), Access::Read, &["json"]).unwrap();
        assert_eq!(ok.path(), plain(file.canonicalize().unwrap()));
        assert_eq!(ok.access(), Access::Read);
        // Per window, per access, per extension.
        assert!(g.check("w-2", &s(&file), Access::Read, &[]).is_err());
        let e = g.check("main", &s(&file), Access::Write, &[]).unwrap_err();
        assert!(e.contains("only writes where you choose"), "{e}");
        assert!(g.check("main", &s(&file), Access::Read, &["toml"]).unwrap_err().contains(".toml"));
        // A save grant for a file that does not exist yet.
        let out = t.0.join("out.zip");
        g.grant("main", &out, Access::Write, GrantSource::Dialog).unwrap();
        assert!(g.check("main", &s(&out), Access::Write, &["zip"]).is_ok());
        assert!(g.check("main", &s(&t.0.join("other.zip")), Access::Write, &["zip"]).is_err());
        assert!(g.check("main", &s(&out), Access::Read, &[]).is_err(), "a save grant does not read");
        g.forget("main");
        assert!(g.check("main", &s(&file), Access::Read, &[]).is_err());
    }

    #[test]
    fn other_spellings_and_links_cannot_escape_a_grant() {
        let t = TempDir::new("grants-escape");
        std::fs::create_dir_all(t.0.join("chosen/sub")).unwrap();
        std::fs::create_dir_all(t.0.join("secret")).unwrap();
        std::fs::write(t.0.join("secret/key.txt"), "k").unwrap();
        std::fs::write(t.0.join("chosen/sub/a.log"), "a").unwrap();
        let g = PathGrants::default();
        g.grant("main", &t.0.join("chosen"), Access::Read, GrantSource::Dialog).unwrap();
        // Inside the chosen folder, however spelled.
        assert!(g.check("main", &s(&t.0.join("chosen/sub/a.log")), Access::Read, &[]).is_ok());
        assert!(g
            .check("main", &s(&t.0.join("chosen/sub/../sub/./a.log")), Access::Read, &[])
            .is_ok());
        // `..` out of it is refused.
        assert!(g
            .check("main", &s(&t.0.join("chosen/../secret/key.txt")), Access::Read, &[])
            .is_err());
        // A write grant is exact: `..` cannot point it at a neighbour.
        g.grant("main", &t.0.join("chosen/out.csv"), Access::Write, GrantSource::Dialog)
            .unwrap();
        assert!(g.check("main", &s(&t.0.join("chosen/sub/../out.csv")), Access::Write, &[]).is_ok());
        assert!(g.check("main", &s(&t.0.join("chosen/../out.csv")), Access::Write, &[]).is_err());
        assert!(
            g.check("main", &s(&t.0.join("chosen/sub/a.log")), Access::Write, &[]).is_err(),
            "a read tree does not write"
        );
        // A link inside the chosen folder that points outside resolves before the check.
        let link = t.0.join("chosen/link");
        #[cfg(unix)]
        let linked = std::os::unix::fs::symlink(t.0.join("secret"), &link).is_ok();
        #[cfg(windows)]
        let linked = std::os::windows::fs::symlink_dir(t.0.join("secret"), &link).is_ok();
        if linked {
            assert!(g.check("main", &s(&link.join("key.txt")), Access::Read, &[]).is_err());
            assert!(g.check("main", &s(&link.join("new.txt")), Access::Write, &[]).is_err());
        } else {
            eprintln!("cannot create links here; the link case is skipped");
        }
    }

    #[test]
    fn grants_are_bounded_expire_and_need_real_paths() {
        let t = TempDir::new("grants-bounded");
        let g = PathGrants::default();
        let now = Instant::now();
        let project = t.0.join("p.imtproj");
        g.grant_at("main", &project, Access::Write, GrantSource::Project, now).unwrap();
        for i in 0..MAX_GRANTS + 10 {
            g.grant_at("main", &t.0.join(format!("f{i}.csv")), Access::Write, GrantSource::Dialog, now)
                .unwrap();
        }
        assert_eq!(g.lock()["main"].len(), MAX_GRANTS);
        assert!(
            g.check_at("main", &s(&t.0.join("f0.csv")), Access::Write, &[], now).is_err(),
            "the oldest went first"
        );
        let last = s(&t.0.join(format!("f{}.csv", MAX_GRANTS + 9)));
        assert!(g.check_at("main", &last, Access::Write, &[], now).is_ok());
        assert!(g.check_at("main", &last, Access::Write, &[], now + GRANT_TTL).is_err(), "expired");
        // The project file stays.
        assert!(g.check_at("main", &s(&project), Access::Write, &[], now + GRANT_TTL * 10).is_ok());
        // Only real paths can be granted or checked.
        assert!(g.grant("main", &t.0.join("missing.log"), Access::Read, GrantSource::Drop).is_err());
        assert!(g
            .grant("main", &t.0.join("no/such/x.csv"), Access::Write, GrantSource::Dialog)
            .is_err());
        assert_eq!(g.grant_all("main", &[t.0.join("missing"), t.0.clone()], GrantSource::Drop), 1);
        for bad in ["", "relative.csv", "a\0b", &"x".repeat(MAX_PATH_LEN + 1)] {
            assert!(g.check("main", bad, Access::Read, &[]).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn validators() {
        let t = TempDir::new("paths-validators");
        let file = t.0.join("s.json");
        std::fs::write(&file, "{}").unwrap();
        assert!(output_file(&s(&t.0.join("chart.PNG")), &["png"]).is_ok());
        assert!(output_file("chart.png", &["png"]).unwrap_err().contains("absolute"));
        assert!(output_file(&s(&t.0.join("x.exe")), &["png", "svg"])
            .unwrap_err()
            .contains(".png or .svg"));
        assert!(output_file(&s(&t.0.join("no/such/x.png")), &["png"])
            .unwrap_err()
            .contains("does not exist"));
        std::fs::create_dir_all(t.0.join("dir.png")).unwrap();
        assert!(output_file(&s(&t.0.join("dir.png")), &["png"]).unwrap_err().contains("is a folder"));
        assert!(output_file("/", &["png"]).is_err());
        assert!(output_file(&s(&t.0.join("..")), &[]).is_err());
        assert_eq!(child(&t.0, "v1.toml").unwrap(), t.0.join("v1.toml"));
        for bad in ["..", ".", "a/b", r"a\b", "", "/abs", "x\0y"] {
            assert!(child(&t.0, bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn verbatim_windows_paths_get_their_ordinary_spelling() {
        assert_eq!(plain(PathBuf::from(r"\\?\C:\Users\x")), PathBuf::from(r"C:\Users\x"));
        assert_eq!(plain(PathBuf::from(r"\\?\UNC\server\share\x")), PathBuf::from(r"\\server\share\x"));
        assert_eq!(plain(PathBuf::from(r"\\?\Volume{1}\x")), PathBuf::from(r"\\?\Volume{1}\x"));
        assert_eq!(plain(PathBuf::from("/tmp/x")), PathBuf::from("/tmp/x"));
    }

    #[test]
    fn dialog_requests_are_checked() {
        let ok = PickRequest {
            kind: PickKind::Save,
            title: "Export".into(),
            filters: vec![PickFilter {
                name: "Zip".into(),
                extensions: vec!["zip".into()],
            }],
            default_name: Some("imt-diagnostics.zip".into()),
        };
        assert_eq!(ok.clone().validated().unwrap().access(), Access::Write);
        let open = PickRequest {
            kind: PickKind::OpenFolders,
            ..ok.clone()
        };
        assert_eq!(open.access(), Access::Read);
        for name in ["../x.zip", "/etc/x.zip", "a/b.zip", "", ".."] {
            let bad = PickRequest {
                default_name: Some(name.into()),
                ..ok.clone()
            };
            assert!(bad.validated().is_err(), "{name}");
        }
        let bad_ext = PickRequest {
            filters: vec![PickFilter {
                name: "x".into(),
                extensions: vec!["*.exe".into()],
            }],
            ..ok.clone()
        };
        assert!(bad_ext.validated().is_err());
        let long = PickRequest {
            title: "t".repeat(MAX_TITLE + 1),
            ..ok.clone()
        };
        assert!(long.validated().is_err());
        let wire: PickRequest = serde_json::from_value(serde_json::json!({ "kind": "openFiles", "title": "Open" })).unwrap();
        assert!(wire.filters.is_empty() && wire.default_name.is_none());
    }

    #[test]
    fn window_grants_follow_the_host_window_and_the_window_events() {
        let app = crate::test_support::test_app_with_commands(None);
        let t = TempDir::new("grants-window");
        let file = t.0.join("a.log");
        std::fs::write(&file, "x").unwrap();
        // Files from the command line (queued for a window) are granted to it, and its pop-outs.
        crate::windows::queue_open(app.handle(), "main", vec![s(&file)]);
        assert!(WindowGrants::of(app.handle(), "main").read_file(&s(&file), &[]).is_ok());
        assert!(WindowGrants::of(app.handle(), "p-main-1").read_file(&s(&file), &[]).is_ok());
        assert!(WindowGrants::of(app.handle(), "w-2").read_file(&s(&file), &[]).is_err());
        assert!(WindowGrants::of(app.handle(), "main")
            .read_dir(&s(&file))
            .unwrap_err()
            .contains("not a folder"));
        assert!(WindowGrants::of(app.handle(), "main").read_file(&s(&t.0), &[]).is_err());
        // Dropped files.
        let w2 = crate::test_support::window(&app, "w-2");
        let drop = tauri::WindowEvent::DragDrop(tauri::DragDropEvent::Drop {
            paths: vec![file.clone()],
            position: tauri::PhysicalPosition::new(0.0, 0.0),
        });
        crate::window_manager::on_window_event(&w2.as_ref().window(), &drop);
        assert!(WindowGrants::of(app.handle(), "w-2").read_file(&s(&file), &[]).is_ok());
        // The command refuses what nobody chose, through the IPC as the webview calls it.
        let main = crate::test_support::window(&app, "main");
        let other = t.0.join("other.toml");
        std::fs::write(&other, "name = 'x'").unwrap();
        let e = crate::tools::tests::ipc(&main, "import_profile", serde_json::json!({ "path": s(&other) })).unwrap_err();
        assert!(e.to_string().contains("only reads files you choose"), "{e}");
        // The one error contract: code, message, retryable.
        assert_eq!((e["code"].as_str(), e["retryable"].as_bool()), (Some("notGranted"), Some(false)));
    }
}
