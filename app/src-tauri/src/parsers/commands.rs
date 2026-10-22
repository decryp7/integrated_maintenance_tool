//! The Settings ▸ Parsers commands (ADR 0026): list, edit, validate, import, export, enable and remove parsers
//! of the app-wide library; every change is handed to every window's store, which says which of its sources
//! were read with a parser that changed (re-read with `parsers_reread`). "Parse with…" (`parser_assign`) and
//! the re-read are import jobs of the calling window.

use super::dto::*;
use crate::activity::{domain_event, privacy, OpId};
use crate::jobs::{JobKind, JobSpec};
use crate::paths::WindowGrants;
use crate::windows::CurrentWindow;
use crate::{AppState, CmdError, CmdResult, ErrorCode, State};
use imt_core::batch::SourceId;
use imt_core::parser::library::{Library, LibraryError};
use imt_core::parser::{Parser, MAX_PARSER_BYTES};
use imt_core::ports::ParserUse;
use std::collections::HashMap;
use std::path::Path;
use tauri::{AppHandle, Manager, Runtime};

impl From<LibraryError> for CmdError {
    fn from(e: LibraryError) -> CmdError {
        match e {
            LibraryError::Invalid(p) => CmdError::invalid(p.message.clone())
                .line(p.line)
                .detail(p.column.map(|c| format!("column {c}"))),
            LibraryError::Conflict(_) => CmdError::new(ErrorCode::FileExists, e.to_string()),
            LibraryError::NotFound(_) => CmdError::not_found(e.to_string()),
            LibraryError::BuiltIn(_) => CmdError::invalid(e.to_string()),
            LibraryError::Io(m) => CmdError::new(ErrorCode::Io, m),
        }
    }
}

/// The library as the list shows it, with the sources of the window each parser read.
pub(crate) fn library_dto(lib: &Library, uses: &[ParserUse], names: &HashMap<u32, String>, stale: usize) -> ParserLibraryDto {
    let parsers = lib
        .parsers()
        .iter()
        .map(|p| {
            let reference = p.parser.reference();
            let mut sources: Vec<ParserSourceDto> = uses
                .iter()
                .filter(|u| u.parser.id == reference.id)
                .flat_map(|u| u.sources.iter())
                .map(|s| ParserSourceDto {
                    id: s.0,
                    name: names.get(&s.0).cloned().unwrap_or_else(|| format!("source {}", s.0)),
                })
                .collect();
            sources.sort_by(|a, b| a.name.cmp(&b.name));
            ParserEntryDto {
                id: reference.id,
                name: p.parser.name.clone(),
                version: reference.version,
                description: p.parser.description.clone(),
                category: p.parser.category.clone(),
                kind: p.parser.format.kind.clone(),
                priority: p.parser.matching.priority,
                origin: p.origin.into(),
                enabled: p.enabled,
                hash: reference.hash,
                sources,
            }
        })
        .collect();
    ParserLibraryDto {
        parsers,
        broken: lib
            .broken()
            .iter()
            .map(|b| BrokenParserDto {
                file: b.file.clone(),
                error: (&b.error).into(),
            })
            .collect(),
        stale,
    }
}

/// The list for the calling window: the store's parser uses and source names when it has a store.
async fn list(app_state: &AppState, state: &CurrentWindow) -> CmdResult<ParserLibraryDto> {
    let (uses, names) = match state.engine.opened() {
        Some(store) => {
            crate::sync::blocking("the parsers of the sources", move || {
                let uses = store.parsers().map_err(CmdError::from)?;
                let names = store
                    .summary()
                    .map_err(CmdError::from)?
                    .sources
                    .into_iter()
                    .map(|s| (s.id, s.name))
                    .collect();
                Ok::<_, CmdError>((uses, names))
            })
            .await?
        }
        None => (Vec::new(), HashMap::new()),
    };
    let stale = state.engine.stale_parsers();
    Ok(app_state.parsers.with(|lib| library_dto(lib, &uses, &names, stale)))
}

/// Hands the library to every window's store (each says which of its sources need reading again).
fn apply_everywhere<R: Runtime>(app: &AppHandle<R>, app_state: &AppState) {
    let setup = app_state.parsers.setup();
    if let Some(windows) = app.try_state::<crate::windows::Windows>() {
        for w in windows.all() {
            w.engine.set_parsers(setup.clone());
        }
    }
}

/// The library and the sources of the window each parser read.
#[tauri::command]
pub(crate) async fn parsers_list(app_state: State<'_, AppState>, state: CurrentWindow) -> CmdResult<ParserLibraryDto> {
    list(&app_state, &state).await
}

