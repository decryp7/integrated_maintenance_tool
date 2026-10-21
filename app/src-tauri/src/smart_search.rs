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

fn swap(label: &str, id: Option<JobId>) -> Option<JobId> {
    let mut running = RUNNING.lock().unwrap_or_else(|p| p.into_inner());
    let map = running.get_or_insert_with(HashMap::new);
    match id {
        Some(id) => map.insert(label.to_string(), id),
        None => map.remove(label),
    }
}

/// Counts the events matching `query` whose text matches only as a substring (not as words); `null` when the
/// scan was cancelled (by the user or by a newer search of the window).
#[tauri::command]
pub async fn search_partial<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, query: Filter) -> CmdResult<Option<PartialMatches>> {
    let label = state.label().to_string();
    let jobs = app.try_state::<crate::jobs::Jobs>().map(|j| j.inner().clone());
    let announce = {
        let label = label.clone();
        move |id: JobId| {
            if let (Some(previous), Some(jobs)) = (swap(&label, Some(id)), jobs.as_ref()) {
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
    swap(&label, None);
    match out? {
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
        assert!(swap("main", None).is_none(), "the finished scan is forgotten");
    }
}
