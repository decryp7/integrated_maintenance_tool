//! What each storage category holds, what clearing it may remove, and the clearing itself. Works on
//! folders resolved by the caller ([`Folders`]) and reaches the AI engine only through [`AiPort`], so it
//! is tested with temporary folders and a fake engine.

use super::model::{CategoryClearResult, CategoryId, StorageCategory};
use super::walk::{confined, remove, size_of, Budget};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::SystemTime;

/// Top-level entries one category lists at most (a folder with more is cleared in several goes).
pub const MAX_ITEMS: usize = 10_000;
/// The file name of an autosaved session (`lifecycle`).
const SESSION_FILE: &str = "last-session.imtproj";

/// What the storage view needs from the AI engine (implemented for `AiCore`; tests pass a fake).
pub trait AiPort: Send + Sync {
    /// Why runtime builds of the pinned release cannot be removed now (a model is loaded or being set up).
    fn engine_busy(&self) -> Option<String>;
    /// Why models cannot be removed now (a set-up is downloading one).
    fn setup_busy(&self) -> Option<String>;
    /// The custom `llama-server` the settings name, if any (never removed).
    fn custom_server(&self) -> Option<PathBuf>;
    fn models_dir(&self) -> PathBuf;
    /// Installed models: file name and whether it is the active one.
    fn models(&self) -> Vec<(String, bool)>;
    fn delete_runtime(&self, tag: &str, id: &str) -> Result<(), String>;
    fn delete_model(&self, file_name: &str) -> Result<(), String>;
}

/// The folders and facts the categories are computed from. `None` folders could not be resolved.
pub struct Folders<'a> {
    /// Where `.imtz` bundles are extracted (`share::bundles_root`).
    pub bundles: Option<PathBuf>,
    /// The webview's user data folder.
    pub webview: Option<PathBuf>,
    pub data: Option<PathBuf>,
    pub log: Option<PathBuf>,
    /// Today's log file (never removed).
    pub active_log: Option<PathBuf>,
    /// The runtime release IMT pins (`RuntimeCatalog::embedded().tag`).
    pub runtime_tag: String,
    /// When this process started; sessions and bundles from this run are kept. `None`: keep them all.
    pub session_start: Option<SystemTime>,
    /// This process is the single (main) instance, which owns the data folder's sessions.
    pub main_instance: bool,
    /// The previous run crashed: its sessions are kept so they can be restored.
    pub previous_crash: bool,
    pub ai: Option<&'a dyn AiPort>,
}

/// How an item is removed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum How {
    /// Deleted from disk (confined to the category's root).
    Path,
    /// Through the AI engine, which stops using it first.
    AiRuntime {
        tag: String,
        id: String,
    },
    AiModel {
        file: String,
    },
}

/// One top-level thing in a category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    pub path: PathBuf,
    pub how: How,
    /// Why it must stay; `None` when clearing may remove it.
    pub protect: Option<String>,
}

impl Item {
    fn path(path: PathBuf) -> Item {
        Item {
            path,
            how: How::Path,
            protect: None,
        }
    }

    fn protected(mut self, reason: Option<String>) -> Item {
        if self.protect.is_none() {
            self.protect = reason;
        }
        self
    }
}

/// A category with what it holds.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Category {
    pub id: CategoryId,
    /// The folder shown to the user (and opened by "Show in folder").
    pub shown: Option<PathBuf>,
    /// Everything removed must be inside this folder.
    pub root: Option<PathBuf>,
    pub items: Vec<Item>,
    /// `Err` with the reason when the category cannot be cleared at all.
    pub clearable: Result<(), String>,
    /// What clearing keeps (shown under the row).
    pub note: Option<String>,
}

impl Category {
    fn new(id: CategoryId, shown: Option<PathBuf>, root: Option<PathBuf>) -> Category {
        let clearable = match &root {
            Some(_) => Ok(()),
            None => Err("IMT cannot find this folder on this computer".to_string()),
        };
        Category {
            id,
            shown,
            root,
            items: Vec::new(),
            clearable,
            note: None,
        }
    }
}

/// The entries of `dir` (at most [`MAX_ITEMS`]), sorted; nothing when it is missing.
fn entries(dir: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| it.flatten().take(MAX_ITEMS).map(|e| e.path()).collect())
        .unwrap_or_default();
    out.sort();
    out
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn is_dir(p: &Path) -> bool {
    std::fs::symlink_metadata(p).is_ok_and(|m| m.is_dir())
}

