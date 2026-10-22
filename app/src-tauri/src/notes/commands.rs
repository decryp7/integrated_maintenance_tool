//! The notes commands (ADR 0021): list (with which anchors still resolve), add, change and remove notes, the
//! project record saved with projects and sessions, and the images notes mark. Thin: drafts and changes are checked
//! by `imt_notes`; event anchors are filled from the store and image anchors from the images this window opened, so
//! the webview never decides what a note points at. Nothing here logs note text.

use super::images::{self, NoteImage};
use super::{author, now_ms};
use crate::error::{CmdError, CmdResult};
use crate::windows::{CurrentWindow, WindowState};
use imt_core::batch::EventId;
use imt_core::ports::EventsRequest;
use imt_notes::{Anchor, Note, NoteChange, NoteDraft, NoteRecord, MAX_NOTES, MAX_REGIONS, MAX_TAGS, MAX_TEXT_BYTES};
use serde::Serialize;
use serde_json::Value;
use std::collections::HashSet;
use std::sync::Arc;

/// A note in a list, and whether its anchor is still there (the event in the store, the source in the workspace,
/// the image in the project or on disk).
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteItem {
    #[serde(flatten)]
    pub note: Note,
    pub resolved: bool,
}

/// The window's notes and the limits.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotesList {
    pub notes: Vec<NoteItem>,
    pub max_notes: usize,
    pub max_text_bytes: usize,
    pub max_tags: usize,
    pub max_regions: usize,
}

/// How many notes a restore brought back, and why saved ones were not loaded.
#[cfg_attr(test, derive(ts_rs::TS))]
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotesRestore {
    pub count: usize,
    pub warning: Option<String>,
}

/// The paths the window's workspace lists (sources and files it did not recognise).
fn listed_paths(win: &WindowState) -> HashSet<String> {
    let Some(s) = win.engine.opened().and_then(|store| store.summary().ok()) else {
        return HashSet::new();
    };
    s.sources
        .into_iter()
        .map(|x| x.path)
        .chain(s.unrecognized.into_iter().map(|u| u.path))
        .collect()
}

/// Which of `notes` resolve now. Blocking (asks the store).
fn resolve(win: &WindowState, notes: &[Note]) -> Vec<NoteItem> {
    let ids: Vec<EventId> = notes.iter().filter_map(|n| n.anchor.event_id()).map(EventId).collect();
    let store = win.engine.opened();
    let found: HashSet<u64> = match (&store, ids.is_empty()) {
        (Some(s), false) => s
            .events(&EventsRequest { ids, group_by: Vec::new() })
            .map(|evs| evs.into_iter().map(|e| e.id.0).collect())
            .unwrap_or_default(),
        _ => HashSet::new(),
    };
    let needs_paths = notes.iter().any(|n| matches!(n.anchor, Anchor::Source { .. }));
    let paths = if needs_paths { listed_paths(win) } else { HashSet::new() };
    let dir = store.as_ref().map(|s| s.store_dir());
    notes
        .iter()
        .map(|n| {
            let resolved = match &n.anchor {
                Anchor::Event { event_id, .. } => found.contains(event_id),
                Anchor::Source { path } => paths.contains(path),
                Anchor::Image { image, .. } => {
                    win.notes.images.get(&image.key).is_some()
                        || dir.as_deref().is_some_and(|d| images::find_copy(d, &image.key).is_some())
                        || image.path.as_deref().is_some_and(|p| std::path::Path::new(p).is_file())
                }
                _ => true,
            };
            NoteItem { note: n.clone(), resolved }
        })
        .collect()
}

/// The anchor as the engine knows it: an event's time, source and excerpt from the store; an image's name and size
/// from the image this window opened (or the one other notes recorded). Blocking.
fn fill(win: &WindowState, anchor: &Anchor) -> CmdResult<Anchor> {
    Ok(match anchor {
        Anchor::Event { event_id, .. } => {
            let store = win.engine.store()?;
            let mut ev = store
                .events(&EventsRequest {
                    ids: vec![EventId(*event_id)],
                    group_by: Vec::new(),
                })?
                .into_iter()
                .next()
                .ok_or_else(|| CmdError::not_found("the event is no longer in the workspace"))?;
            // The excerpt shows what the window shows.
            if let Some(r) = win.host.redacted_view() {
                r.view_event(&mut ev);
            }
            Anchor::Event {
                event_id: *event_id,
                ts: ev.ts,
                source: ev.source_name,
                excerpt: ev.message.lines().next().unwrap_or_default().to_string(),
            }
        }
        Anchor::Source { path } => {
            if !listed_paths(win).contains(path) {
                return Err(CmdError::invalid("the source is not in the workspace"));
            }
            anchor.clone()
        }
        Anchor::Image { image, regions } => {
            let image = match win.notes.images.get(&image.key) {
                Some(open) => {
                    if let Some(store) = win.engine.opened() {
                        images::keep_copy(&store.store_dir(), &open)?;
                    }
                    open.image_ref()
                }
                None => win
                    .notes
                    .snapshot()
                    .notes()
                    .iter()
                    .find_map(|n| match &n.anchor {
                        Anchor::Image { image: known, .. } if known.key == image.key => Some(known.clone()),
                        _ => None,
                    })
                    .ok_or_else(|| CmdError::not_found("the image is no longer open: open it again"))?,
            };
            Anchor::Image {
                image,
                regions: regions.clone(),
            }
        }
        other => other.clone(),
    })
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    crate::sync::blocking("the notes task", f).await
}

