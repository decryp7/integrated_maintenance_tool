//! Opening a diagnostics report (the zip "Report a problem" and `--export-diagnostics` write) as a
//! diagnostics workspace instead of a plain archive.
//!
//! [`divert`] runs before every open (File ▸ Open, drag and drop, the command line all go through
//! `open_paths`): a zip whose `manifest.json` says `imt-diagnostics` ([`detect`]) has only its logs
//! (IMT's, the activity log, each child process's) copied out, within the archive limits of
//! `imt_core::archive`, into `<data>/diagnostics-reports/<name>-<hash>/logs`, and that folder is opened
//! instead, so each log is a source with its own timeline lane and imports through the usual importers.
//! The JSON files are never imported as events: [`read_view`] reads them (bounded, typed) into the
//! "Diagnostics report" panel's [`DiagnosticsReportView`], and snapshot images are read from the zip on
//! demand. Nothing in the zip is ever executed. A malformed, oversized or newer manifest opens the zip as
//! a plain archive and leaves a notice for the window.
//!
//! Commands: `diagnostics_report`, `diagnostics_report_image`.

use super::manifest::{self, FileKind, Manifest, ManifestError, ManifestFile, MANIFEST_NAME};
use crate::activity::Breadcrumb;
use crate::windows::CurrentWindow;
use crate::{err, CmdResult};
use imt_ipc::child_log::ProcessSummary;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, Runtime};

/// Folder (under IMT's data folder) of the logs copied out of opened reports.
pub const REPORTS_DIR: &str = "diagnostics-reports";
/// Copied-out reports kept (the oldest are removed).
pub const KEEP_EXTRACTED: usize = 8;
/// Largest JSON file of a report read for the panel.
pub const MAX_JSON: u64 = 4 << 20;
/// Largest text file (steps, description) read.
pub const MAX_TEXT: u64 = 1 << 20;
/// Most snapshots, processes, steps and breadcrumbs shown.
pub const MAX_SNAPSHOTS: usize = 50;
pub const MAX_PROCESSES: usize = 512;
pub const MAX_STEPS: usize = 1000;
pub const MAX_BREADCRUMBS: usize = 1000;
/// Longest snapshot message or reason kept (characters).
const MAX_FIELD: usize = 4000;
/// The file next to the copied-out logs that names the zip they came from.
const SOURCE_FILE: &str = "source.json";

use crate::sync::lock;

/// The manifest as the panel shows it.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ManifestView {
    pub schema_version: u32,
    pub imt_version: String,
    pub commit: String,
    pub created_at: String,
    pub files: Vec<ManifestFile>,
}

/// One error snapshot of a report (its image comes from `diagnostics_report_image`).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReportSnapshot {
    pub id: String,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub reason: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub view: Option<String>,
    #[serde(default)]
    pub mime: String,
    #[serde(default)]
    pub bytes: u64,
    #[serde(default)]
    pub steps: Vec<String>,
    #[serde(default)]
    pub breadcrumbs: Vec<Breadcrumb>,
}

/// An opened report, read-only (see `app/src/app/diagnostics/diagnostics-report.types.ts`).
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsReportView {
    pub name: String,
    pub manifest: ManifestView,
    pub system_info: Value,
    pub processes: Vec<ProcessSummary>,
    pub steps: Vec<String>,
    pub breadcrumbs: Vec<Breadcrumb>,
    pub snapshots: Vec<ReportSnapshot>,
    pub session_recovery: Option<Value>,
    pub description: Option<String>,
    pub unreadable: Vec<String>,
}

/// What `diagnostics_report` returns.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsReportState {
    pub report: Option<DiagnosticsReportView>,
    pub notice: Option<String>,
}

/// What a zip turned out to be.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Detected {
    /// Not a diagnostics report (or not a zip): opened as usual.
    Plain,
    Report(Manifest),
    /// It looks like one but cannot be read as one: opened as a plain archive, with this notice.
    Fallback(String),
}

fn display_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

