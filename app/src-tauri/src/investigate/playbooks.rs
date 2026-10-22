//! Playbook commands: the library (files in the configuration folder), import and export, running a step against the
//! open data, and recording a run. Runs live in the project's UI state; the engine validates what the UI sends back.

use crate::app_dirs::{resolve, AppDir};
use crate::engine::with_store;
use crate::windows::CurrentWindow;
use crate::{CmdError, CmdResult};
use imt_core::playbook::{self, Playbook, PlaybookRecord, PlaybookRun, Step, StepResult};
use serde::Serialize;
use serde_json::Value;
use std::path::PathBuf;
use tauri::{AppHandle, Runtime};

fn dir<R: Runtime>(app: &AppHandle<R>) -> CmdResult<PathBuf> {
    resolve(app, AppDir::Config)
        .map(|d| d.join("playbooks"))
        .map_err(|e| CmdError::failed(format!("the configuration folder is not available: {e}")))
}

fn invalid(e: anyhow::Error) -> CmdError {
    CmdError::invalid(format!("{e:#}"))
}

/// The library and the built-in templates.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybookLibrary {
    pub playbooks: Vec<Playbook>,
    pub templates: Vec<Playbook>,
    /// Files in the folder that could not be read, each with what to fix.
    pub warnings: Vec<String>,
}

#[tauri::command]
pub async fn playbook_library<R: Runtime>(app: AppHandle<R>) -> CmdResult<PlaybookLibrary> {
    let d = dir(&app)?;
    crate::sync::blocking("the playbook task", move || {
        let lib = playbook::list_library(&d);
        Ok::<_, CmdError>(PlaybookLibrary {
            playbooks: lib.playbooks,
            templates: playbook::templates(),
            warnings: lib.warnings,
        })
    })
    .await
}

/// Saves a playbook (new or changed) in the library; the engine validates it.
#[tauri::command]
pub async fn playbook_save<R: Runtime>(app: AppHandle<R>, playbook: Value) -> CmdResult<Playbook> {
    let p = Playbook::parse(playbook).map_err(invalid)?;
    let d = dir(&app)?;
    crate::sync::blocking_anyhow("the playbook task", move || playbook::save_to_library(&d, &p).map(|()| p)).await
}

#[tauri::command]
pub async fn playbook_remove<R: Runtime>(app: AppHandle<R>, id: String) -> CmdResult<bool> {
    let d = dir(&app)?;
    crate::sync::blocking_anyhow("the playbook task", move || playbook::remove_from_library(&d, &id)).await
}

/// Reads a playbook file the user chose (it is not saved until the UI saves it).
#[tauri::command]
pub async fn playbook_import<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, path: String) -> CmdResult<Playbook> {
    let granted = crate::paths::WindowGrants::of(&app, win.label()).read_file(&path, &["json"])?;
    let p = granted.path().to_path_buf();
    crate::sync::blocking_anyhow("the playbook task", move || playbook::import_from(&p)).await
}

/// Writes a playbook to a file the user chose.
#[tauri::command]
pub async fn playbook_export<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, playbook: Value, path: String) -> CmdResult<()> {
    let p = Playbook::parse(playbook).map_err(invalid)?;
    let granted = crate::paths::WindowGrants::of(&app, win.label()).write(&path, &["json"])?;
    let target = granted.path().to_path_buf();
    crate::sync::blocking_anyhow("the playbook task", move || playbook::export_to(&target, &p)).await
}

/// Runs one step of a playbook against the open data. Queries only read.
#[tauri::command]
pub async fn playbook_run_step(win: CurrentWindow, step: Value) -> CmdResult<StepResult> {
    let step: Step = serde_json::from_value(step).map_err(|e| CmdError::invalid(format!("not a valid step: {e}")))?;
    with_store(&win, move |s| playbook::run_step(s, &step)).await
}

/// Starts a run of a playbook (the playbook is copied into it).
#[tauri::command]
pub async fn playbook_start(playbook: Value, run_id: String) -> CmdResult<PlaybookRun> {
    let p = Playbook::parse(playbook).map_err(invalid)?;
    PlaybookRun::start(&run_id, p, super::now_ms()).map_err(invalid)
}

/// Records a step's result and the user's note in a run, and returns the run.
#[tauri::command]
pub async fn playbook_record(run: Value, step_id: String, result: StepResult, note: String) -> CmdResult<PlaybookRun> {
    let mut run: PlaybookRun = serde_json::from_value(run).map_err(|e| CmdError::invalid(format!("not a valid run: {e}")))?;
    run.playbook.check().map_err(invalid)?;
    run.record(&step_id, &result, &note, super::now_ms()).map_err(invalid)?;
    Ok(run)
}

/// What a project's saved runs restore to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PlaybookRestore {
    pub record: PlaybookRecord,
    pub warnings: Vec<String>,
}

/// Checks the runs saved in a project (none clears them); damaged or newer ones give none and say why.
#[tauri::command]
pub async fn playbook_restore(record: Option<Value>) -> CmdResult<PlaybookRestore> {
    let (record, warnings) = match record {
        Some(v) => PlaybookRecord::restore(v),
        None => (PlaybookRecord::new(), Vec::new()),
    };
    Ok(PlaybookRestore { record, warnings })
}
