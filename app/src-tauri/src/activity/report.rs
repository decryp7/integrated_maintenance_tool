//! "Report a problem": one zip the user reviews and saves, to attach to a bug report. Nothing is sent
//! anywhere. It holds the activity log and a bounded window of IMT's own logs, the logs of the child
//! processes IMT supervises (AI engine, llama-server, plugin processes) with a `processes.json` summary,
//! the error snapshots (with their breadcrumbs), the steps to reproduce, system information (no secrets),
//! the last crash marker and recoverable sessions, the user's description, and a versioned `manifest.json`
//! listing every file with its kind (IMT opens such a zip as a diagnostics workspace: [`open`]).
//! Also Help ▸ Show activity log, which opens IMT's own logs filtered to the activity entries.
//!
//! The same report is written without the UI by `imt-app --export-diagnostics` ([`cli`]).
//!
//! Commands: `problem_report_preview`, `save_problem_report`, `open_activity_log`, and [`open`]'s.

mod children;
pub mod cli;
pub mod manifest;
pub mod open;
mod recovery;
#[cfg(test)]
mod sample;

use super::breadcrumbs::Breadcrumb;
use super::privacy::is_secret_key;
use crate::snapshots::{tail_of, SnapshotStore};
use crate::{err, CmdResult};
use chrono::{DateTime, Datelike, SecondsFormat, Timelike, Utc};
use imt_ipc::child_log::ProcessSummary;
use manifest::{FileKind, Manifest, ManifestFile};
use serde::Serialize;
use serde_json::{json, Value};
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::Manager;

/// IMT log bytes in a report, over the current and the newest rotated file.
pub const LOG_WINDOW: u64 = 2 * 1024 * 1024;
/// Longest description kept (bytes, cut at a character boundary).
pub const MAX_DESCRIPTION: usize = 10 * 1024;
/// Breadcrumbs the steps to reproduce are made from.
const REPORT_BREADCRUMBS: usize = 200;
/// Most error snapshots included (the newest).
const MAX_SNAPSHOTS: usize = 10;
/// Largest settings file read for the system information.
const MAX_CONFIG_FILE: u64 = 1 << 20;

/// One file of the report as previewed.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ReportEntry {
    pub name: String,
    pub bytes: u64,
    pub summary: String,
}

/// What `problem_report_preview` returns: the files the zip would hold, the steps and the system info.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct ProblemReportPreview {
    pub entries: Vec<ReportEntry>,
    pub steps: Vec<String>,
    pub system_info: Value,
}

/// A plugin as listed in the system info.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PluginLine {
    pub id: String,
    pub version: String,
    pub status: String,
}

/// Everything a report is made from (gathered from the app, by the command line, or built by tests).
pub struct ReportInputs {
    pub log_dir: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    /// IMT's data folder: the running marker and the sessions a crash restore offers.
    pub data_dir: Option<PathBuf>,
    pub snapshots: Option<SnapshotStore>,
    pub plugins: Vec<PluginLine>,
    pub breadcrumbs: Vec<Breadcrumb>,
    pub safe_mode: bool,
    /// The children as this session supervises them now (empty when none run, e.g. on the command line).
    pub live_children: Vec<ProcessSummary>,
    /// Whether this run found that the previous one did not close properly (`None`: not known here).
    pub previous_crash: Option<bool>,
    /// When the report is made (also the time of every zip entry).
    pub created_at: DateTime<Utc>,
    /// Replaces the gathered system information (reproducible samples).
    pub system_override: Option<Value>,
}

impl ReportInputs {
    /// Inputs with nothing in them, made now.
    pub fn empty() -> ReportInputs {
        ReportInputs {
            log_dir: None,
            config_dir: None,
            data_dir: None,
            snapshots: None,
            plugins: Vec::new(),
            breadcrumbs: Vec::new(),
            safe_mode: false,
            live_children: Vec::new(),
            previous_crash: None,
            created_at: Utc::now(),
            system_override: None,
        }
    }

    /// The children's log folder (`<logs>/children`).
    pub fn children_dir(&self) -> Option<PathBuf> {
        self.log_dir.as_deref().map(children_dir)
    }
}

/// The children's log folder under IMT's log folder.
pub fn children_dir(log_dir: &Path) -> PathBuf {
    log_dir.join("children")
}

/// One file of the zip.
pub(crate) struct ReportFile {
    pub name: String,
    pub kind: FileKind,
    pub data: Vec<u8>,
    pub summary: String,
}

/// The description, cut to [`MAX_DESCRIPTION`] bytes at a character boundary.
pub fn cap_description(text: &str) -> String {
    if text.len() <= MAX_DESCRIPTION {
        return text.to_string();
    }
    let mut end = MAX_DESCRIPTION;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    format!("{}…", &text[..end])
}

/// Removes every value whose key names a secret, at any depth.
pub fn without_secrets(v: &Value) -> Value {
    match v {
        Value::Object(map) => Value::Object(
            map.iter()
                .filter(|(k, _)| !is_secret_key(k))
                .map(|(k, v)| (k.clone(), without_secrets(v)))
                .collect(),
        ),
        Value::Array(items) => Value::Array(items.iter().map(without_secrets).collect()),
        other => other.clone(),
    }
}

