//! The knowledge commands (ADR 0029): list, read, create, import, change, remove and export knowledge files of either
//! scope, and the project record saved with projects and sessions. Thin: arguments are checked here and by the
//! `imt_ai::knowledge` types; files are read and written off the UI thread. Nothing here logs knowledge text.

use super::super::commands::{core, markdown_target};
use super::super::error::{AiCmdError, AiErrorCode, AiResult};
use crate::windows::CurrentWindow;
use imt_ai::knowledge::{KnowledgeChange, KnowledgeInfo, KnowledgeRecord, KnowledgeScope, MAX_FILES, MAX_FILE_BYTES, MAX_NAME_CHARS};
use serde::Serialize;
use serde_json::Value;
use tauri::{AppHandle, Runtime};

type Ai<T> = AiResult<T>;

/// Both scopes' files (without text), what went wrong reading them, and the limits.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeList {
    pub project: Vec<KnowledgeInfo>,
    pub global: Vec<KnowledgeInfo>,
    /// Problems reading the global files (damaged index, unreadable files).
    pub warnings: Vec<String>,
    /// The global files were saved by a newer IMT: they cannot be changed here.
    pub global_read_only: bool,
    pub max_file_bytes: usize,
    pub max_files: usize,
}

/// A knowledge file with its text, for the editor.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeText {
    #[serde(flatten)]
    pub info: KnowledgeInfo,
    pub text: String,
}

/// The project's files after a restore, and why saved ones were not loaded.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRestore {
    pub files: Vec<KnowledgeInfo>,
    pub warning: Option<String>,
}

fn list(win: &CurrentWindow, core: &super::super::AiCore) -> KnowledgeList {
    let g = core.knowledge();
    KnowledgeList {
        project: win.ai.knowledge.snapshot().infos(KnowledgeScope::Project),
        global: g.snapshot().infos(KnowledgeScope::Global),
        warnings: g.warnings(),
        global_read_only: g.read_only(),
        max_file_bytes: MAX_FILE_BYTES,
        max_files: MAX_FILES,
    }
}

#[tauri::command]
pub async fn ai_knowledge_list<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> Ai<KnowledgeList> {
    let core = core(&app)?;
    Ok(list(&win, &core))
}

#[tauri::command]
pub async fn ai_knowledge_get<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, scope: KnowledgeScope, id: String) -> Ai<KnowledgeText> {
    let core = core(&app)?;
    let set = match scope {
        KnowledgeScope::Project => win.ai.knowledge.snapshot(),
        KnowledgeScope::Global => core.knowledge().snapshot(),
    };
    let f = set
        .get(&id)
        .ok_or_else(|| AiCmdError::not_found("this knowledge file no longer exists"))?;
    Ok(KnowledgeText {
        info: f.info(scope),
        text: f.text.clone(),
    })
}

/// Adds a file `name` with `text` (empty when not given) to `scope`.
#[tauri::command]
pub async fn ai_knowledge_create<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    scope: KnowledgeScope,
    name: String,
    text: Option<String>,
) -> Ai<KnowledgeInfo> {
    let core = core(&app)?;
    let w = win.arc().clone();
    blocking(move || {
        let text = text.unwrap_or_default();
        let f = match scope {
            KnowledgeScope::Project => w.ai.knowledge.change(|s| s.add(&name, &text).cloned())?,
            KnowledgeScope::Global => core.knowledge().add(&name, &text)?,
        };
        log::info!(Scope = scope.label(), Bytes = f.text.len(); "Knowledge file added");
        Ok(f.info(scope))
    })
    .await
}

