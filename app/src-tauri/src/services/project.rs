//! Projects (`.imtproj`) as files: what a window's state saves, writing and reading them, and what an
//! opened project lets the window reopen. The commands (`save_project`, `open_project`, `open_bundle`,
//! `save_session`) check paths and run the import; this knows no window or job.

use crate::error::{CmdError, CmdResult};
use crate::ErrorCode;
use imt_core::project::{ProjectFile, PROJECT_VERSION};
use imt_core::workspace::WorkspaceState;
use serde_json::Value;
use std::path::{Path, PathBuf};

/// Most captures a project reopens (the engine keeps at most 32 open).
pub const MAX_PROJECT_CAPTURES: usize = 32;

/// A project of the open files (`state`), the profile text and the UI's state.
pub fn snapshot(name: String, profile: String, state: WorkspaceState, ui: Value, reports: Vec<imt_core::report::Report>) -> ProjectFile {
    ProjectFile {
        version: PROJECT_VERSION,
        name,
        sources: state.roots,
        removed: state.removed,
        profile,
        ui,
        clock: state.offsets,
        reports,
    }
}

/// Writes `project` to `file`; an existing file only with `overwrite` (else `fileExists`). Blocking.
pub fn save(file: &Path, project: &ProjectFile, overwrite: bool) -> CmdResult<()> {
    imt_core::overwrite::check_overwrite(file, overwrite)?;
    project
        .save(file)
        .map_err(|e| CmdError::new(ErrorCode::Io, format!("cannot save the project: {e:#}")))
}

/// Reads a project file. Blocking.
pub fn load(file: &Path) -> CmdResult<ProjectFile> {
    ProjectFile::load(file).map_err(|e| {
        let code = if file.exists() { ErrorCode::Invalid } else { ErrorCode::NotFound };
        CmdError::new(code, format!("{e:#}"))
    })
}

/// What reopening `project` restores in the workspace.
pub fn workspace_state(project: &ProjectFile) -> WorkspaceState {
    WorkspaceState {
        roots: project.sources.clone(),
        removed: project.removed.clone(),
        offsets: project.clock.clone(),
    }
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
    fn projects_round_trip_and_refuse_overwrites() {
        let t = TempDir::new("project-service");
        let file = t.0.join("p.imtproj");
        let state = WorkspaceState {
            roots: vec![t.0.join("logs").to_string_lossy().into_owned()],
            ..Default::default()
        };
        let ui = serde_json::json!({ "profiling": { "captures": [t.0.join("a.cpuprofile").to_string_lossy(), "relative", 3] } });
        let p = snapshot("Incident".into(), "name = 'x'".into(), state, ui, Vec::new());
        save(&file, &p, false).unwrap();
        assert_eq!(save(&file, &p, false).unwrap_err().code, ErrorCode::FileExists);
        save(&file, &p, true).unwrap();
        let back = load(&file).unwrap();
        assert_eq!((back.name.as_str(), back.sources.len()), ("Incident", 1));
        assert_eq!(workspace_state(&back).roots, back.sources);
        assert_eq!(captures(&back.ui), vec![t.0.join("a.cpuprofile")]);
        assert_eq!(load(&t.0.join("missing.imtproj")).unwrap_err().code, ErrorCode::NotFound);
        std::fs::write(t.0.join("bad.imtproj"), "{").unwrap();
        assert_eq!(load(&t.0.join("bad.imtproj")).unwrap_err().code, ErrorCode::Invalid);
        assert!(captures(&Value::Null).is_empty());
    }
}