/// IMT's own log files, newest first: today's daily file, then older daily files (and earlier versions'
/// `imt.log`, `imt_*.log`) by modification time.
pub fn log_files(log_dir: &Path) -> Vec<PathBuf> {
    let name = crate::diagnostics::LOG_NAME;
    let active = crate::logging::active_log_file(log_dir);
    let mut rotated: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(log_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.starts_with(name) && n.ends_with(".log") && e.path() != active && e.path().is_file()
                })
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect()
        })
        .unwrap_or_default();
    rotated.sort_by(|(ta, pa), (tb, pb)| tb.cmp(ta).then_with(|| pb.cmp(pa)));
    let mut out = Vec::new();
    if active.is_file() {
        out.push(active);
    }
    out.extend(rotated.into_iter().map(|(_, p)| p));
    out
}

/// The newest [`LOG_WINDOW`] bytes of IMT's logs: the current file's tail, then the newest rotated file
/// with what is left. Returns (file name, bytes), newest first.
fn log_window(log_dir: &Path) -> Vec<(String, Vec<u8>)> {
    let mut left = LOG_WINDOW;
    let mut out = Vec::new();
    for path in log_files(log_dir).into_iter().take(2) {
        if left == 0 {
            break;
        }
        if let Some(tail) = tail_of(&path, left) {
            left = left.saturating_sub(tail.len() as u64);
            let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            out.push((name, tail));
        }
    }
    out
}

/// Whether a CLEF line is an activity entry (`SourceContext = IMT.Activity`).
fn activity_entry(line: &str) -> Option<Value> {
    let v = serde_json::from_str::<Value>(line).ok()?;
    (v.get("SourceContext").and_then(Value::as_str) == Some(super::SOURCE_CONTEXT)).then_some(v)
}

/// The activity entries (`SourceContext = IMT.Activity`) among CLEF lines, oldest first.
fn activity_lines(logs: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    // Oldest file first, so the result reads in time order.
    for (_, bytes) in logs.iter().rev() {
        for line in String::from_utf8_lossy(bytes).lines() {
            if activity_entry(line).is_some() {
                out.extend_from_slice(line.as_bytes());
                out.push(b'\n');
            }
        }
    }
    out
}

/// Breadcrumbs rebuilt from the activity entries of a log (the newest `max`), for a report made when the
/// session that logged them is gone (the command line).
pub fn breadcrumbs_from_log(activity: &[u8], max: usize) -> Vec<Breadcrumb> {
    let mut out: Vec<Breadcrumb> = String::from_utf8_lossy(activity)
        .lines()
        .filter_map(activity_entry)
        .filter_map(|v| {
            let time = v.get("@t").and_then(Value::as_str)?.to_string();
            let props = v
                .as_object()?
                .iter()
                .filter(|(k, _)| !k.starts_with('@'))
                .map(|(k, v)| {
                    let s = match v {
                        Value::String(s) => s.clone(),
                        other => other.to_string(),
                    };
                    (k.clone(), s)
                })
                .collect();
            Breadcrumb::from_props(time, &props)
        })
        .collect();
    let skip = out.len().saturating_sub(max);
    out.split_off(skip)
}

/// A settings file: missing is fine (`None`); too large or not JSON is a reason (never its content).
fn read_config(path: &Path) -> Result<Option<Value>, String> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("cannot be read ({})", e.kind())),
    };
    if meta.len() > MAX_CONFIG_FILE {
        return Err(format!("is larger than {} KB", MAX_CONFIG_FILE / 1024));
    }
    let text = std::fs::read_to_string(path).map_err(|e| format!("cannot be read ({})", e.kind()))?;
    serde_json::from_str(&text)
        .map(Some)
        .map_err(|e| format!("is not valid JSON (line {}, column {})", e.line(), e.column()))
}

/// OS, build, memory, plugins and settings (secrets removed), and the settings files that could not be read.
pub fn system_info(inputs: &ReportInputs) -> Value {
    if let Some(fixed) = &inputs.system_override {
        return fixed.clone();
    }
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let mut unreadable = Vec::new();
    let settings = inputs.config_dir.as_deref().map(|d| {
        let mut section = serde_json::Map::new();
        for (key, file) in [
            ("settings", "settings.json"),
            ("diagnostics", "diagnostics.json"),
            ("plugins", "plugins.json"),
        ] {
            let value = match read_config(&d.join(file)) {
                Ok(v) => v.map(|v| without_secrets(&v)),
                Err(why) => {
                    unreadable.push(json!({ "file": file, "reason": why }));
                    None
                }
            };
            section.insert(key.into(), value.unwrap_or(Value::Null));
        }
        Value::Object(section)
    });
    json!({
        "appVersion": env!("CARGO_PKG_VERSION"),
        "commit": super::build_commit(),
        "sessionId": super::session_id(),
        "os": std::env::consts::OS,
        "osVersion": sysinfo::System::long_os_version(),
        "arch": std::env::consts::ARCH,
        "cpuCores": std::thread::available_parallelism().map(|n| n.get()).unwrap_or(0),
        "totalMemoryMb": sys.total_memory() / (1024 * 1024),
        "availableMemoryMb": sys.available_memory() / (1024 * 1024),
        "safeMode": inputs.safe_mode,
        "activityDetail": format!("{:?}", super::config().detail).to_lowercase(),
        "plugins": inputs.plugins,
        "configuration": settings,
        "unreadable": unreadable,
    })
}

