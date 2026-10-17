//! "Report a problem": one zip the user reviews and saves, to attach to a bug report. Nothing is sent
//! anywhere. It holds the activity log and a bounded window of IMT's own logs, the error snapshots (with
//! their breadcrumbs), the steps to reproduce, system information (no secrets) and the user's
//! description. Also Help ▸ Show activity log, which opens IMT's own logs filtered to the activity entries.
//!
//! Commands: `problem_report_preview`, `save_problem_report`, `open_activity_log`.

use super::breadcrumbs::Breadcrumb;
use super::privacy::is_secret_key;
use crate::snapshots::{tail_of, SnapshotStore};
use crate::{err, CmdResult};
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

/// Everything a report is made from (gathered from the app, or built by tests).
pub struct ReportInputs {
    pub log_dir: Option<PathBuf>,
    pub config_dir: Option<PathBuf>,
    pub snapshots: Option<SnapshotStore>,
    pub plugins: Vec<PluginLine>,
    pub breadcrumbs: Vec<Breadcrumb>,
    pub safe_mode: bool,
}

/// One file of the zip.
struct ReportFile {
    name: String,
    data: Vec<u8>,
    summary: String,
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

/// IMT's own log files, newest first: the active `imt.log`, then rotated ones by modification time.
pub fn log_files(log_dir: &Path) -> Vec<PathBuf> {
    let name = crate::diagnostics::LOG_NAME;
    let active = log_dir.join(format!("{name}.log"));
    let mut rotated: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(log_dir)
        .map(|entries| {
            entries
                .flatten()
                .filter(|e| {
                    let n = e.file_name().to_string_lossy().into_owned();
                    n.starts_with(name) && n.ends_with(".log") && e.path() != active
                })
                .filter_map(|e| Some((e.metadata().ok()?.modified().ok()?, e.path())))
                .collect()
        })
        .unwrap_or_default();
    rotated.sort_by_key(|(t, _)| std::cmp::Reverse(*t));
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

/// The activity entries (`SourceContext = IMT.Activity`) among CLEF lines, oldest first.
fn activity_lines(logs: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut out = Vec::new();
    // Oldest file first, so the result reads in time order.
    for (_, bytes) in logs.iter().rev() {
        for line in String::from_utf8_lossy(bytes).lines() {
            let is_activity = serde_json::from_str::<Value>(line)
                .ok()
                .and_then(|v| v.get("SourceContext").and_then(|s| s.as_str().map(|s| s == super::SOURCE_CONTEXT)))
                .unwrap_or(false);
            if is_activity {
                out.extend_from_slice(line.as_bytes());
                out.push(b'\n');
            }
        }
    }
    out
}

fn read_json(path: &Path) -> Option<Value> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > 1 << 20 {
        return None;
    }
    serde_json::from_str(&std::fs::read_to_string(path).ok()?).ok()
}

/// OS, build, memory, plugins and settings (secrets removed).
pub fn system_info(inputs: &ReportInputs) -> Value {
    let mut sys = sysinfo::System::new();
    sys.refresh_memory();
    let settings = inputs.config_dir.as_deref().map(|d| {
        json!({
            "settings": read_json(&d.join("settings.json")).map(|v| without_secrets(&v)),
            "diagnostics": read_json(&d.join("diagnostics.json")).map(|v| without_secrets(&v)),
            "plugins": read_json(&d.join("plugins.json")).map(|v| without_secrets(&v)),
        })
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
    })
}

/// The zip's files, in order.
fn report_files(inputs: &ReportInputs, description: &str) -> Vec<ReportFile> {
    let mut files = Vec::new();
    let description = cap_description(description.trim());
    files.push(ReportFile {
        name: "description.txt".into(),
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
        summary: format!("{} steps from your recent activity", super::steps(&inputs.breadcrumbs, None).len()),
        data: steps.into_bytes(),
    });
    files.push(ReportFile {
        name: "breadcrumbs.json".into(),
        summary: format!("{} recent operations (no file contents)", inputs.breadcrumbs.len()),
        data: serde_json::to_vec_pretty(&inputs.breadcrumbs).unwrap_or_default(),
    });
    let info = system_info(inputs);
    files.push(ReportFile {
        name: "system-info.json".into(),
        summary: "OS, IMT version and build, memory, plugins, settings without secrets".into(),
        data: serde_json::to_vec_pretty(&info).unwrap_or_default(),
    });
    let logs = inputs.log_dir.as_deref().map(log_window).unwrap_or_default();
    let activity = activity_lines(&logs);
    files.push(ReportFile {
        name: "logs/activity.log".into(),
        summary: format!("{} activity entries", activity.iter().filter(|b| **b == b'\n').count()),
        data: activity,
    });
    for (name, bytes) in logs {
        files.push(ReportFile {
            name: format!("logs/{name}"),
            summary: "IMT's own log (recent part)".into(),
            data: bytes,
        });
    }
    if let Some(store) = &inputs.snapshots {
        let all = store.list();
        let newest = &all[all.len().saturating_sub(MAX_SNAPSHOTS)..];
        for info in newest {
            if let Ok((format, image)) = store.read_image(&info.id) {
                files.push(ReportFile {
                    name: format!("snapshots/{}.{}", info.id, format.extension()),
                    summary: format!("Screenshot: {}", info.reason),
                    data: image,
                });
            }
            let mut sidecar = serde_json::to_vec_pretty(info).unwrap_or_default();
            sidecar.push(b'\n');
            files.push(ReportFile {
                name: format!("snapshots/{}.json", info.id),
                summary: format!("Error details and {} breadcrumbs", info.breadcrumbs.len()),
                data: sidecar,
            });
        }
    }
    files
}

/// The preview the user reviews before saving.
pub fn preview(inputs: &ReportInputs, description: &str) -> ProblemReportPreview {
    ProblemReportPreview {
        entries: report_files(inputs, description)
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

/// Writes the report to `dest` (a `.zip` in an existing folder). Returns the number of files written.
/// A failed write leaves no partial file.
pub fn write(inputs: &ReportInputs, description: &str, dest: &Path) -> anyhow::Result<usize> {
    let dest = crate::snapshots::checked_destination(dest).map_err(|e| anyhow::anyhow!("{e}"))?;
    let result = (|| -> anyhow::Result<usize> {
        let files = report_files(inputs, description);
        let mut zip = zip::ZipWriter::new(std::fs::File::create(&dest)?);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
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
    ReportInputs {
        log_dir: app.path().app_log_dir().ok(),
        config_dir: crate::snapshots::config_dir(app),
        snapshots: crate::snapshots::store(app).ok(),
        plugins,
        breadcrumbs: super::breadcrumbs(None, REPORT_BREADCRUMBS),
        safe_mode: app.try_state::<crate::lifecycle::Lifecycle>().is_some_and(|l| l.safe_mode),
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
    let log_dir = app.path().app_log_dir().map_err(err)?;
    let data_dir = app.path().app_data_dir().map_err(err)?;
    let project = write_activity_project(&data_dir, &log_dir).map_err(err)?;
    crate::window_manager::open_new(&app, vec![project.to_string_lossy().into_owned()]).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::snapshots::{Environment, ImageFormat, Limits, SnapshotContext};
    use std::collections::BTreeMap;

    fn temp(tag: &str) -> PathBuf {
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

    fn inputs_in(d: &Path) -> ReportInputs {
        let logs = d.join("logs");
        let config = d.join("config");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        std::fs::write(
            logs.join("imt.log"),
            "{\"@m\":\"started\",\"SourceContext\":\"imt_app\"}\n{\"@m\":\"User export.csv via menu\",\"SourceContext\":\"IMT.Activity\"}\n",
        )
        .unwrap();
        std::fs::write(
            config.join("settings.json"),
            r#"{"version":1,"sections":{"ai":{"braveApiKey":"sk-secret","model":"small"},"web":{"token":"t0k"}}}"#,
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
            safe_mode: false,
        }
    }

    fn zip_text(zip: &mut zip::ZipArchive<std::fs::File>, name: &str) -> String {
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
            "description.txt",
            "steps-to-reproduce.txt",
            "breadcrumbs.json",
            "system-info.json",
            "logs/activity.log",
            "logs/imt.log",
        ] {
            assert!(names.contains(&want), "{want} missing from {names:?}");
        }
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
            !info.contains("sk-secret") && !info.contains("t0k") && !info.contains("braveApiKey"),
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
        // Not a zip, or a missing folder: refused and nothing written.
        assert!(write(&inputs, "", &d.join("report.txt")).is_err());
        assert!(write(&inputs, "", &d.join("missing/r.zip")).is_err());
        std::fs::remove_dir_all(d).unwrap();
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
        assert_eq!(p.entries[0].summary, "No description");
        assert_eq!(p.entries[0].bytes, 0);
        assert!(p.system_info["configuration"].is_null());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn the_log_window_is_bounded_to_the_newest_two_files() {
        let d = temp("window");
        let line = format!("{{\"@m\":\"{}\"}}\n", "x".repeat(1000));
        let big = line.repeat((LOG_WINDOW as usize / line.len()) + 100);
        std::fs::write(d.join("imt_old.log"), "old\n").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
        std::fs::write(d.join("imt_newer.log"), "newer\n").unwrap();
        std::fs::write(d.join("imt.log"), &big).unwrap();
        std::fs::write(d.join("other.log"), "x").unwrap();
        let files = log_files(&d);
        assert_eq!(files.len(), 3);
        assert!(files[0].ends_with("imt.log") && files[1].ends_with("imt_newer.log"));
        let w = log_window(&d);
        let total: usize = w.iter().map(|(_, b)| b.len()).sum();
        assert!(total as u64 <= LOG_WINDOW, "{total}");
        assert_eq!(w[0].0, "imt.log");
        // A small current file leaves room for the newest rotated one, never the older one.
        std::fs::write(d.join("imt.log"), "now\n").unwrap();
        let names: Vec<String> = log_window(&d).into_iter().map(|(n, _)| n).collect();
        assert_eq!(names, ["imt.log", "imt_newer.log"]);
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