/// The newest modification time of a file, or of the files directly in a folder (the folder's own when
/// it holds none).
fn newest(p: &Path) -> Option<SystemTime> {
    let modified = |p: &Path| std::fs::symlink_metadata(p).and_then(|m| m.modified()).ok();
    if !is_dir(p) {
        return modified(p);
    }
    let files = entries(p)
        .into_iter()
        .filter(|e| std::fs::symlink_metadata(e).is_ok_and(|m| m.is_file()));
    files.filter_map(|e| modified(&e)).max().or_else(|| modified(p))
}

/// Kept when written in this run, or when that cannot be told (unknown start or modification time).
fn from_this_run(p: &Path, start: Option<SystemTime>, what: &str) -> Option<String> {
    match (start, newest(p)) {
        (Some(start), Some(t)) if t < start => None,
        _ => Some(format!("{what} in this session")),
    }
}

/// The webview's cache folders under its user data folder, per engine. What is not listed (cookies,
/// local storage with the UI's preferences, IndexedDB) is never touched.
fn webview_cache_dirs(webview: &Path) -> Vec<PathBuf> {
    let rel: &[&str] = if cfg!(windows) {
        &[
            "EBWebView/Default/Cache",
            "EBWebView/Default/Code Cache",
            "EBWebView/Default/GPUCache",
            "EBWebView/Default/DawnGraphiteCache",
            "EBWebView/Default/DawnWebGPUCache",
            "EBWebView/GrShaderCache",
            "EBWebView/GraphiteDawnCache",
            "EBWebView/ShaderCache",
        ]
    } else if cfg!(target_os = "linux") {
        &["WebKitCache", "CacheStorage"]
    } else {
        &[]
    };
    rel.iter().map(|r| webview.join(r)).collect()
}

/// Every category, in display order. The stores entry is listed only when its folder exists.
pub fn categories(f: &Folders<'_>) -> Vec<Category> {
    let mut out = vec![
        bundle_extracts(f),
        webview_cache(f),
        ai_runtimes_outdated(f),
        ai_runtimes(f),
        ai_models(f),
        logs(f),
        error_snapshots(f),
        sessions(f),
    ];
    if let Some(stores) = stores(f) {
        out.push(stores);
    }
    out
}

fn bundle_extracts(f: &Folders<'_>) -> Category {
    let mut c = Category::new(CategoryId::BundleExtracts, f.bundles.clone(), f.bundles.clone());
    if let Some(root) = &f.bundles {
        c.items = entries(root)
            .into_iter()
            .map(|p| {
                let keep = from_this_run(&p, f.session_start, "opened");
                Item::path(p).protected(keep)
            })
            .collect();
        c.note = Some("Bundles opened in this session are kept.".into());
    }
    c
}

fn webview_cache(f: &Folders<'_>) -> Category {
    let mut c = Category::new(CategoryId::WebviewCache, f.webview.clone(), f.webview.clone());
    if let Some(root) = &f.webview {
        c.items = webview_cache_dirs(root)
            .into_iter()
            .filter(|p| p.exists())
            .map(Item::path)
            .collect();
        if cfg!(target_os = "macos") {
            c.clearable = Err("macOS manages the web view's cache".into());
        } else {
            c.note = Some("Files the running web view holds open are removed after IMT restarts.".into());
        }
    }
    c
}

fn runtimes_root(f: &Folders<'_>) -> Option<PathBuf> {
    f.data.as_ref().map(|d| d.join("ai").join("runtimes"))
}

fn ai_runtimes_outdated(f: &Folders<'_>) -> Category {
    let root = runtimes_root(f);
    let mut c = Category::new(CategoryId::AiRuntimesOutdated, root.clone(), root.clone());
    if let Some(root) = &root {
        let custom = f.ai.and_then(|a| a.custom_server());
        c.items = entries(root)
            .into_iter()
            .filter(|p| {
                let n = name_of(p);
                is_dir(p) && imt_ai::runtimes::builds::is_build_id(&n) && n != "downloads" && n != f.runtime_tag
            })
            .map(|p| {
                let keep = custom
                    .as_ref()
                    .filter(|s| s.starts_with(&p))
                    .map(|_| "the settings use its llama-server".to_string());
                Item::path(p).protected(keep)
            })
            .collect();
    }
    c
}