/// The settings files the system information could not read, as "file: reason".
pub fn unreadable_settings(info: &Value) -> Vec<String> {
    info.get("unreadable")
        .and_then(Value::as_array)
        .map(|items| {
            items
                .iter()
                .filter_map(|u| Some(format!("{} {}", u.get("file")?.as_str()?, u.get("reason")?.as_str()?)))
                .collect()
        })
        .unwrap_or_default()
}

/// The zip's files, in order, without the manifest.
fn report_files(inputs: &ReportInputs, description: &str) -> Vec<ReportFile> {
    let mut files = Vec::new();
    let description = cap_description(description.trim());
    files.push(ReportFile {
        name: "description.txt".into(),
        kind: FileKind::Description,
        summary: if description.is_empty() {
            "No description".into()
        } else {
            format!("Your description ({} characters)", description.chars().count())
        },
        data: description.into_bytes(),
    });
    let steps = super::steps_text(&inputs.breadcrumbs, None);
    files.push(ReportFile {
        name: "steps-to-reproduce.txt".into(),
        kind: FileKind::Steps,
        summary: format!("{} steps from your recent activity", super::steps(&inputs.breadcrumbs, None).len()),
        data: steps.into_bytes(),
    });
    files.push(ReportFile {
        name: "breadcrumbs.json".into(),
        kind: FileKind::Breadcrumbs,
        summary: format!("{} recent operations (no file contents)", inputs.breadcrumbs.len()),
        data: serde_json::to_vec_pretty(&inputs.breadcrumbs).unwrap_or_default(),
    });
    let info = system_info(inputs);
    let unreadable = unreadable_settings(&info);
    files.push(ReportFile {
        name: "system-info.json".into(),
        kind: FileKind::SystemInfo,
        summary: if unreadable.is_empty() {
            "OS, IMT version and build, memory, plugins, settings without secrets".into()
        } else {
            format!(
                "OS, IMT version and build, memory, plugins, settings without secrets ({} unreadable)",
                unreadable.len()
            )
        },
        data: serde_json::to_vec_pretty(&info).unwrap_or_default(),
    });
    let kids = children::collect(inputs.children_dir().as_deref(), &inputs.live_children);
    files.push(ReportFile {
        name: "processes.json".into(),
        kind: FileKind::Processes,
        summary: children::processes_summary(&kids.processes),
        data: serde_json::to_vec_pretty(&kids.processes).unwrap_or_default(),
    });
    let recovery = recovery::session_recovery(inputs.data_dir.as_deref(), inputs.previous_crash);
    files.push(ReportFile {
        name: "session-recovery.json".into(),
        kind: FileKind::SessionRecovery,
        summary: recovery::summary(&recovery),
        data: serde_json::to_vec_pretty(&recovery).unwrap_or_default(),
    });
    let logs = inputs.log_dir.as_deref().map(log_window).unwrap_or_default();
    let activity = activity_lines(&logs);
    files.push(ReportFile {
        name: "logs/activity.log".into(),
        kind: FileKind::Activity,
        summary: format!("{} activity entries", activity.iter().filter(|b| **b == b'\n').count()),
        data: activity,
    });
    for (name, bytes) in logs {
        files.push(ReportFile {
            name: format!("logs/{name}"),
            kind: FileKind::Log,
            summary: "IMT's own log (recent part)".into(),
            data: bytes,
        });
    }
    for log in kids.logs {
        files.push(ReportFile {
            name: format!("logs/children/{}", log.file),
            kind: FileKind::ChildLog,
            summary: log.summary,
            data: log.data,
        });
    }
    if let Some(store) = &inputs.snapshots {
        let all = store.list();
        let newest = &all[all.len().saturating_sub(MAX_SNAPSHOTS)..];
        for info in newest {
            if let Ok((format, image)) = store.read_image(&info.id) {
                files.push(ReportFile {
                    name: format!("snapshots/{}.{}", info.id, format.extension()),
                    kind: FileKind::Snapshot,
                    summary: format!("Screenshot: {}", info.reason),
                    data: image,
                });
            }
            let mut sidecar = serde_json::to_vec_pretty(info).unwrap_or_default();
            sidecar.push(b'\n');
            files.push(ReportFile {
                name: format!("snapshots/{}.json", info.id),
                kind: FileKind::SnapshotDetails,
                summary: format!("Error details and {} breadcrumbs", info.breadcrumbs.len()),
                data: sidecar,
            });
        }
    }
    files
}