/// Imports a Markdown or text file the user chose (open dialog) as a new knowledge file named after it.
#[tauri::command]
pub async fn ai_knowledge_import<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    scope: KnowledgeScope,
    path: String,
) -> Ai<KnowledgeInfo> {
    let core = core(&app)?;
    let granted = crate::paths::WindowGrants::of(&app, win.label()).read_file(&path, &["md", "markdown", "txt"])?;
    let w = win.arc().clone();
    blocking(move || {
        let path = granted.path().to_path_buf();
        let len = std::fs::metadata(&path)
            .map_err(|e| AiCmdError::new(AiErrorCode::Io, format!("cannot read {}: {e}", path.display())))?
            .len();
        if len > MAX_FILE_BYTES as u64 {
            return Err(AiCmdError::new(
                AiErrorCode::Limit,
                format!(
                    "{} is too large for a knowledge file ({} KiB; at most {} KiB): split it or shorten it",
                    path.display(),
                    len.div_ceil(1024),
                    MAX_FILE_BYTES / 1024
                ),
            ));
        }
        let bytes = std::fs::read(&path).map_err(|e| AiCmdError::new(AiErrorCode::Io, format!("cannot read {}: {e}", path.display())))?;
        let text = String::from_utf8(bytes).map_err(|_| AiCmdError::invalid(format!("{} is not UTF-8 text", path.display())))?;
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
        let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
        let name: String = stem.chars().filter(|c| !c.is_control()).take(MAX_NAME_CHARS).collect();
        let name = if name.trim().is_empty() { "Imported".to_string() } else { name };
        let f = match scope {
            KnowledgeScope::Project => w.ai.knowledge.change(|s| s.add(&name, text).cloned())?,
            KnowledgeScope::Global => core.knowledge().add(&name, text)?,
        };
        log::info!(Scope = scope.label(), Bytes = f.text.len(); "Knowledge file imported");
        Ok(f.info(scope))
    })
    .await
}

/// Renames, enables or disables, or replaces the text of a file (only the fields given).
#[tauri::command]
pub async fn ai_knowledge_update<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    scope: KnowledgeScope,
    id: String,
    change: KnowledgeChange,
) -> Ai<KnowledgeInfo> {
    let core = core(&app)?;
    let w = win.arc().clone();
    blocking(move || {
        let f = match scope {
            KnowledgeScope::Project => w.ai.knowledge.change(|s| s.update(&id, &change).cloned())?,
            KnowledgeScope::Global => core.knowledge().update(&id, &change)?,
        };
        Ok(f.info(scope))
    })
    .await
}

#[tauri::command]
pub async fn ai_knowledge_remove<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, scope: KnowledgeScope, id: String) -> Ai<bool> {
    let core = core(&app)?;
    let w = win.arc().clone();
    blocking(move || {
        match scope {
            KnowledgeScope::Project => w.ai.knowledge.change(|s| s.remove(&id))?,
            KnowledgeScope::Global => core.knowledge().remove(&id)?,
        };
        log::info!(Scope = scope.label(); "Knowledge file removed");
        Ok(true)
    })
    .await
}

/// Writes a file's text to a Markdown file the user chose (save dialog); never replaces one without `overwrite`.
#[tauri::command]
pub async fn ai_knowledge_export<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    scope: KnowledgeScope,
    id: String,
    path: String,
    overwrite: Option<bool>,
) -> Ai<String> {
    let core = core(&app)?;
    let set = match scope {
        KnowledgeScope::Project => win.ai.knowledge.snapshot(),
        KnowledgeScope::Global => core.knowledge().snapshot(),
    };
    let text = set
        .get(&id)
        .map(|f| f.text.clone())
        .ok_or_else(|| AiCmdError::not_found("this knowledge file no longer exists"))?;
    // Only where the user chose to save (the save dialog granted it to this window).
    let granted = crate::paths::WindowGrants::of(&app, win.label()).write(&path, &["md", "markdown"])?;
    let target = markdown_target(&granted.path().to_string_lossy(), overwrite.unwrap_or(false))?;
    blocking(move || {
        crate::config_store::write_atomic(&target, text.as_bytes())
            .map_err(|e| AiCmdError::new(AiErrorCode::Io, format!("cannot write {}: {e}", target.display())))?;
        Ok(target.display().to_string())
    })
    .await
}

/// The window's project knowledge as saved in projects and sessions (`KnowledgeRecord` v1).
#[tauri::command]
pub async fn ai_knowledge_project_record<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> Ai<KnowledgeRecord> {
    core(&app)?;
    Ok(KnowledgeRecord::of(&win.ai.knowledge.snapshot()))
}

/// Replaces the window's project knowledge with a saved record (project or session; none clears it). Damaged or newer
/// records give none and say why; damaged files in a record are skipped.
#[tauri::command]
pub async fn ai_knowledge_project_restore<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    record: Option<Value>,
) -> Ai<KnowledgeRestore> {
    core(&app)?;
    let (set, warning) = KnowledgeRecord::read(record.as_ref());
    let files = set.infos(KnowledgeScope::Project);
    win.ai.knowledge.replace(set);
    Ok(KnowledgeRestore { files, warning })
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Ai<T> + Send + 'static) -> Ai<T> {
    crate::sync::blocking("the knowledge task", f).await
}