fn ai_runtimes(f: &Folders<'_>) -> Category {
    let builds = runtimes_root(f).map(|r| r.join(&f.runtime_tag));
    let mut c = Category::new(CategoryId::AiRuntimes, builds.clone(), builds.clone());
    let Some(dir) = &builds else { return c };
    let Some(ai) = f.ai else {
        c.clearable = Err("the AI assistant is not running in this session".into());
        return c;
    };
    let busy = ai.engine_busy();
    c.items = entries(dir)
        .into_iter()
        .filter(|p| is_dir(p) && imt_ai::runtimes::builds::is_build_id(&name_of(p)))
        .map(|p| Item {
            how: How::AiRuntime {
                tag: f.runtime_tag.clone(),
                id: name_of(&p),
            },
            path: p,
            protect: busy.clone(),
        })
        .collect();
    c.note = Some("The AI assistant downloads a runtime again when it needs one.".into());
    c
}

fn ai_models(f: &Folders<'_>) -> Category {
    let Some(ai) = f.ai else {
        let dir = f.data.as_ref().map(|d| d.join("models"));
        let mut c = Category::new(CategoryId::AiModels, dir.clone(), dir);
        c.clearable = Err("the AI assistant is not running in this session".into());
        return c;
    };
    let dir = ai.models_dir();
    let mut c = Category::new(CategoryId::AiModels, Some(dir.clone()), Some(dir.clone()));
    let busy = ai.setup_busy();
    c.items = ai
        .models()
        .into_iter()
        .take(MAX_ITEMS)
        .map(|(file, active)| Item {
            path: dir.join(&file),
            protect: busy
                .clone()
                .or_else(|| active.then(|| "the active model: choose another one or remove it in Settings ▸ AI".to_string())),
            how: How::AiModel { file },
        })
        .collect();
    c.note = Some("The active model is kept.".into());
    c
}

fn logs(f: &Folders<'_>) -> Category {
    let mut c = Category::new(CategoryId::Logs, f.log.clone(), f.log.clone());
    if let Some(dir) = &f.log {
        let prefix = format!("{}.", crate::diagnostics::LOG_NAME);
        c.items = entries(dir)
            .into_iter()
            .filter(|p| {
                let n = name_of(p);
                !is_dir(p) && n.starts_with(&prefix) && n.ends_with(".log")
            })
            .map(|p| {
                let current =
                    f.active_log.as_ref().is_some_and(|a| a == &p) || name_of(&p) == format!("{}.log", crate::diagnostics::LOG_NAME);
                Item::path(p).protected(current.then(|| "IMT is writing to it".to_string()))
            })
            .collect();
        c.note = Some("Today's log is kept.".into());
    }
    c
}

fn error_snapshots(f: &Folders<'_>) -> Category {
    let dir = f.log.as_ref().map(|d| d.join("snapshots"));
    let mut c = Category::new(CategoryId::ErrorSnapshots, dir.clone(), dir.clone());
    if let Some(dir) = &dir {
        // Images and their sidecars; the "notice shown" marker is a preference, not a snapshot.
        c.items = entries(dir)
            .into_iter()
            .filter(|p| !is_dir(p) && name_of(p) != "notice-shown")
            .map(Item::path)
            .collect();
    }
    c
}

fn sessions(f: &Folders<'_>) -> Category {
    let shown = f.data.as_ref().map(|d| d.join("sessions"));
    let mut c = Category::new(CategoryId::Sessions, shown, f.data.clone());
    let Some(data) = &f.data else { return c };
    if !f.main_instance {
        c.clearable = Err("another IMT instance owns these sessions; clear them from that window".into());
    }
    let mut items: Vec<Item> = Vec::new();
    for folder in ["sessions", "recovered"] {
        let crash = (folder == "recovered" && f.previous_crash).then(|| "kept to restore the session after the crash".to_string());
        for p in entries(&data.join(folder)).into_iter().filter(|p| is_dir(p)) {
            let keep = from_this_run(&p, f.session_start, "saved");
            items.push(Item::path(p).protected(crash.clone()).protected(keep));
        }
    }
    let legacy = data.join(SESSION_FILE);
    if legacy.is_file() {
        let keep = from_this_run(&legacy, f.session_start, "saved");
        items.push(Item::path(legacy).protected(keep));
    }
    c.items = items;
    c.note = Some("Sessions of this run are kept.".into());
    c
}

