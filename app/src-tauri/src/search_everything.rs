//! Search everything (`search_everything`): the palette's text looked up in the window's events, sources, fields,
//! marker kinds, notes and knowledge files by `debrief_core::search_all`, within its time budget.
//!
//! Thin: this module hands the engine the window's store and what the app holds outside it (notes, project and
//! global knowledge; never anything from the webview) and keeps one search per window: a newer search of the
//! window cancels the older one at its next step (it answers `null`), so typing never queues up searches.

use crate::error::{CmdError, ErrorCode};
use crate::windows::{CurrentWindow, WindowState};
use crate::CmdResult;
use debrief_core::batch::EventId;
use debrief_core::find::{self, FindValuesRequest, ValueMatch};
use debrief_core::search_all::{self, Control, Corpus, DocScope, SearchAllError, SearchAllReply, SearchAllRequest, SearchDoc};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Runtime};

/// Characters of a note's first line used as its title.
const NOTE_TITLE_CHARS: usize = 80;

/// The newest search of each window.
static NEWEST: Mutex<Option<HashMap<String, u64>>> = Mutex::new(None);
static NEXT: AtomicU64 = AtomicU64::new(1);

fn with_newest<T>(f: impl FnOnce(&mut HashMap<String, u64>) -> T) -> T {
    let mut newest = NEWEST.lock().unwrap_or_else(|p| p.into_inner());
    f(newest.get_or_insert_with(HashMap::new))
}

/// Starts a search of window `label`: it replaces the window's previous one.
fn begin(label: &str) -> u64 {
    let id = NEXT.fetch_add(1, Ordering::Relaxed);
    with_newest(|m| m.insert(label.to_string(), id));
    id
}

/// Whether search `id` is still the window's newest.
fn is_newest(label: &str, id: u64) -> bool {
    with_newest(|m| m.get(label) == Some(&id))
}

/// Forgets the window's search when it is still `id` (a newer one stays).
fn end(label: &str, id: u64) {
    with_newest(|m| {
        if m.get(label) == Some(&id) {
            m.remove(label);
        }
    });
}

/// A window is gone: forgets its search (a search still running for it stops at its next step).
pub fn forget_window<R: Runtime>(_: &AppHandle<R>, label: &str) {
    with_newest(|m| m.remove(label));
}

/// A note's title: its first line, shortened.
fn note_title(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default();
    match line.char_indices().nth(NOTE_TITLE_CHARS) {
        Some((at, _)) => format!("{}…", &line[..at]),
        None => line.to_string(),
    }
}

/// The window's notes and knowledge files (global ones only when the AI assistant is part of this build).
fn corpus<R: Runtime>(app: &AppHandle<R>, win: &WindowState) -> Corpus {
    let notes = win
        .notes
        .snapshot()
        .notes()
        .iter()
        .map(|n| SearchDoc {
            id: n.id.clone(),
            scope: DocScope::Project,
            title: note_title(&n.text),
            event: n.anchor.event_id().map(EventId),
            text: n.text.clone(),
        })
        .collect();
    let file = |scope: DocScope| {
        move |f: &debrief_ai::knowledge::KnowledgeFile| SearchDoc {
            id: f.id.clone(),
            scope,
            title: f.name.clone(),
            event: None,
            text: f.text.clone(),
        }
    };
    let mut knowledge: Vec<SearchDoc> = win.ai.knowledge.snapshot().files().iter().map(file(DocScope::Project)).collect();
    if let Ok(core) = crate::ai::commands::core(app) {
        knowledge.extend(core.knowledge().snapshot().files().iter().map(file(DocScope::Global)));
    }
    Corpus { notes, knowledge }
}

/// Searches the window's everything for `request`; `null` when the text has no words or a newer search of the
/// window replaced this one.
#[tauri::command]
pub async fn search_everything<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    request: SearchAllRequest,
) -> CmdResult<Option<SearchAllReply>> {
    let label = state.label().to_string();
    let id = begin(&label);
    let corpus = corpus(&app, &state);
    let store = state.engine.store()?;
    let out = {
        let label = label.clone();
        crate::sync::blocking("search everything", move || {
            let replaced = move || !is_newest(&label, id);
            match search_all::run(store.as_ref(), &corpus, &request, &Control::new(search_all::BUDGET, &replaced)) {
                Ok(reply) => Ok(reply),
                Err(SearchAllError::Cancelled) => Ok(None),
                Err(e @ SearchAllError::Invalid(_)) => Err(CmdError::new(ErrorCode::Invalid, e.to_string())),
            }
        })
        .await
    };
    end(&label, id);
    out
}

