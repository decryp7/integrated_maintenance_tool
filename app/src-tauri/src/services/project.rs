//! Projects (ADR 0014 §6): a project is the window's store folder, saved as a self-contained `.debriefz`; the UI's
//! state and reports travel in it as `ui.json`. Debrief's own workspace files (`.debriefproj`: the autosaved session
//! with the store folder to reopen, the activity log, unpacked bundles) name files to import instead. The
//! commands (`save_project`, `open_project`, `open_bundle`, `save_session`) check paths and run the jobs; this
//! knows no window or job.

use crate::error::{CmdError, CmdResult};
use crate::profile::ProfileDoc;
use crate::ErrorCode;
use debrief_core::project::{ProjectFile, PROJECT_VERSION};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Most captures a project reopens (the engine keeps at most 32 open).
pub const MAX_PROJECT_CAPTURES: usize = 32;

/// The UI's state and reports in a project folder (exported with it).
pub const UI_FILE: &str = "ui.json";
/// Largest `ui.json` read back.
const MAX_UI_BYTES: u64 = 64 << 20;

/// What `save_project` answers.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SavedProject {
    pub path: String,
    pub name: String,
}

/// What opening a project (or a workspace file, or a bundle) answers: the summary, the profile, the UI's state
/// and reports, and the files that were missing.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct OpenedProject {
    pub path: String,
    pub name: String,
    pub summary: debrief_core::api::WorkspaceSummary,
    pub profile: ProfileDoc,
    pub ui: Value,
    pub reports: Vec<Value>,
    pub missing: Vec<String>,
    /// A bundle: there is no project file to save over.
    pub read_only: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bundle: Option<Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
}

/// The window's session as a workspace file: its store folder (reopened instead of importing again), the opened
/// roots (imported when the store is gone), the profile text and the UI's state.
pub fn session(name: String, profile: String, roots: Vec<String>, store: Option<String>, ui: Value) -> ProjectFile {
    ProjectFile {
        version: PROJECT_VERSION,
        name,
        sources: roots,
        profile,
        ui,
        store,
        ..ProjectFile::default()
    }
}

/// Writes the UI's state and the reports into a project folder (atomically). Blocking.
pub fn write_ui(dir: &Path, ui: &Value, reports: &[debrief_core::report::Report]) -> CmdResult<()> {
    let io = |e: std::io::Error| CmdError::new(ErrorCode::Io, format!("cannot save the project's view: {e}"));
    let json = serde_json::to_vec(&serde_json::json!({ "version": 1, "ui": ui, "reports": reports }))
        .map_err(|e| CmdError::new(ErrorCode::Internal, e.to_string()))?;
    let tmp = dir.join(format!("{UI_FILE}.tmp"));
    std::fs::write(&tmp, json)
        .and_then(|_| std::fs::rename(&tmp, dir.join(UI_FILE)))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io(e)
        })
}

/// The UI's state and the reports of a project folder; nothing when there are none or they cannot be read
/// (the project still opens). Blocking.
pub fn read_ui(dir: &Path) -> (Value, Vec<Value>) {
    let file = dir.join(UI_FILE);
    let parsed = std::fs::metadata(&file)
        .ok()
        .filter(|m| m.len() <= MAX_UI_BYTES)
        .and_then(|_| std::fs::read(&file).ok())
        .and_then(|b| serde_json::from_slice::<Value>(&b).ok());
    match parsed {
        Some(v) => {
            let reports = v.get("reports").and_then(Value::as_array).cloned().unwrap_or_default();
            (v.get("ui").cloned().unwrap_or(Value::Null), reports)
        }
        None => (Value::Null, Vec::new()),
    }
}

/// Reads a project file. Blocking.
pub fn load(file: &Path) -> CmdResult<ProjectFile> {
    ProjectFile::load(file).map_err(|e| {
        let code = if file.exists() { ErrorCode::Invalid } else { ErrorCode::NotFound };
        CmdError::new(code, format!("{e:#}"))
    })
}

/// The profiling captures a project's UI state reopens: `ui.profiling.captures`, the one place the UI state
/// names files (see `profiling-persistence.ts`). Opening the project is the user's choice of them.
pub fn captures(ui: &Value) -> Vec<PathBuf> {
    ui.pointer("/profiling/captures")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(Value::as_str)
        .filter(|p| crate::paths::parse(p).is_ok())
        .take(MAX_PROJECT_CAPTURES)
        .map(PathBuf::from)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[test]
    fn sessions_and_the_view_of_a_project_round_trip() {
        let t = TempDir::new("project-service");
        let file = t.0.join("session.debriefproj");
        let ui = serde_json::json!({ "profiling": { "captures": [t.0.join("a.cpuprofile").to_string_lossy(), "relative", 3] } });
        let s = session(
            "Last session".into(),
            "name = 'x'".into(),
            vec![t.0.join("logs").to_string_lossy().into_owned()],
            Some(t.0.join("store").to_string_lossy().into_owned()),
            ui.clone(),
        );
        s.save(&file).unwrap();
        let back = load(&file).unwrap();
        assert_eq!((back.name.as_str(), back.sources.len()), ("Last session", 1));
        assert!(back.store.is_some());
        assert_eq!(captures(&back.ui), vec![t.0.join("a.cpuprofile")]);
        assert_eq!(load(&t.0.join("missing.debriefproj")).unwrap_err().code, ErrorCode::NotFound);
        std::fs::write(t.0.join("bad.debriefproj"), "{").unwrap();
        assert_eq!(load(&t.0.join("bad.debriefproj")).unwrap_err().code, ErrorCode::Invalid);
        assert!(captures(&Value::Null).is_empty());
        // The view of a project folder.
        assert_eq!(read_ui(&t.0), (Value::Null, vec![]));
        write_ui(&t.0, &ui, &[]).unwrap();
        assert_eq!(read_ui(&t.0).0, ui);
        std::fs::write(t.0.join(UI_FILE), "not json").unwrap();
        assert_eq!(read_ui(&t.0).0, Value::Null, "a damaged view does not stop the project");
        assert!(write_ui(&t.0.join("missing"), &ui, &[]).is_err());
    }
}
