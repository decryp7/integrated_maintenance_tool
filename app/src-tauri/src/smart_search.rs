//! The background scan of smart search (`search_partial`): the search box answers from the full-text index at
//! once (a word search, `textMode: "words"`); this scan then counts the events that hold the text only as a
//! substring, so the UI can add them ("N more partial matches added", `textMode: "wordsOrSubstring"`).
//!
//! The scan is an analysis job (Cancel in the Jobs pane stops it in the store). A window runs one scan at a
//! time: a newer search cancels the previous scan, so typing never queues up full scans.

use crate::jobs::{Controls, JobId};
use crate::windows::CurrentWindow;
use crate::CmdResult;
use imt_core::ports::{Filter, PartialMatches, StoreError};
use std::collections::HashMap;
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};

/// The running scan of each window.
static RUNNING: Mutex<Option<HashMap<String, JobId>>> = Mutex::new(None);

fn with_running<T>(f: impl FnOnce(&mut HashMap<String, JobId>) -> T) -> T {
    let mut running = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
    f(running.get_or_insert_with(HashMap::new))
}

/// Records `id` as the window's scan; returns the scan it supersedes.
fn started(label: &str, id: JobId) -> Option<JobId> {
    with_running(|m| m.insert(label.to_string(), id))
}

/// Forgets the window's scan when it is still `id` (a newer scan stays).
fn finished(label: &str, id: Option<JobId>) {
    with_running(|m| {
        if id.is_some() && m.get(label).copied() == id {
            m.remove(label);
        }
    });
}

/// Counts the events matching `query` whose text matches only as a substring (not as words); `null` when the
/// scan was cancelled (by the user or by a newer search of the window).
#[tauri::command]
pub async fn search_partial<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, query: Filter) -> CmdResult<Option<PartialMatches>> {
    let label = state.label().to_string();
    let jobs = app.try_state::<crate::jobs::Jobs>().map(|j| j.inner().clone());
    let mine = std::sync::Arc::new(Mutex::new(None));
    let announce = {
        let (label, mine) = (label.clone(), mine.clone());
        move |id: JobId| {
            *mine.lock().unwrap_or_else(|p| p.into_inner()) = Some(id);
            if let (Some(previous), Some(jobs)) = (started(&label, id), jobs.as_ref()) {
                // Already finished or gone: nothing to stop.
                let _ = jobs.cancel(previous);
            }
        }
    };
    let ql = Some(query.ql.clone());
    let out = crate::jobs::analysis_announced(
        &app,
        &state,
        "Find partial matches",
        "grid",
        ql,
        Controls::default(),
        announce,
        move |store, _| Ok(store.partial_matches(&query)),
    )
    .await;
    finished(&label, *mine.lock().unwrap_or_else(|p| p.into_inner()));
    let out = match out {
        Err(e) if e.code == crate::ErrorCode::Cancelled => return Ok(None),
        other => other?,
    };
    match out {
        Ok(found) => Ok(Some(found)),
        Err(StoreError::Cancelled) => Ok(None),
        Err(e) => Err(e.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, samples, test_app};
    use imt_core::query::TextMode;

    #[test]
    fn counts_partial_matches_of_the_window() {
        let app = test_app(None);
        let words = |text: &str| Filter {
            text: text.into(),
            text_mode: TextMode::Words,
            ..Filter::default()
        };
        // Nothing opened: nothing found, not an error.
        let none = block(search_partial(app.handle().clone(), main_window(&app), words("imeout"))).unwrap();
        assert_eq!(none.map(|m| m.added).unwrap_or_default(), 0);
        crate::test_support::import(&main_window(&app), &[samples().join("serilog-clef")]);
        let found = block(search_partial(app.handle().clone(), main_window(&app), words("imeout")))
            .unwrap()
            .expect("not cancelled");
        assert!(found.complete);
        let page = block(crate::workspace_commands::query_page(
            main_window(&app),
            imt_core::ports::PageQuery {
                query: Filter {
                    text: "imeout".into(),
                    ..Filter::default()
                },
                ..Default::default()
            },
        ))
        .unwrap();
        assert_eq!(
            found.added as usize, page.total,
            "every substring match of `imeout` is a partial match"
        );
        assert!(with_running(|m| m.get("main").is_none()), "the finished scan is forgotten");
        // A newer scan is not forgotten when an older one finishes.
        assert!(started("w", JobId(1)).is_none());
        assert_eq!(started("w", JobId(2)), Some(JobId(1)));
        finished("w", Some(JobId(1)));
        assert_eq!(with_running(|m| m.get("w").copied()), Some(JobId(2)));
        finished("w", Some(JobId(2)));
        finished("w", None);
        assert!(with_running(|m| m.get("w").is_none()));
    }
}