/// The values the logs hold that the search box's words refer to (`CompanyA.Graphic.Builder.exe` for "graphic
/// builder"), best first, for a "Did you mean" line. A query: nothing changes.
#[tauri::command]
pub async fn find_values<R: Runtime>(_app: AppHandle<R>, state: CurrentWindow, request: FindValuesRequest) -> CmdResult<Vec<ValueMatch>> {
    if request.text.chars().count() > search_all::MAX_TEXT_CHARS {
        return Err(CmdError::new(
            ErrorCode::Invalid,
            format!("search for at most {} characters", search_all::MAX_TEXT_CHARS),
        ));
    }
    let store = state.engine.store()?;
    crate::sync::blocking("find values", move || {
        find::resolve_terms(store.as_ref(), &request.text, request.limit)
            .map_err(|e| CmdError::new(ErrorCode::Internal, format!("the values cannot be looked up: {e}")))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, import, main_window, samples, test_app};
    use debrief_core::search_all::KindOutcome;

    fn search(app: &tauri::App<tauri::test::MockRuntime>, text: &str) -> CmdResult<Option<SearchAllReply>> {
        block(search_everything(
            app.handle().clone(),
            main_window(app),
            SearchAllRequest {
                text: text.into(),
                limit: 0,
            },
        ))
    }

    #[test]
    fn searches_the_window_and_its_notes() {
        let app = test_app(None);
        // Nothing opened: nothing found, not an error.
        let empty = search(&app, "timeout").unwrap().expect("words");
        assert_eq!(empty.events.total, 0);
        assert_eq!(empty.events.outcome, KindOutcome::Complete);
        assert_eq!(search(&app, "  ").unwrap(), None, "no words: nothing to search");

        import(&main_window(&app), &[samples().join("serilog-clef")]);
        let win = main_window(&app);
        let draft: debrief_notes::NoteDraft = serde_json::from_value(serde_json::json!({
            "anchor": { "kind": "project" },
            "text": "Gateway timeouts\nThey start after the deploy."
        }))
        .unwrap();
        win.notes.change(|s| s.add(&draft, "tester", 1).map(|_| ())).unwrap();

        let found = search(&app, "timeout").unwrap().expect("words");
        assert!(found.events.total > 0, "the samples log timeouts");
        assert!(found.events.items.iter().all(|e| e.snippet.iter().any(|p| p.mark)));
        assert_eq!(found.notes.total, 1);
        assert_eq!(found.notes.items[0].title, "Gateway timeouts");
        assert_eq!(found.notes.items[0].event, None);

        let too_long = search(&app, &"x".repeat(search_all::MAX_TEXT_CHARS + 1)).unwrap_err();
        assert_eq!(too_long.code, ErrorCode::Invalid);
        assert!(with_newest(|m| m.get("main").is_none()), "a finished search is forgotten");
    }

    #[test]
    fn find_values_answers_with_the_values_the_words_refer_to() {
        let app = test_app(None);
        let ask = |text: &str| {
            block(find_values(
                app.handle().clone(),
                main_window(&app),
                FindValuesRequest {
                    text: text.into(),
                    limit: 0,
                },
            ))
        };
        assert!(ask("timeout").unwrap().is_empty(), "nothing opened: no values, not an error");
        import(&main_window(&app), &[samples().join("serilog-clef")]);
        assert!(ask("--").unwrap().is_empty(), "no words: no values");
        let too_long = ask(&"x".repeat(search_all::MAX_TEXT_CHARS + 1)).unwrap_err();
        assert_eq!(too_long.code, ErrorCode::Invalid);
    }

    #[test]
    fn a_newer_search_of_the_window_replaces_the_older_one() {
        let first = begin("w-search");
        assert!(is_newest("w-search", first));
        let second = begin("w-search");
        assert!(!is_newest("w-search", first), "the older search stops at its next step");
        assert!(is_newest("w-search", second));
        // Another window's searches are its own.
        let other = begin("w-other");
        assert!(is_newest("w-search", second) && is_newest("w-other", other));
        end("w-search", first);
        assert!(is_newest("w-search", second), "the older search ending leaves the newer one");
        end("w-search", second);
        end("w-other", other);
        assert!(!is_newest("w-search", second));
    }

    #[test]
    fn a_closed_window_forgets_its_search() {
        let app = test_app(None);
        let running = begin("w-closing");
        let other = begin("w-staying");
        forget_window(app.handle(), "w-closing");
        assert!(
            !is_newest("w-closing", running),
            "the running search of a closed window is cancelled"
        );
        assert!(with_newest(|m| !m.contains_key("w-closing")), "and its entry is gone");
        assert!(is_newest("w-staying", other), "other windows keep theirs");
        // Closing through the window teardown reaches it too (the scope is registered with the app).
        crate::window_scoped::destroyed(app.handle(), "w-staying");
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while is_newest("w-staying", other) && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(!is_newest("w-staying", other));
    }

    #[test]
    fn notes_are_titled_by_their_first_line() {
        assert_eq!(note_title("\n  First line  \nmore"), "First line");
        assert_eq!(note_title(""), "");
        let long = "é".repeat(NOTE_TITLE_CHARS + 5);
        assert_eq!(note_title(&long).chars().count(), NOTE_TITLE_CHARS + 1);
    }
}