fn fallback_notice(path: &Path, why: &str) -> String {
    format!("{} was opened as a plain archive: {why}.", display_name(path))
}

/// One entry of the zip, at most `cap` bytes (by its declared and its actual size).
fn read_entry<R: Read + std::io::Seek>(zip: &mut zip::ZipArchive<R>, name: &str, cap: u64) -> Result<Vec<u8>, String> {
    let entry = zip.by_name(name).map_err(|_| format!("{name} is missing"))?;
    if entry.size() > cap {
        return Err(format!("{name} is larger than {} KB", cap / 1024));
    }
    let mut out = Vec::new();
    entry
        .take(cap + 1)
        .read_to_end(&mut out)
        .map_err(|e| format!("{name} cannot be read ({e})"))?;
    if out.len() as u64 > cap {
        return Err(format!("{name} is larger than {} KB", cap / 1024));
    }
    Ok(out)
}

fn open_zip(path: &Path) -> Result<zip::ZipArchive<std::fs::File>, String> {
    let f = std::fs::File::open(path).map_err(|e| format!("cannot be read ({})", e.kind()))?;
    zip::ZipArchive::new(f).map_err(|_| "is not a valid zip archive".to_string())
}

/// Whether `path` is a diagnostics report. Reads the zip's directory and its manifest only.
pub fn detect(path: &Path) -> Detected {
    let mut magic = [0u8; 4];
    let is_zip = std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && imt_core::archive::is_zip(&magic);
    if !is_zip {
        return Detected::Plain;
    }
    let Ok(mut zip) = open_zip(path) else { return Detected::Plain };
    let bytes = match read_entry(&mut zip, MANIFEST_NAME, manifest::MAX_MANIFEST_BYTES) {
        Ok(b) => b,
        Err(_) if zip.index_for_name(MANIFEST_NAME).is_none() => return Detected::Plain,
        Err(_) => return Detected::Fallback(fallback_notice(path, &ManifestError::TooLarge.to_string())),
    };
    match manifest::parse(&bytes) {
        Ok(m) => Detected::Report(m),
        Err(ManifestError::OtherKind) => Detected::Plain,
        // A manifest of some other program that is not even JSON.
        Err(ManifestError::Malformed(_)) if !String::from_utf8_lossy(&bytes).contains(manifest::MANIFEST_KIND) => Detected::Plain,
        Err(e) => Detected::Fallback(fallback_notice(path, &e.to_string())),
    }
}

/// `<stem>-<hash>`: one folder per zip (its canonical path, size and time).
fn folder_key(path: &Path) -> String {
    use std::hash::{Hash, Hasher};
    let canon = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let meta = std::fs::metadata(&canon).ok();
    let mut h = std::collections::hash_map::DefaultHasher::new();
    canon.hash(&mut h);
    meta.as_ref().map(|m| m.len()).hash(&mut h);
    meta.and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .hash(&mut h);
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    format!("{}-{:016x}", imt_ipc::child_log::log_name(&stem), h.finish())
}

/// Removes all but the newest [`KEEP_EXTRACTED`] copied-out reports (never `keep`). Never fails.
fn prune(reports_dir: &Path, keep: &Path) {
    let Ok(rd) = std::fs::read_dir(reports_dir) else { return };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = rd
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_dir()) && e.path() != keep)
        .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.0));
    for (_, d) in dirs.into_iter().skip(KEEP_EXTRACTED.saturating_sub(1)) {
        let _ = std::fs::remove_dir_all(d);
    }
}