/// One parser's text (as stored: comments kept).
#[tauri::command]
pub(crate) async fn parser_get(app_state: State<'_, AppState>, id: String) -> CmdResult<ParserDocDto> {
    app_state.parsers.with(|lib| {
        lib.get(&id)
            .map(|p| ParserDocDto {
                id: id.clone(),
                origin: p.origin.into(),
                toml: p.toml.clone(),
            })
            .ok_or_else(|| CmdError::not_found(format!("no parser \"{id}\" in the library")))
    })
}

/// Checks a parser text (syntax, schema, limits, regexes) with the line and column of the first error.
#[tauri::command]
pub(crate) async fn parser_validate(toml: String) -> CmdResult<ParserValidationDto> {
    crate::sync::blocking("checking the parser", move || Ok(validate(&toml))).await
}

pub(crate) fn validate(toml: &str) -> ParserValidationDto {
    match Parser::from_toml(toml) {
        Ok(p) => ParserValidationDto {
            ok: true,
            error: None,
            id: Some(p.id.to_string()),
            name: Some(p.name.clone()),
            version: Some(p.version.to_string()),
        },
        Err(e) => ParserValidationDto {
            ok: false,
            error: Some((&e).into()),
            id: None,
            name: None,
            version: None,
        },
    }
}

/// Saves a user parser (a built-in's id makes an override); `previous_id` is the parser it was edited from.
/// An existing user parser with the same id is replaced only when it is `previous_id` or with `overwrite`.
#[tauri::command]
pub(crate) async fn parser_save<R: Runtime>(
    app: AppHandle<R>,
    app_state: State<'_, AppState>,
    state: CurrentWindow,
    toml: String,
    previous_id: Option<String>,
    overwrite: Option<bool>,
) -> CmdResult<ParserLibraryDto> {
    let id = app_state
        .parsers
        .with(|lib| lib.save(&toml, previous_id.as_deref(), overwrite.unwrap_or(false)))?;
    domain_event("parser.saved", None, &[("Parser", &id)]);
    apply_everywhere(&app, &app_state);
    list(&app_state, &state).await
}

/// Saves a copy of a parser under a new id; returns it.
#[tauri::command]
pub(crate) async fn parser_duplicate<R: Runtime>(app: AppHandle<R>, app_state: State<'_, AppState>, id: String) -> CmdResult<ParserDocDto> {
    let doc = app_state.parsers.with(|lib| -> CmdResult<ParserDocDto> {
        let new_id = lib.duplicate(&id)?;
        let p = lib.get(&new_id).ok_or_else(|| CmdError::not_found(new_id.clone()))?;
        Ok(ParserDocDto {
            id: new_id.clone(),
            origin: p.origin.into(),
            toml: p.toml.clone(),
        })
    })?;
    apply_everywhere(&app, &app_state);
    Ok(doc)
}

/// Removes a user parser or an override (the built-in comes back).
#[tauri::command]
pub(crate) async fn parser_remove<R: Runtime>(
    app: AppHandle<R>,
    app_state: State<'_, AppState>,
    state: CurrentWindow,
    id: String,
) -> CmdResult<ParserLibraryDto> {
    app_state.parsers.with(|lib| lib.remove(&id))?;
    domain_event("parser.removed", None, &[("Parser", &id)]);
    apply_everywhere(&app, &app_state);
    list(&app_state, &state).await
}

/// Enables or disables a parser for automatic matching ("Parse with…" may still choose it).
#[tauri::command]
pub(crate) async fn parser_set_enabled<R: Runtime>(
    app: AppHandle<R>,
    app_state: State<'_, AppState>,
    state: CurrentWindow,
    id: String,
    enabled: bool,
) -> CmdResult<ParserLibraryDto> {
    app_state.parsers.with(|lib| lib.set_enabled(&id, enabled))?;
    apply_everywhere(&app, &app_state);
    list(&app_state, &state).await
}

/// Reads a parser file the user chose and says what importing it would do (nothing is saved: the UI saves
/// the returned text with `parser_save`, `overwrite` after the user confirmed an update).
#[tauri::command]
pub(crate) async fn parser_import_check(
    app_state: State<'_, AppState>,
    grants: WindowGrants,
    path: String,
) -> CmdResult<ParserImportCheckDto> {
    let file = grants.read_file(&path, &["toml"])?;
    let len = std::fs::metadata(file.path())
        .map_err(|e| CmdError::new(ErrorCode::Io, e.to_string()))?
        .len();
    if len > MAX_PARSER_BYTES as u64 {
        return Err(CmdError::new(
            ErrorCode::Limit,
            format!(
                "the file is {} KiB; a parser file has at most {} KiB",
                len / 1024,
                MAX_PARSER_BYTES / 1024
            ),
        ));
    }
    let toml = std::fs::read_to_string(file.path()).map_err(|e| CmdError::new(ErrorCode::Io, e.to_string()))?;
    let check = app_state.parsers.with(|lib| lib.check_import(&toml)).map_err(|e| {
        CmdError::invalid(e.message.clone())
            .line(e.line)
            .detail(e.column.map(|c| format!("column {c}")))
    })?;
    Ok(ParserImportCheckDto {
        id: check.parser.id.to_string(),
        name: check.parser.name.clone(),
        version: check.parser.version.to_string(),
        relation: check.relation.into(),
        existing: check.existing.map(|e| ParserExistingDto {
            origin: e.origin.into(),
            version: e.parser.version.to_string(),
            toml: e.toml,
        }),
        toml,
    })
}