/// Every file of the zip: `manifest.json` (listing the others) first.
pub(crate) fn all_files(inputs: &ReportInputs, description: &str) -> Vec<ReportFile> {
    let files = report_files(inputs, description);
    let info = system_info(inputs);
    let text = |k: &str| info.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
    let manifest = Manifest::new(
        text("appVersion"),
        text("commit"),
        inputs.created_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        files
            .iter()
            .map(|f| ManifestFile {
                name: f.name.clone(),
                kind: f.kind,
                bytes: f.data.len() as u64,
            })
            .collect(),
    );
    let mut all = Vec::with_capacity(files.len() + 1);
    all.push(ReportFile {
        name: manifest::MANIFEST_NAME.into(),
        kind: FileKind::Other,
        summary: format!("Index of the {} files (IMT opens this zip as a diagnostics report)", files.len()),
        data: serde_json::to_vec_pretty(&manifest).unwrap_or_default(),
    });
    all.extend(files);
    all
}

/// The preview the user reviews before saving.
pub fn preview(inputs: &ReportInputs, description: &str) -> ProblemReportPreview {
    ProblemReportPreview {
        entries: all_files(inputs, description)
            .into_iter()
            .map(|f| ReportEntry {
                name: f.name,
                bytes: f.data.len() as u64,
                summary: f.summary,
            })
            .collect(),
        steps: super::steps(&inputs.breadcrumbs, None),
        system_info: system_info(inputs),
    }
}

/// A zip entry time (zip times are local wall clock without a zone; IMT writes UTC). Clamped to the
/// format's range (1980..2107).
fn zip_time(t: &DateTime<Utc>) -> zip::DateTime {
    let year = t.year().clamp(1980, 2107) as u16;
    zip::DateTime::from_date_and_time(
        year,
        t.month() as u8,
        t.day() as u8,
        t.hour() as u8,
        t.minute() as u8,
        t.second() as u8,
    )
    .unwrap_or_default()
}

/// Writes the report to `dest` (a `.zip` in an existing folder). Returns the number of files written.
/// A failed write leaves no partial file. Entries are written in a fixed order with the report's time, so
/// the same inputs give the same bytes.
pub fn write(inputs: &ReportInputs, description: &str, dest: &Path) -> anyhow::Result<usize> {
    let dest = crate::snapshots::checked_destination(dest).map_err(|e| anyhow::anyhow!("{e}"))?;
    let result = (|| -> anyhow::Result<usize> {
        let files = all_files(inputs, description);
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&dest)?);
        // The zip crate records the writing platform (DOS on Windows, Unix elsewhere) unless told; fixing it keeps
        // a report byte-identical wherever it is built (the committed sample is checked on every CI platform).
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip_time(&inputs.created_at))
            .system(zip::System::Unix)
            .unix_permissions(0o644);
        for f in &files {
            zip.start_file(f.name.as_str(), opts)?;
            zip.write_all(&f.data)?;
        }
        zip.finish()?;
        Ok(files.len())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&dest);
    }
    result
}

fn inputs<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> ReportInputs {
    let plugins = app
        .try_state::<crate::windows::Windows>()
        .and_then(|w| w.all().into_iter().next())
        .map(|w| {
            w.store_read()
                .summary()
                .plugins
                .into_iter()
                .map(|p| PluginLine {
                    id: p.id,
                    version: p.version,
                    status: serde_json::to_value(p.status)
                        .ok()
                        .and_then(|v| v.as_str().map(String::from))
                        .unwrap_or_default(),
                })
                .collect()
        })
        .unwrap_or_default();
    let life = app.try_state::<crate::lifecycle::Lifecycle>();
    ReportInputs {
        log_dir: crate::app_dirs::log(app).ok(),
        config_dir: crate::snapshots::config_dir(app),
        data_dir: crate::app_dirs::data(app).ok(),
        snapshots: crate::snapshots::store(app).ok(),
        plugins,
        breadcrumbs: super::breadcrumbs(None, REPORT_BREADCRUMBS),
        safe_mode: life.as_ref().is_some_and(|l| l.safe_mode),
        live_children: imt_ipc::child_log::statuses(),
        previous_crash: life.as_ref().map(|l| l.previous_crash),
        created_at: Utc::now(),
        system_override: None,
    }
}

/// What a problem report would contain, for the user to review.
#[tauri::command]
pub async fn problem_report_preview<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    description: Option<String>,
) -> CmdResult<ProblemReportPreview> {
    let inputs = inputs(&app);
    tauri::async_runtime::spawn_blocking(move || preview(&inputs, description.as_deref().unwrap_or("")))
        .await
        .map_err(err)
}

/// Writes the problem report to `path` (chosen in the save dialog). Refuses to replace a file unless
/// `overwrite`. Returns the number of files in the zip.
#[tauri::command]
pub async fn save_problem_report<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    path: String,
    description: Option<String>,
    overwrite: Option<bool>,
) -> CmdResult<usize> {
    imt_core::overwrite::check_overwrite(Path::new(&path), overwrite.unwrap_or(false)).map_err(err)?;
    let inputs = inputs(&app);
    let count = tauri::async_runtime::spawn_blocking(move || write(&inputs, description.as_deref().unwrap_or(""), Path::new(&path)))
        .await
        .map_err(err)?
        .map_err(err)?;
    log::info!(Files = count as u64; "Problem report saved");
    Ok(count)
}