/// Copies the report's logs out of the zip into `reports_dir/<key>/logs` and returns that folder.
pub fn extract_logs(zip_path: &Path, m: &Manifest, reports_dir: &Path) -> Result<PathBuf, String> {
    let root = reports_dir.join(folder_key(zip_path));
    let _ = std::fs::remove_dir_all(&root);
    let logs = root.join("logs");
    std::fs::create_dir_all(&logs).map_err(|e| format!("cannot prepare {} ({e})", logs.display()))?;
    let mut zip = open_zip(zip_path)?;
    let mut total = 0u64;
    let mut copied = 0usize;
    for f in m.files.iter().filter(|f| f.kind.is_log() && f.name.starts_with("logs/")) {
        let data = read_entry(&mut zip, &f.name, imt_core::archive::MAX_ENTRY_BYTES)?;
        total += data.len() as u64;
        if total > imt_core::archive::MAX_ARCHIVE_BYTES {
            return Err(format!(
                "its logs expand to more than {} MiB",
                imt_core::archive::MAX_ARCHIVE_BYTES >> 20
            ));
        }
        if data.is_empty() {
            continue;
        }
        // The name is a validated relative path (see `manifest::safe_name`).
        let target = root.join(&f.name);
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent).map_err(|e| format!("cannot prepare {} ({e})", parent.display()))?;
        }
        std::fs::write(&target, &data).map_err(|e| format!("cannot write {} ({e})", target.display()))?;
        copied += 1;
    }
    if copied == 0 {
        let _ = std::fs::remove_dir_all(&root);
        return Err("it contains no logs".into());
    }
    let source = serde_json::json!({ "zip": zip_path.canonicalize().unwrap_or_else(|_| zip_path.to_path_buf()) });
    let _ = std::fs::write(root.join(SOURCE_FILE), source.to_string());
    prune(reports_dir, &root);
    Ok(logs)
}

fn cut(s: String, max: usize) -> String {
    if s.chars().count() <= max {
        s
    } else {
        let mut t: String = s.chars().take(max).collect();
        t.push('…');
        t
    }
}

/// Reads the panel's data from the zip: every file is optional, bounded and parsed into its type; what
/// cannot be read is listed in `unreadable`.
pub fn read_view(zip_path: &Path, m: &Manifest) -> DiagnosticsReportView {
    let mut unreadable = Vec::new();
    let mut zip = open_zip(zip_path).map_err(|e| unreadable.push(format!("the zip {e}"))).ok();
    let mut text = |kind: FileKind, cap: u64, unreadable: &mut Vec<String>| -> Option<String> {
        let f = m.of_kind(kind).next()?;
        let z = zip.as_mut()?;
        match read_entry(z, &f.name, cap) {
            Ok(b) => Some(String::from_utf8_lossy(&b).into_owned()),
            Err(e) => {
                unreadable.push(e);
                None
            }
        }
    };
    fn json<T: serde::de::DeserializeOwned>(name: &str, t: Option<String>, unreadable: &mut Vec<String>) -> Option<T> {
        let t = t?;
        match serde_json::from_str(&t) {
            Ok(v) => Some(v),
            Err(_) => {
                unreadable.push(format!("{name} is not valid"));
                None
            }
        }
    }
    let system_info = json::<Value>(
        "system-info.json",
        text(FileKind::SystemInfo, MAX_JSON, &mut unreadable),
        &mut unreadable,
    )
    .map(|v| super::without_secrets(&v))
    .unwrap_or(Value::Null);
    let mut processes: Vec<ProcessSummary> = json(
        "processes.json",
        text(FileKind::Processes, MAX_JSON, &mut unreadable),
        &mut unreadable,
    )
    .unwrap_or_default();
    processes.truncate(MAX_PROCESSES);
    let mut breadcrumbs: Vec<Breadcrumb> = json(
        "breadcrumbs.json",
        text(FileKind::Breadcrumbs, MAX_JSON, &mut unreadable),
        &mut unreadable,
    )
    .unwrap_or_default();
    breadcrumbs.truncate(MAX_BREADCRUMBS);
    let steps: Vec<String> = text(FileKind::Steps, MAX_TEXT, &mut unreadable)
        .map(|t| {
            t.lines()
                .filter(|l| !l.trim().is_empty())
                .take(MAX_STEPS)
                .map(|l| cut(l.to_string(), MAX_FIELD))
                .collect()
        })
        .unwrap_or_default();
    let session_recovery = json::<Value>(
        "session-recovery.json",
        text(FileKind::SessionRecovery, MAX_JSON, &mut unreadable),
        &mut unreadable,
    );
    let description = text(FileKind::Description, MAX_TEXT, &mut unreadable)
        .map(|d| cut(d.trim().to_string(), super::MAX_DESCRIPTION))
        .filter(|d| !d.is_empty());
    let mut snapshots = Vec::new();
    for f in m.of_kind(FileKind::SnapshotDetails).take(MAX_SNAPSHOTS) {
        let Some(z) = zip.as_mut() else { break };
        let parsed = read_entry(z, &f.name, MAX_JSON)
            .and_then(|b| serde_json::from_slice::<ReportSnapshot>(&b).map_err(|_| format!("{} is not valid", f.name)));
        match parsed {
            Ok(mut s) if crate::snapshots::is_valid_id(&s.id) => {
                s.reason = cut(s.reason, MAX_FIELD);
                s.message = cut(s.message, MAX_FIELD);
                s.view = s.view.map(|v| cut(v, MAX_FIELD));
                s.steps.truncate(MAX_STEPS);
                s.breadcrumbs.truncate(MAX_BREADCRUMBS);
                snapshots.push(s);
            }
            Ok(_) => unreadable.push(format!("{} has an invalid id", f.name)),
            Err(e) => unreadable.push(e),
        }
    }
    DiagnosticsReportView {
        name: display_name(zip_path),
        manifest: ManifestView {
            schema_version: m.schema_version,
            imt_version: m.imt_version.clone(),
            commit: m.commit.clone(),
            created_at: m.created_at.clone(),
            files: m.files.iter().filter(|f| f.kind != FileKind::Other).cloned().collect(),
        },
        system_info,
        processes,
        steps,
        breadcrumbs,
        snapshots,
        session_recovery,
        description,
        unreadable,
    }
}