fn stores(f: &Folders<'_>) -> Option<Category> {
    let dir = f.data.as_ref().map(|d| d.join("stores")).filter(|d| d.is_dir())?;
    let mut c = Category::new(CategoryId::Stores, Some(dir.clone()), Some(dir.clone()));
    c.items = entries(&dir).into_iter().map(Item::path).collect();
    c.clearable = Err("persistent stores are removed with their project; clearing them here is not available yet".into());
    Some(c)
}

/// A category's row: sizes from a bounded traversal shared through `budget`.
pub fn usage(c: &Category, budget: &mut Budget) -> StorageCategory {
    let mut bytes = 0;
    let mut files = 0;
    let mut clearable_bytes = 0;
    let mut capped = c.items.len() >= MAX_ITEMS;
    for item in &c.items {
        let s = size_of(&item.path, budget);
        bytes += s.bytes;
        files += s.files;
        capped |= s.capped;
        if item.protect.is_none() {
            clearable_bytes += s.bytes;
        }
    }
    let clearable = c.clearable.is_ok();
    StorageCategory {
        id: c.id,
        label: c.id.label().to_string(),
        kind: c.id.kind(),
        path: c.shown.as_ref().map(|p| p.to_string_lossy().into_owned()),
        exists: c.shown.as_ref().is_some_and(|p| p.is_dir()),
        bytes,
        files,
        clearable_bytes: if clearable { clearable_bytes } else { 0 },
        capped,
        clearable,
        note: match &c.clearable {
            Err(e) => Some(e.clone()),
            Ok(()) => c.note.clone(),
        },
    }
}

/// Clears one category: every unprotected item, confined to the canonical root. `step` is called after
/// each item (progress). Things that stay are reported in the result.
pub fn clear(c: &Category, ai: Option<&dyn AiPort>, cancel: &AtomicBool, step: &mut dyn FnMut()) -> CategoryClearResult {
    if let Err(e) = &c.clearable {
        return CategoryClearResult::failed(c.id, e.clone());
    }
    let mut out = CategoryClearResult::new(c.id);
    let Some(root) = &c.root else {
        return CategoryClearResult::failed(c.id, "IMT cannot find this folder on this computer");
    };
    let root = match std::fs::canonicalize(root) {
        Ok(r) => r,
        // Nothing there yet: nothing to clear.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return out,
        Err(e) => return CategoryClearResult::failed(c.id, format!("cannot open {} ({e})", root.display())),
    };
    for item in &c.items {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        if let Some(reason) = &item.protect {
            out.skip(&item.path, reason.clone());
        } else {
            match confined(&root, &item.path) {
                Err(e) => out.skip(&item.path, e),
                Ok(path) => clear_item(&path, &item.how, ai, cancel, &mut out),
            }
        }
        step();
    }
    out
}

fn clear_item(path: &Path, how: &How, ai: Option<&dyn AiPort>, cancel: &AtomicBool, out: &mut CategoryClearResult) {
    let engine = |out: &mut CategoryClearResult, run: &dyn Fn(&dyn AiPort) -> Result<(), String>| {
        let Some(ai) = ai else {
            out.skip(path, "the AI assistant is not running in this session");
            return;
        };
        let size = size_of(path, &mut Budget::standard());
        match run(ai) {
            Ok(()) => {
                out.freed_bytes += size.bytes;
                out.removed_files += size.files;
            }
            Err(e) => out.skip(path, e),
        }
    };
    match how {
        How::Path => {
            remove(path, cancel, out);
        }
        How::AiRuntime { tag, id } => engine(out, &|ai| ai.delete_runtime(tag, id)),
        How::AiModel { file } => engine(out, &|ai| ai.delete_model(file)),
    }
}

/// The categories to clear for `ids`: unknown ids are refused, duplicates dropped.
pub fn parse_ids(ids: &[String]) -> Result<Vec<CategoryId>, String> {
    if ids.is_empty() {
        return Err("choose what to clear".into());
    }
    if ids.len() > CategoryId::ALL.len() * 2 {
        return Err("too many storage categories".into());
    }
    let mut out = Vec::new();
    for raw in ids {
        let id = CategoryId::parse(raw).ok_or_else(|| {
            let shown: String = raw.chars().filter(|c| !c.is_control()).take(40).collect();
            format!("unknown storage category '{shown}'")
        })?;
        if !out.contains(&id) {
            out.push(id);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests;