/// Writes a parser file (as stored, self-contained) to a path the user chose; an existing file only with
/// `overwrite`.
#[tauri::command]
pub(crate) async fn parser_export(
    app_state: State<'_, AppState>,
    grants: WindowGrants,
    op: OpId,
    id: String,
    path: String,
    overwrite: Option<bool>,
) -> CmdResult<()> {
    let toml = app_state
        .parsers
        .with(|lib| lib.get(&id).map(|p| p.toml.clone()))
        .ok_or_else(|| CmdError::not_found(format!("no parser \"{id}\" in the library")))?;
    let target = grants.write(&path, &["toml"])?;
    imt_core::overwrite::check_overwrite(Path::new(&path), overwrite.unwrap_or(false))?;
    std::fs::write(target.path(), &toml).map_err(|e| CmdError::new(ErrorCode::Io, e.to_string()))?;
    let (safe, bytes) = (privacy::safe_path(Path::new(&path)), toml.len().to_string());
    domain_event(
        "export.written",
        op.as_deref(),
        &[("Export", "parser"), ("Path", &safe), ("Bytes", &bytes)],
    );
    Ok(())
}

/// Composes a parser for the format wizard (not saved): a new one from a regex or an output template, or a
/// copy of the library's parser `base` with a timestamp format, regex or template added first.
#[tauri::command]
pub(crate) async fn parser_draft(app_state: State<'_, AppState>, request: ParserDraftRequest) -> CmdResult<ParserDocDto> {
    let base = match request.base.as_deref() {
        Some(id) => Some(
            app_state
                .parsers
                .with(|lib| lib.get(id).map(|p| (p.parser.clone(), p.origin)))
                .ok_or_else(|| CmdError::not_found(format!("no parser \"{id}\" in the library")))?,
        ),
        None => None,
    };
    let req: imt_core::parser::draft::DraftRequest = request.into();
    let toml = imt_core::parser::draft::compose(&req, base.as_ref().map(|(p, _)| p))
        .map_err(|e| CmdError::invalid(e.message.clone()).line(e.line))?;
    let parser = Parser::from_toml(&toml).map_err(|e| CmdError::invalid(e.message))?;
    // Saving a changed built-in makes an override.
    let origin = match base.map(|(_, o)| o) {
        Some(imt_core::parser::library::Origin::Builtin) => ParserOrigin::Override,
        Some(o) => o.into(),
        None => ParserOrigin::User,
    };
    Ok(ParserDocDto {
        id: parser.id.to_string(),
        origin,
        toml,
    })
}

/// Reads again the window's sources whose parser changed and retries the files no parser read: an import job.
#[tauri::command]
pub(crate) async fn parsers_reread<R: Runtime>(app: AppHandle<R>, state: CurrentWindow) -> CmdResult<u64> {
    let spec = JobSpec::new(JobKind::Import, "Read the files again with the changed parsers");
    let win = state.arc().clone();
    crate::engine::import::start(&app, &state, spec, Box::new(|store| store.reread_parsers()), move |_| {
        win.engine.clear_stale_parsers();
        Ok(())
    })
}

/// "Parse with…": reads one source with the library's parser `parser_id` (`None`: the automatic choice).
#[tauri::command]
pub(crate) async fn parser_assign<R: Runtime>(
    app: AppHandle<R>,
    app_state: State<'_, AppState>,
    state: CurrentWindow,
    source_id: u32,
    parser_id: Option<String>,
) -> CmdResult<u64> {
    if let Some(id) = parser_id.as_deref() {
        if app_state.parsers.with(|lib| lib.get(id).is_none()) {
            return Err(CmdError::not_found(format!("no parser \"{id}\" in the library")));
        }
    }
    let title = match parser_id.as_deref() {
        Some(id) => format!("Read the file with the parser {id}"),
        None => "Read the file with the automatic parser".to_string(),
    };
    let spec = JobSpec::new(JobKind::Import, title);
    crate::engine::import::start(
        &app,
        &state,
        spec,
        Box::new(move |store| store.assign_parser(SourceId(source_id), parser_id.as_deref())),
        |_| Ok(()),
    )
}