/// One snapshot image of the report as a `data:` URL (PNG or JPEG only, at most the snapshot size limit).
pub fn read_image(zip_path: &Path, m: &Manifest, id: &str) -> Result<String, String> {
    if !crate::snapshots::is_valid_id(id) {
        return Err("invalid snapshot id".into());
    }
    let f = m
        .of_kind(FileKind::Snapshot)
        .find(|f| f.name == format!("snapshots/{id}.png") || f.name == format!("snapshots/{id}.jpg"))
        .ok_or("this report has no such snapshot")?;
    let bytes = read_entry(&mut open_zip(zip_path)?, &f.name, crate::snapshots::MAX_IMAGE_BYTES as u64)?;
    let mime = if bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
        "image/png"
    } else if bytes.starts_with(&[0xFF, 0xD8, 0xFF]) {
        "image/jpeg"
    } else {
        return Err("the snapshot is not a PNG or JPEG image".into());
    };
    Ok(format!(
        "data:{mime};base64,{}",
        base64::Engine::encode(&base64::engine::general_purpose::STANDARD, bytes)
    ))
}

/// A report opened in a window.
#[derive(Debug, Clone)]
pub struct Opened {
    pub zip: PathBuf,
    pub manifest: Manifest,
    pub view: DiagnosticsReportView,
    /// The folder of its copied-out logs (opened as the workspace's sources).
    pub logs: PathBuf,
}

/// What [`divert_in`] did with a list of paths.
#[derive(Debug, Default)]
pub struct Diverted {
    pub paths: Vec<String>,
    /// The last report among them.
    pub opened: Option<Opened>,
    pub notices: Vec<String>,
}