/// File name of the project Help ▸ Show activity log opens (rewritten each time).
pub const ACTIVITY_PROJECT: &str = "activity-log.imtproj";

/// A project over IMT's own log files whose saved query keeps only the activity entries
/// (`SourceContext = IMT.Activity`): the UI restores it like any project, so the new window opens filtered.
pub fn activity_project(logs: &[PathBuf]) -> imt_core::project::ProjectFile {
    let activity = json!({ "field": "SourceContext", "op": "eq", "value": super::SOURCE_CONTEXT });
    imt_core::project::ProjectFile {
        version: imt_core::project::PROJECT_VERSION,
        name: "IMT activity log".into(),
        sources: logs.iter().map(|p| p.to_string_lossy().into_owned()).collect(),
        profile: imt_core::profile::DEFAULT_PROFILE_TOML.into(),
        ui: json!({
            "query": { "query": { "filters": [activity] } },
            // Presets in the saved-queries list of that window: the activity, its errors, everything.
            "savedQueries": [
                { "id": "imt-activity", "name": "Activity", "query": { "filters": [activity] } },
                { "id": "imt-activity-errors", "name": "Activity errors", "query": { "filters": [activity], "levels": ["Warning", "Error", "Fatal"] } },
                { "id": "imt-all", "name": "All of IMT's log", "query": {} },
            ],
        }),
        ..Default::default()
    }
}

/// Writes the activity project into `data_dir` and returns its path.
pub fn write_activity_project(data_dir: &Path, log_dir: &Path) -> anyhow::Result<PathBuf> {
    let logs = log_files(log_dir);
    anyhow::ensure!(!logs.is_empty(), "IMT has not written a log yet ({})", log_dir.display());
    std::fs::create_dir_all(data_dir)?;
    let path = data_dir.join(ACTIVITY_PROJECT);
    activity_project(&logs).save(&path)?;
    Ok(path)
}