#[tauri::command]
pub async fn notes_list(win: CurrentWindow) -> CmdResult<NotesList> {
    let w = win.arc().clone();
    blocking(move || {
        let set = w.notes.snapshot();
        Ok(NotesList {
            notes: resolve(&w, set.notes()),
            max_notes: MAX_NOTES,
            max_text_bytes: MAX_TEXT_BYTES,
            max_tags: MAX_TAGS,
            max_regions: MAX_REGIONS,
        })
    })
    .await
}

/// Adds a note; its anchor is filled by the engine.
#[tauri::command]
pub async fn notes_add(win: CurrentWindow, draft: NoteDraft) -> CmdResult<Note> {
    let w: Arc<WindowState> = win.arc().clone();
    blocking(move || {
        let anchor = fill(&w, &draft.anchor)?;
        let draft = NoteDraft { anchor, ..draft };
        let note = w.notes.change(|s| s.add(&draft, &author(), now_ms()).cloned())?;
        log::info!(Kind = note.anchor.kind(); "Note added");
        Ok(note)
    })
    .await
}

/// Changes the text, tags, severity or (image notes) regions of a note: only the fields given.
#[tauri::command]
pub async fn notes_update(win: CurrentWindow, id: String, change: NoteChange) -> CmdResult<Note> {
    Ok(win.notes.change(|s| s.update(&id, &change, now_ms()).cloned())?)
}

#[tauri::command]
pub async fn notes_remove(win: CurrentWindow, id: String) -> CmdResult<bool> {
    win.notes.change(|s| s.remove(&id))?;
    log::info!("Note removed");
    Ok(true)
}

/// The window's notes as saved in projects and sessions (`NoteRecord` v1).
#[tauri::command]
pub async fn notes_record(win: CurrentWindow) -> CmdResult<NoteRecord> {
    Ok(NoteRecord::of(&win.notes.snapshot()))
}

/// Replaces the window's notes with a saved record (project or session; none clears them). A damaged or newer record
/// gives none and says why; damaged notes in a record are skipped and counted.
#[tauri::command]
pub async fn notes_restore(win: CurrentWindow, record: Option<Value>) -> CmdResult<NotesRestore> {
    let (set, warning) = NoteRecord::read(record.as_ref());
    let count = set.notes().len();
    win.notes.replace(set);
    Ok(NotesRestore { count, warning })
}

/// Opens an image file of the workspace (the Explorer lists it) or one the user chose (dialog, drag-and-drop) to mark
/// notes on: decoded in Rust into a clean copy.
#[tauri::command]
pub async fn notes_image_open(win: CurrentWindow, grants: crate::paths::WindowGrants, path: String) -> CmdResult<NoteImage> {
    let w = win.arc().clone();
    let granted = grants.read_or_listed(&path, imt_ai::vision::SourceFormat::EXTENSIONS, |raw| {
        listed_paths(&w).contains(raw)
    })?;
    let file = granted.path().to_path_buf();
    blocking(move || {
        let img = images::open_file(&file)?;
        Ok(w.notes.images.insert(img).dto.clone())
    })
    .await
}

/// The image `key` that notes mark: open in this window, else the project's copy.
#[tauri::command]
pub async fn notes_image_get(win: CurrentWindow, key: String) -> CmdResult<NoteImage> {
    if let Some(open) = win.notes.images.get(&key) {
        return Ok(open.dto.clone());
    }
    let known = win
        .notes
        .snapshot()
        .notes()
        .iter()
        .find_map(|n| match &n.anchor {
            Anchor::Image { image, .. } if image.key == key => Some(image.clone()),
            _ => None,
        })
        .ok_or_else(|| CmdError::not_found("no note marks this image"))?;
    let w = win.arc().clone();
    blocking(move || {
        let dir = w.engine.store()?.store_dir();
        let img = match images::open_copy(&dir, &known) {
            Ok(img) => img,
            // No copy (a note made before a store existed): the original, if it is still there and listed.
            Err(e) => match known.path.as_deref().filter(|p| listed_paths(&w).contains(*p)) {
                Some(p) => images::open_file(std::path::Path::new(p))?,
                None => return Err(e),
            },
        };
        Ok(w.notes.images.insert(img).dto.clone())
    })
    .await
}