/// Replaces each diagnostics report among `paths` by the folder of its logs (see the module docs).
pub fn divert_in(reports_dir: &Path, paths: Vec<String>) -> Diverted {
    let mut out = Diverted::default();
    for p in paths {
        let path = PathBuf::from(&p);
        if !path.is_file() {
            out.paths.push(p);
            continue;
        }
        match detect(&path) {
            Detected::Plain => out.paths.push(p),
            Detected::Fallback(notice) => {
                out.notices.push(notice);
                out.paths.push(p);
            }
            Detected::Report(m) => match extract_logs(&path, &m, reports_dir) {
                Ok(logs) => {
                    out.paths.push(logs.to_string_lossy().into_owned());
                    out.opened = Some(Opened {
                        view: read_view(&path, &m),
                        zip: path,
                        manifest: m,
                        logs,
                    });
                }
                Err(e) => {
                    out.notices.push(fallback_notice(&path, &e));
                    out.paths.push(p);
                }
            },
        }
    }
    out
}

/// The reports opened per window, and notices not yet shown. Managed on first use.
#[derive(Default)]
pub struct OpenedReports {
    opened: Mutex<HashMap<String, Arc<Opened>>>,
    notices: Mutex<HashMap<String, String>>,
}

fn reports<R: Runtime>(app: &AppHandle<R>) -> tauri::State<'_, OpenedReports> {
    if app.try_state::<OpenedReports>().is_none() {
        app.manage(OpenedReports::default());
    }
    app.state::<OpenedReports>()
}

/// A window closed: the report it opened and its pending notice go with it.
pub fn forget_window<R: Runtime>(app: &AppHandle<R>, window: &str) {
    let state = reports(app);
    lock(&state.opened).remove(window);
    lock(&state.notices).remove(window);
}

/// Called by `open_paths` before opening: diagnostics reports become the folders of their logs, and the
/// window remembers the report for its panel. Never fails: anything unexpected opens the paths as given.
pub async fn divert<R: Runtime>(app: &AppHandle<R>, window: &str, paths: Vec<String>) -> Vec<String> {
    let Ok(data) = crate::app_dirs::data(app) else { return paths };
    let given = paths.clone();
    let done = tauri::async_runtime::spawn_blocking(move || divert_in(&data.join(REPORTS_DIR), paths)).await;
    let Ok(done) = done else { return given };
    let state = reports(app);
    if let Some(n) = done.notices.last() {
        log::warn!(Notices = done.notices.len() as u64; "A zip that looked like a diagnostics report was opened as a plain archive");
        lock(&state.notices).insert(window.to_string(), n.clone());
    }
    if let Some(o) = done.opened {
        // Counts only: names stay out of the log.
        log::info!(Files = o.manifest.files.len() as u64, Processes = o.view.processes.len() as u64; "Diagnostics report opened");
        lock(&state.opened).insert(window.to_string(), Arc::new(o));
    }
    done.paths
}

/// The copied-out report a workspace source belongs to: `<reports>/<key>/logs/...` → `<reports>/<key>`.
fn report_root_of(source: &Path) -> Option<PathBuf> {
    let mut cur = source.parent();
    while let Some(dir) = cur {
        if dir.file_name().is_some_and(|n| n == "logs")
            && dir
                .parent()
                .and_then(|p| p.parent())
                .and_then(|p| p.file_name())
                .is_some_and(|n| n == REPORTS_DIR)
        {
            return dir.parent().map(Path::to_path_buf);
        }
        cur = dir.parent();
    }
    None
}

/// The report behind a restored workspace (after a restart): found from its sources' folder, read again
/// from the zip it came from when that still exists.
fn recover(sources: &[String]) -> Option<Opened> {
    let root = sources.iter().find_map(|s| report_root_of(Path::new(s)))?;
    let meta = std::fs::metadata(root.join(SOURCE_FILE)).ok()?;
    if meta.len() > 64 * 1024 {
        return None;
    }
    let v: Value = serde_json::from_str(&std::fs::read_to_string(root.join(SOURCE_FILE)).ok()?).ok()?;
    let zip = PathBuf::from(v.get("zip")?.as_str()?);
    match detect(&zip) {
        Detected::Report(m) => Some(Opened {
            view: read_view(&zip, &m),
            zip,
            manifest: m,
            logs: root.join("logs"),
        }),
        _ => None,
    }
}