/// Help ▸ Show activity log: opens IMT's own logs (they are CLEF, so IMT reads them) in a new window,
/// filtered to the activity entries. Returns the new window's label.
#[tauri::command]
pub fn open_activity_log<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> CmdResult<String> {
    let log_dir = crate::app_dirs::log(&app).map_err(err)?;
    let data_dir = crate::app_dirs::data(&app).map_err(err)?;
    let project = write_activity_project(&data_dir, &log_dir).map_err(err)?;
    crate::window_manager::open_new(&app, vec![project.to_string_lossy().into_owned()]).map_err(err)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::snapshots::{Environment, ImageFormat, Limits, SnapshotContext};
    use std::collections::BTreeMap;

    pub(crate) fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("imt-report-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn crumb(op: &str) -> Breadcrumb {
        Breadcrumb {
            time: "2026-10-17T08:00:00Z".into(),
            operation: op.into(),
            trigger: "menu".into(),
            operation_id: None,
            outcome: None,
            error_code: None,
            duration_ms: None,
            view: None,
            window: "main".into(),
            params: BTreeMap::new(),
        }
    }

    pub(crate) fn inputs_in(d: &Path) -> ReportInputs {
        let logs = d.join("logs");
        let config = d.join("config");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            logs.join("imt.log"),
            "{\"@t\":\"2026-10-17T08:00:00Z\",\"@m\":\"started\",\"SourceContext\":\"imt_app\"}\n{\"@t\":\"2026-10-17T08:00:01Z\",\"@m\":\"User export.csv via menu\",\"SourceContext\":\"IMT.Activity\",\"Operation\":\"export.csv\",\"Trigger\":\"menu\",\"Window\":\"main\",\"DurationMs\":12}\n",
        )
        .unwrap();
        std::fs::write(
            config.join("settings.json"),
            r#"{"version":1,"sections":{"ai":{"serviceApiKey":"sk-secret","model":"small"},"web":{"token":"t0k"}}}"#,
        )
        .unwrap();
        let store = SnapshotStore::new(logs.join("snapshots"), Some(logs.join("imt.log")), Limits::default());
        let ctx = SnapshotContext {
            reason: "panel-failed".into(),
            message: "boom".into(),
            breadcrumbs: vec![crumb("file.open")],
            ..Default::default()
        };
        let png = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1];
        store
            .save(
                ImageFormat::Png,
                &png,
                &ctx,
                &Environment {
                    app_version: "1".into(),
                    os: "x".into(),
                    arch: "y".into(),
                },
            )
            .unwrap();
        ReportInputs {
            log_dir: Some(logs),
            config_dir: Some(config),
            snapshots: Some(store),
            plugins: vec![PluginLine {
                id: "net.decryptology.web".into(),
                version: "1.0.0".into(),
                status: "active".into(),
            }],
            breadcrumbs: vec![crumb("file.open"), crumb("query.change")],
            ..ReportInputs::empty()
        }
    }

    pub(crate) fn zip_text(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> String {
        let mut s = String::new();
        std::io::Read::read_to_string(&mut zip.by_name(name).unwrap(), &mut s).unwrap();
        s
    }

    #[test]
    fn report_zip_contents_and_redaction() {
        let d = temp("zip");
        let inputs = inputs_in(&d);
        let p = preview(&inputs, "It crashed when I exported");
        let names: Vec<&str> = p.entries.iter().map(|e| e.name.as_str()).collect();
        for want in [
            "manifest.json",
            "description.txt",
            "steps-to-reproduce.txt",
            "breadcrumbs.json",
            "system-info.json",
            "processes.json",
            "session-recovery.json",
            "logs/activity.log",
            "logs/imt.log",
        ] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
        }
        assert_eq!(names[0], "manifest.json");
        assert!(names.iter().any(|n| n.starts_with("snapshots/") && n.ends_with(".png")));
        assert_eq!(p.steps, vec!["1. Opened files from the menu", "2. Applied query from the menu"]);
        assert_eq!(p.system_info["plugins"][0]["id"], "net.decryptology.web");

        let out = d.join("report.zip");
        let n = write(&inputs, "It crashed when I exported", &out).unwrap();
        assert_eq!(n, p.entries.len());
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        assert_eq!(zip_text(&mut zip, "description.txt"), "It crashed when I exported");
        let info = zip_text(&mut zip, "system-info.json");
        assert!(
            !info.contains("sk-secret") && !info.contains("t0k") && !info.contains("serviceApiKey"),
            "{info}"
        );
        assert!(info.contains("\"model\": \"small\"") && info.contains("sessionId") && info.contains("commit"));
        let activity = zip_text(&mut zip, "logs/activity.log");
        assert!(activity.contains("User export.csv") && !activity.contains("started"));
        assert!(zip_text(&mut zip, "steps-to-reproduce.txt").starts_with("1. Opened files"));
        let sidecar = zip
            .file_names()
            .filter_map(Result::ok)
            .find(|n| n.starts_with("snapshots/") && n.ends_with(".json"))
            .unwrap()
            .into_owned();
        assert!(zip_text(&mut zip, &sidecar).contains("\"breadcrumbs\""));
        // The manifest lists every other file with its kind and size.
        let m = manifest::parse(zip_text(&mut zip, "manifest.json").as_bytes()).unwrap();
        assert_eq!(m.files.len(), n - 1);
        assert_eq!(m.imt_version, env!("CARGO_PKG_VERSION"));
        let kind = |name: &str| m.files.iter().find(|f| f.name == name).map(|f| f.kind);
        assert_eq!(kind("logs/activity.log"), Some(FileKind::Activity));
        assert_eq!(kind("logs/imt.log"), Some(FileKind::Log));
        assert_eq!(kind(&sidecar), Some(FileKind::SnapshotDetails));
        assert_eq!(
            m.files.iter().find(|f| f.name == "description.txt").unwrap().bytes,
            "It crashed when I exported".len() as u64
        );
        // Not a zip, or a missing folder: refused and nothing written.
        assert!(write(&inputs, "", &d.join("report.txt")).is_err());
        assert!(write(&inputs, "", &d.join("missing/r.zip")).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn child_logs_and_processes_are_in_the_report_and_capped() {
        use imt_ipc::child_log::{ChildEvent, ChildKind, ChildLog, ChildLogSpec, Exit};
        let d = temp("children");
        let mut inputs = inputs_in(&d);
        inputs.snapshots = None;
        let dir = inputs.children_dir().unwrap();
        let worker = ChildLog::open(Some(&dir), &ChildLogSpec::new("ai-worker", ChildKind::AiWorker));
        worker.record(&ChildEvent::Start { pid: 10 });
        worker.record(&ChildEvent::Exit {
            pid: Some(10),
            exit: Exit::Code(3),
        });
        worker.record(&ChildEvent::Start { pid: 11 });
        // A big log of another child: only its tail goes in.
        let noisy = ChildLog::open(Some(&dir), &ChildLogSpec::new("plugin-noisy", ChildKind::Plugin));
        noisy.record(&ChildEvent::Start { pid: 12 });
        for i in 0..2000 {
            noisy.record(&ChildEvent::Output(format!("{i} {}", "y".repeat(300))));
        }
        // And a previous session's file.
        std::fs::write(dir.join("ai-worker.1.log"), "{\"Event\":\"start\",\"Pid\":1}\n").unwrap();
        std::fs::write(dir.join("../children/not-a-log.txt"), "x").unwrap();
        // The worker runs now: its live state wins over its file.
        inputs.live_children = vec![worker.status()];
        let p = preview(&inputs, "");
        let entry = |n: &str| p.entries.iter().find(|e| e.name == n).cloned();
        let w = entry("logs/children/ai-worker.log").unwrap();
        assert_eq!(w.summary, "AI engine: 2 runs, 1 restart, last exit code 3, running");
        let noisy_entry = entry("logs/children/plugin-noisy.log").unwrap();
        assert!(noisy_entry.bytes <= children::CHILD_LOG_TAIL, "{}", noisy_entry.bytes);
        assert!(entry("logs/children/ai-worker.1.log").unwrap().summary.contains("previous session"));
        assert!(entry("logs/children/not-a-log.txt").is_none());
        let total: u64 = p
            .entries
            .iter()
            .filter(|e| e.name.starts_with("logs/children/"))
            .map(|e| e.bytes)
            .sum();
        assert!(total <= children::CHILD_LOGS_TOTAL);
        let processes = entry("processes.json").unwrap();
        assert_eq!(processes.summary, "2 child processes, 1 restart, 0 quarantined");

        let out = d.join("r.zip");
        write(&inputs, "", &out).unwrap();
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let procs: Vec<ProcessSummary> = serde_json::from_str(&zip_text(&mut zip, "processes.json")).unwrap();
        let w = procs.iter().find(|p| p.name == "ai-worker").unwrap();
        assert_eq!(
            (w.state, w.restarts, w.pid),
            (imt_ipc::child_log::ChildRunState::Running, 1, Some(11))
        );
        // The noisy plugin is not running live: its file ends on a start, so its state is unknown.
        let n = procs.iter().find(|p| p.name == "plugin-noisy").unwrap();
        assert_eq!(n.state, imt_ipc::child_log::ChildRunState::Unknown);
        // Its tail starts at a whole line.
        let tail = zip_text(&mut zip, "logs/children/plugin-noisy.log");
        assert!(tail.lines().all(|l| serde_json::from_str::<Value>(l).is_ok()));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn corrupt_settings_are_listed_not_fatal() {
        let d = temp("corrupt");
        let mut inputs = inputs_in(&d);
        inputs.snapshots = None;
        let config = inputs.config_dir.clone().unwrap();
        std::fs::write(config.join("settings.json"), "{ not json").unwrap();
        std::fs::write(config.join("plugins.json"), vec![b' '; (MAX_CONFIG_FILE + 1) as usize]).unwrap();
        let info = system_info(&inputs);
        let bad = unreadable_settings(&info);
        assert_eq!(bad.len(), 2, "{bad:?}");
        assert!(bad[0].starts_with("settings.json is not valid JSON"));
        assert!(bad[1].starts_with("plugins.json is larger than"));
        assert!(info["configuration"]["settings"].is_null());
        let out = d.join("r.zip");
        assert!(write(&inputs, "", &out).is_ok());
        let p = preview(&inputs, "");
        assert!(p
            .entries
            .iter()
            .any(|e| e.name == "system-info.json" && e.summary.contains("2 unreadable")));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn breadcrumbs_are_rebuilt_from_the_activity_log() {
        let lines = b"{\"@t\":\"2026-10-14T09:00:00Z\",\"SourceContext\":\"IMT.Activity\",\"Operation\":\"file.open\",\"Trigger\":\"menu\",\"Window\":\"main\",\"DurationMs\":5,\"Files\":3}\n\
            {\"@t\":\"2026-10-14T09:00:01Z\",\"SourceContext\":\"imt_app\",\"Operation\":\"x\"}\n\
            not json\n\
            {\"@t\":\"2026-10-14T09:00:02Z\",\"SourceContext\":\"IMT.Activity\",\"Operation\":\"query.change\",\"Trigger\":\"keyboard\"}\n";
        let b = breadcrumbs_from_log(lines, 10);
        assert_eq!(b.len(), 2);
        assert_eq!((b[0].operation.as_str(), b[0].duration_ms), ("file.open", Some(5)));
        assert_eq!(b[0].params.get("Files").map(String::as_str), Some("3"));
        assert_eq!(breadcrumbs_from_log(lines, 1)[0].operation, "query.change");
    }

    #[test]
    fn descriptions_are_capped_and_empty_ones_noted() {
        let long = "é".repeat(MAX_DESCRIPTION);
        let capped = cap_description(&long);
        assert!(capped.len() <= MAX_DESCRIPTION + '…'.len_utf8() && capped.ends_with('…'));
        assert_eq!(cap_description("short"), "short");
        let d = temp("desc");
        let mut inputs = inputs_in(&d);
        inputs.snapshots = None;
        inputs.log_dir = None;
        inputs.config_dir = None;
        let p = preview(&inputs, "   ");
        let desc = p.entries.iter().find(|e| e.name == "description.txt").unwrap();
        assert_eq!(desc.summary, "No description");
        assert_eq!(desc.bytes, 0);
        assert!(p.system_info["configuration"].is_null());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn zip_times_are_clamped_to_the_format() {
        let t = DateTime::parse_from_rfc3339("1970-01-01T00:00:00Z").unwrap().with_timezone(&Utc);
        assert_eq!(zip_time(&t).year(), 1980);
        let t = DateTime::parse_from_rfc3339("2026-10-14T09:30:15Z").unwrap().with_timezone(&Utc);
        let z = zip_time(&t);
        assert_eq!(
            (z.year(), z.month(), z.day(), z.hour(), z.minute(), z.second()),
            (2026, 10, 14, 9, 30, 14)
        );
    }

    #[test]
    fn the_log_window_is_bounded_to_the_newest_two_files() {
        let d = temp("window");
        let line = format!("{{\"@m\":\"{}\"}}\n", "x".repeat(1000));
        let big = line.repeat((LOG_WINDOW as usize / line.len()) + 100);
        std::fs::write(d.join("imt_old.log"), "old\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(d.join("imt.2000-01-02.log"), "newer\n").unwrap();
        let today = crate::logging::active_log_file(&d);
        let today_name = today.file_name().unwrap().to_string_lossy().into_owned();
        std::fs::write(&today, &big).unwrap();
        std::fs::write(d.join("other.log"), "x").unwrap();
        let files = log_files(&d);
        assert_eq!(files.len(), 3);
        assert!(files[0] == today && files[1].ends_with("imt.2000-01-02.log"), "{files:?}");
        let w = log_window(&d);
        let total: usize = w.iter().map(|(_, b)| b.len()).sum();
        assert!(total as u64 <= LOG_WINDOW, "{total}");
        assert_eq!(w[0].0, today_name);
        // A small current file leaves room for the newest older one, never the oldest.
        std::fs::write(&today, "now\n").unwrap();
        let names: Vec<String> = log_window(&d).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, [today_name.as_str(), "imt.2000-01-02.log"]);
        assert!(log_files(&d.join("missing")).is_empty());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn the_activity_project_opens_the_logs_filtered() {
        let d = temp("project");
        let logs = d.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        assert!(write_activity_project(&d.join("data"), &logs).is_err(), "no log yet");
        std::fs::write(logs.join("imt.log"), "{}\n").unwrap();
        let path = write_activity_project(&d.join("data"), &logs).unwrap();
        let p = imt_core::project::ProjectFile::load(&path).unwrap();
        assert_eq!(p.sources.len(), 1);
        assert!(p.sources[0].ends_with("imt.log"));
        assert_eq!(p.ui["query"]["query"]["filters"][0]["value"], "IMT.Activity");
        assert_eq!(p.ui["query"]["query"]["filters"][0]["field"], "SourceContext");
        assert_eq!(p.ui["savedQueries"][0]["name"], "Activity");
        assert!(imt_core::profile::Profile::from_toml(&p.profile).is_ok());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn secrets_are_removed_at_any_depth() {
        let v = json!({"a": {"apiKey": "x", "list": [{"password": "p", "ok": 1}]}, "Token": "t", "keep": true});
        assert_eq!(without_secrets(&v), json!({"a": {"list": [{"ok": 1}]}, "keep": true}));
    }

    #[test]
    fn commands_against_a_mock_app() {
        let _g = crate::activity::test_capture();
        log::info!(target: super::super::SOURCE_CONTEXT, SourceContext = super::super::SOURCE_CONTEXT, Operation = "report.test", Trigger = "menu", Window = "main"; "User report.test via menu");
        let app = crate::test_support::test_app(None);
        let handle = app.handle().clone();
        let p = tauri::async_runtime::block_on(problem_report_preview(handle.clone(), Some("d".into()))).unwrap();
        assert!(p.steps.iter().any(|s| s.contains("report.test")), "{:?}", p.steps);
        assert!(!p.system_info["plugins"].as_array().unwrap().is_empty());
        let out = std::env::temp_dir().join(format!("imt-report-cmd-{}.zip", std::process::id()));
        let _ = std::fs::remove_file(&out);
        let n = tauri::async_runtime::block_on(save_problem_report(handle.clone(), out.to_string_lossy().into(), None, None)).unwrap();
        assert!(n >= 5);
        let refused = tauri::async_runtime::block_on(save_problem_report(handle.clone(), out.to_string_lossy().into(), None, None));
        assert_eq!(
            refused.unwrap_err(),
            format!("{} already exists; confirm to replace it", out.display())
        );
        assert!(
            tauri::async_runtime::block_on(save_problem_report(handle.clone(), out.to_string_lossy().into(), None, Some(true))).is_ok()
        );
        let _ = std::fs::remove_file(out);
        // A mock app has no real window to open; the project it would open is checked below.
        let _ = open_activity_log(handle);
    }

    #[test]
    fn snapshots_saved_through_the_command_carry_the_windows_breadcrumbs() {
        let _g = crate::activity::test_capture();
        log::info!(target: super::super::SOURCE_CONTEXT, SourceContext = super::super::SOURCE_CONTEXT, Operation = "export.csv", Trigger = "menu", Window = "w-snap-crumbs", Title = "Export CSV"; "User export.csv via menu");
        let app = tauri::test::mock_app();
        let png = base64::Engine::encode(
            &base64::engine::general_purpose::STANDARD,
            [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1],
        );
        let ctx = SnapshotContext {
            reason: "manual".into(),
            message: "boom".into(),
            manual: true,
            ..Default::default()
        };
        let saved = crate::snapshots::save_snapshot(app.handle(), "w-snap-crumbs", png, "image/png".into(), ctx).unwrap();
        let store = crate::snapshots::store(app.handle()).unwrap();
        let info = store.list().into_iter().find(|i| i.id == saved.id).unwrap();
        assert_eq!(info.breadcrumbs.last().unwrap().operation, "export.csv");
        assert_eq!(info.steps, vec!["1. Clicked 'Export CSV' from the menu", "2. Error: boom"]);
        store.delete(&saved.id).unwrap();
    }
}