/// The calling window's report, while its workspace still shows that report's logs, and a notice left by
/// the last open (shown once).
#[tauri::command]
pub async fn diagnostics_report<R: Runtime>(app: AppHandle<R>, state: CurrentWindow) -> CmdResult<DiagnosticsReportState> {
    let sources: Vec<String> = crate::workspace_commands::summary_of(&state)
        .await?
        .sources
        .into_iter()
        .map(|s| s.path)
        .collect();
    let label = state.label().to_string();
    let reports = reports(&app);
    let notice = lock(&reports.notices).remove(&label);
    let current = lock(&reports.opened).get(&label).cloned();
    let current = match current {
        Some(o) if sources.iter().any(|s| Path::new(s).starts_with(&o.logs)) => Some(o),
        _ => {
            let found = tauri::async_runtime::spawn_blocking(move || recover(&sources)).await.map_err(err)?;
            let mut opened = lock(&reports.opened);
            match found {
                Some(o) => {
                    let o = Arc::new(o);
                    opened.insert(label.clone(), o.clone());
                    Some(o)
                }
                None => {
                    opened.remove(&label);
                    None
                }
            }
        }
    };
    Ok(DiagnosticsReportState {
        report: current.map(|o| o.view.clone()),
        notice,
    })
}

/// One snapshot image of the calling window's report, as a `data:` URL.
#[tauri::command]
pub async fn diagnostics_report_image<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, id: String) -> CmdResult<String> {
    let opened = lock(&reports(&app).opened)
        .get(state.label())
        .cloned()
        .ok_or("no diagnostics report is open in this window")?;
    crate::error::lift(
        tauri::async_runtime::spawn_blocking(move || read_image(&opened.zip, &opened.manifest, &id))
            .await
            .map_err(err)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity::report::tests::{inputs_in, temp};
    use std::io::Write;

    fn report_zip(d: &Path) -> PathBuf {
        let inputs = inputs_in(d);
        let out = d.join("imt-diagnostics-test.zip");
        crate::activity::report::write(&inputs, "It froze", &out).unwrap();
        out
    }

    fn zip_with(d: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
        let p = d.join(name);
        let mut z = zip::ZipWriter::new(std::fs::File::create(&p).unwrap());
        for (n, data) in files {
            z.start_file(*n, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
        p
    }

    #[test]
    fn a_written_report_opens_as_a_diagnostics_workspace() {
        let d = temp("open-roundtrip");
        let zip = report_zip(&d);
        let Detected::Report(m) = detect(&zip) else {
            panic!("not detected")
        };
        let done = divert_in(
            &d.join(REPORTS_DIR),
            vec![
                zip.to_string_lossy().into_owned(),
                d.join("other.log").to_string_lossy().into_owned(),
            ],
        );
        assert!(done.notices.is_empty(), "{:?}", done.notices);
        let o = done.opened.unwrap();
        assert_eq!(done.paths[0], o.logs.to_string_lossy());
        assert!(done.paths[1].ends_with("other.log"));
        // Only logs were copied out.
        assert!(o.logs.join("imt.log").is_file() && o.logs.join("activity.log").is_file());
        assert!(!o.logs.join("../system-info.json").exists());
        assert_eq!(report_root_of(&o.logs.join("imt.log")).unwrap(), o.logs.parent().unwrap());
        // The panel's data.
        let v = &o.view;
        assert_eq!(v.name, "imt-diagnostics-test.zip");
        assert_eq!(v.description.as_deref(), Some("It froze"));
        assert_eq!(v.steps, ["1. Opened files from the menu", "2. Applied query from the menu"]);
        assert_eq!(v.breadcrumbs.len(), 2);
        assert_eq!(v.snapshots.len(), 1);
        assert_eq!(v.system_info["plugins"][0]["id"], "net.decryptology.web");
        assert!(v.unreadable.is_empty(), "{:?}", v.unreadable);
        assert_eq!(v.manifest.files.len(), m.files.len());
        let img = read_image(&zip, &m, &v.snapshots[0].id).unwrap();
        assert!(img.starts_with("data:image/png;base64,"));
        assert!(read_image(&zip, &m, "../x").is_err());
        assert!(read_image(&zip, &m, "20261017T083000123-1a2b3c4d").is_err());
        // After a restart the report is found again from the workspace's sources.
        let again = recover(&[o.logs.join("imt.log").to_string_lossy().into_owned()]).unwrap();
        assert_eq!(again.view, o.view);
        assert!(recover(&[d.join("x.log").to_string_lossy().into_owned()]).is_none());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn other_zips_and_bad_manifests_fall_back_to_a_plain_archive() {
        let d = temp("open-fallback");
        let good = serde_json::to_vec(&Manifest::new("1".into(), "c".into(), "t".into(), vec![])).unwrap();
        let newer = String::from_utf8(good.clone())
            .unwrap()
            .replace("\"schemaVersion\":1", "\"schemaVersion\":99");
        let plain = zip_with(&d, "plain.zip", &[("a.log", b"x")]);
        let bundle = zip_with(&d, "bundle.zip", &[("manifest.json", b"{\"kind\":\"imt-bundle\"}")]);
        let alien = zip_with(&d, "alien.zip", &[("manifest.json", b"<xml/>")]);
        let broken = zip_with(&d, "broken.zip", &[("manifest.json", b"{\"kind\":\"imt-diagnostics\",")]);
        let future = zip_with(&d, "future.zip", &[("manifest.json", newer.as_bytes())]);
        let huge_manifest = vec![b' '; manifest::MAX_MANIFEST_BYTES as usize + 10];
        let huge = zip_with(&d, "huge.zip", &[("manifest.json", &huge_manifest)]);
        let empty = zip_with(&d, "empty.zip", &[("manifest.json", &good)]);
        std::fs::write(d.join("text.zip"), "not a zip").unwrap();
        assert_eq!(detect(&plain), Detected::Plain);
        assert_eq!(detect(&bundle), Detected::Plain);
        assert_eq!(detect(&alien), Detected::Plain);
        assert_eq!(detect(&d.join("text.zip")), Detected::Plain);
        assert_eq!(detect(&d.join("missing.zip")), Detected::Plain);
        for (zip, want) in [(&broken, "not valid"), (&future, "newer IMT"), (&huge, "larger than")] {
            match detect(zip) {
                Detected::Fallback(n) => assert!(n.contains(want) && n.contains("plain archive"), "{n}"),
                other => panic!("{other:?}"),
            }
        }
        // A report without logs, or a newer one: opened as given, with a notice.
        let done = divert_in(
            &d.join(REPORTS_DIR),
            vec![empty.to_string_lossy().into_owned(), future.to_string_lossy().into_owned()],
        );
        assert!(done.opened.is_none());
        assert_eq!(done.paths.len(), 2);
        assert_eq!(done.notices.len(), 2);
        assert!(done.notices[0].contains("no logs"), "{:?}", done.notices);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn oversized_and_malformed_files_are_listed_not_fatal() {
        let d = temp("open-bad-files");
        let files = vec![
            ManifestFile {
                name: "logs/imt.log".into(),
                kind: FileKind::Log,
                bytes: 3,
            },
            ManifestFile {
                name: "processes.json".into(),
                kind: FileKind::Processes,
                bytes: 1,
            },
            ManifestFile {
                name: "system-info.json".into(),
                kind: FileKind::SystemInfo,
                bytes: 1,
            },
            ManifestFile {
                name: "snapshots/bad.json".into(),
                kind: FileKind::SnapshotDetails,
                bytes: 1,
            },
            ManifestFile {
                name: "breadcrumbs.json".into(),
                kind: FileKind::Breadcrumbs,
                bytes: 1,
            },
        ];
        let m = serde_json::to_vec(&Manifest::new("1".into(), "c".into(), "t".into(), files)).unwrap();
        let big = vec![b'['; MAX_JSON as usize + 1];
        let zip = zip_with(
            &d,
            "r.zip",
            &[
                ("manifest.json", &m),
                ("logs/imt.log", b"{}\n"),
                ("processes.json", b"not json"),
                ("system-info.json", &big),
                ("snapshots/bad.json", b"{\"id\":\"../../x\"}"),
            ],
        );
        let Detected::Report(m) = detect(&zip) else { panic!() };
        let v = read_view(&zip, &m);
        assert!(v.processes.is_empty() && v.system_info.is_null() && v.snapshots.is_empty());
        let all = v.unreadable.join(" | ");
        assert!(all.contains("processes.json is not valid"), "{all}");
        assert!(all.contains("system-info.json is larger than"), "{all}");
        assert!(all.contains("invalid id"), "{all}");
        assert!(all.contains("breadcrumbs.json is missing"), "{all}");
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn copied_out_reports_are_pruned() {
        let d = temp("open-prune");
        let reports = d.join(REPORTS_DIR);
        for i in 0..(KEEP_EXTRACTED + 3) {
            std::fs::create_dir_all(reports.join(format!("old-{i}"))).unwrap();
        }
        let zip = report_zip(&d);
        let Detected::Report(m) = detect(&zip) else { panic!() };
        let logs = extract_logs(&zip, &m, &reports).unwrap();
        let left = std::fs::read_dir(&reports).unwrap().count();
        assert_eq!(left, KEEP_EXTRACTED);
        assert!(logs.is_dir());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn commands_serve_the_windows_report() {
        let d = temp("open-cmd");
        let zip = report_zip(&d);
        let app = crate::test_support::test_app(None);
        let handle = app.handle().clone();
        let paths = tauri::async_runtime::block_on(divert(&handle, crate::windows::MAIN, vec![zip.to_string_lossy().into_owned()]));
        assert!(paths[0].ends_with("logs"), "{paths:?}");
        let win = crate::test_support::main_window(&app);
        // The workspace does not show the report's logs yet: no report.
        let st = tauri::async_runtime::block_on(diagnostics_report(handle.clone(), win)).unwrap();
        assert!(st.report.is_none());
        let win = crate::test_support::main_window(&app);
        crate::test_support::import(&win, &[PathBuf::from(&paths[0])]);
        let st = tauri::async_runtime::block_on(diagnostics_report(handle.clone(), crate::test_support::main_window(&app))).unwrap();
        let report = st.report.unwrap();
        assert!(report.processes.is_empty());
        let id = report.snapshots[0].id.clone();
        let img =
            tauri::async_runtime::block_on(diagnostics_report_image(handle.clone(), crate::test_support::main_window(&app), id)).unwrap();
        assert!(img.starts_with("data:image/png"));
        // A bad zip leaves a notice, shown once.
        let bad = zip_with(
            &d,
            "bad.zip",
            &[("manifest.json", b"{\"kind\":\"imt-diagnostics\",\"schemaVersion\":5}")],
        );
        tauri::async_runtime::block_on(divert(&handle, crate::windows::MAIN, vec![bad.to_string_lossy().into_owned()]));
        let st = tauri::async_runtime::block_on(diagnostics_report(handle.clone(), crate::test_support::main_window(&app))).unwrap();
        assert!(st.notice.unwrap().contains("newer IMT"));
        let st = tauri::async_runtime::block_on(diagnostics_report(handle.clone(), crate::test_support::main_window(&app))).unwrap();
        assert!(st.notice.is_none());
        std::fs::remove_dir_all(d).unwrap();
    }

    /// A closed window's report and notice go with it (they used to stay for the app's lifetime).
    #[test]
    fn a_closed_window_forgets_its_report() {
        let app = crate::test_support::test_app(None);
        let state = reports(app.handle());
        lock(&state.notices).insert("w-4".into(), "n".into());
        lock(&state.notices).insert("main".into(), "kept".into());
        crate::window_scoped::destroyed(app.handle(), "w-4");
        assert!(!lock(&state.notices).contains_key("w-4"));
        assert!(lock(&state.notices).contains_key("main"));
    }
}
