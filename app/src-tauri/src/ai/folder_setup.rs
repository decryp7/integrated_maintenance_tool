//! "Set up analysis for this folder…": the survey (engine, no AI; cancellable, with progress), composing the
//! selected proposals into a validated profile, and the optional AI refinement. Applying the composed profile
//! goes through the normal `save_profile` (validated again, archived, undoable in the UI).
//!
//! Wire contract: `app/src/app/ai/folder-setup/folder-setup.types.ts`.

use super::commands::{core, WindowStore};
use super::error::{AiCmdError, AiErrorCode, AiResult};
use crate::windows::{read, CurrentWindow};
use crate::{CmdResult, State};
use imt_ai::assistant::FolderRefinement;
use imt_ai::StoreAccess;
use imt_core::survey::{self, ComposedProfile, Fragment, SurveyControl, SurveyError, SurveyProgress, SurveyRequest};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::Mutex;
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Runtime};

/// How often survey progress is sent.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);
/// Surveys one window may run at once (the wizard runs one; this bounds a misbehaving webview).
const MAX_PER_WINDOW: usize = 2;
/// Most fragments one compose request may carry.
const MAX_FRAGMENTS: usize = 200;

/// Running surveys by (window, survey id). App-wide managed state.
#[derive(Debug, Default)]
pub struct SurveyJobs(Mutex<HashMap<(String, String), SurveyControl>>);

/// Removes its job when the survey ends, however it ends.
struct JobGuard<'a> {
    jobs: &'a SurveyJobs,
    key: (String, String),
}

impl Drop for JobGuard<'_> {
    fn drop(&mut self) {
        self.jobs.lock().remove(&self.key);
    }
}

impl SurveyJobs {
    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<(String, String), SurveyControl>> {
        self.0.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn start(&self, window: &str, id: &str) -> CmdResult<(SurveyControl, JobGuard<'_>)> {
        let id = crate::activity::valid_op_id(id).ok_or("invalid survey id: use 1-64 letters, digits or '-'")?;
        let mut jobs = self.lock();
        if jobs.keys().filter(|(w, _)| w == window).count() >= MAX_PER_WINDOW {
            return Err("a folder survey is already running in this window; cancel it first".into());
        }
        let key = (window.to_string(), id);
        if jobs.contains_key(&key) {
            return Err("a survey with this id is already running".into());
        }
        let control = SurveyControl::new();
        jobs.insert(key.clone(), control.clone());
        Ok((control, JobGuard { jobs: self, key }))
    }

    fn cancel(&self, window: &str, id: &str) -> bool {
        match self.lock().get(&(window.to_string(), id.to_string())) {
            Some(c) => {
                c.cancel();
                true
            }
            None => false,
        }
    }
}

/// Sends progress every [`PROGRESS_EVERY`] (when it changed) until dropped.
struct Ticker(Option<mpsc::Sender<()>>);

impl Ticker {
    fn start(control: SurveyControl, channel: Channel<SurveyProgress>) -> Ticker {
        let (stop, stopped) = mpsc::channel::<()>();
        let spawned = std::thread::Builder::new().name("imt-survey-progress".into()).spawn(move || {
            let mut last = None;
            while let Err(RecvTimeoutError::Timeout) = stopped.recv_timeout(PROGRESS_EVERY) {
                let now = control.progress();
                if last != Some(now) {
                    last = Some(now);
                    if channel.send(now).is_err() {
                        break;
                    }
                }
            }
        });
        if let Err(e) = spawned {
            log::warn!("Survey progress is unavailable: {e}");
        }
        Ticker(Some(stop))
    }
}

impl Drop for Ticker {
    fn drop(&mut self) {
        self.0.take();
    }
}

/// `{ setup: FolderSetup }`, or `{ cancelled: true }` (a normal outcome).
fn outcome(r: Result<survey::FolderSetup, SurveyError>) -> anyhow::Result<Value> {
    match r {
        Ok(setup) => Ok(json!({ "setup": setup })),
        Err(SurveyError::Cancelled) => Ok(json!({ "cancelled": true })),
        Err(e) => Err(anyhow::anyhow!(e)),
    }
}

/// Surveys the window's files and returns validated heuristic proposals; cancellable with
/// [`folder_survey_cancel`] (same `surveyId`, same window), with progress on `onProgress`.
#[tauri::command]
pub async fn folder_survey(
    state: CurrentWindow,
    jobs: State<'_, SurveyJobs>,
    survey_id: String,
    request: Option<SurveyRequest>,
    on_progress: Channel<SurveyProgress>,
) -> CmdResult<Value> {
    let (control, _job) = jobs.start(state.label(), &survey_id)?;
    let _ticker = Ticker::start(control.clone(), on_progress.clone());
    let base = state.profile_doc().toml;
    let req = request.unwrap_or_default();
    let watched = control.clone();
    let out = read(&state, move |ws| outcome(ws.survey_folder(&base, &req, &watched))).await?;
    let _ = on_progress.send(control.progress());
    Ok(out)
}

/// Cancels the window's survey `surveyId`; false when it is not running.
#[tauri::command]
pub fn folder_survey_cancel(state: CurrentWindow, jobs: State<'_, SurveyJobs>, survey_id: String) -> bool {
    jobs.cancel(state.label(), &survey_id)
}

/// The active profile with `fragments` merged in, validated against the window's plugins, with its diff.
/// Nothing is saved: the UI applies `toml` with `save_profile`.
#[tauri::command]
pub async fn folder_setup_compose(state: CurrentWindow, fragments: Vec<Fragment>) -> CmdResult<ComposedProfile> {
    if fragments.is_empty() || fragments.len() > MAX_FRAGMENTS {
        return Err(format!("select 1 to {MAX_FRAGMENTS} proposals"));
    }
    let base = state.profile_doc().toml;
    read(&state, move |ws| survey::compose(ws, &base, &fragments)).await
}

/// AI proposals for the folder (the model refines the survey's facts; the engine validates every answer).
/// Cancel with `ai_cancel`. Fails with `unavailable` / `noModel` when no model is set up.
#[tauri::command]
pub async fn ai_refine_folder_setup<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    request: Option<SurveyRequest>,
) -> AiResult<FolderRefinement> {
    let core = core(&app)?;
    let flag = win.ai.begin_op();
    let assistant = match core.assistant(flag.clone()) {
        Ok(a) => a,
        Err(e) => {
            win.ai.end_op(&flag);
            return Err(e);
        }
    };
    let base = win.profile_doc().toml;
    let w = win.arc().clone();
    let req = request.unwrap_or_default();
    let r = tauri::async_runtime::spawn_blocking(move || {
        // The store is borrowed per engine step, never across the model's generation.
        let access = WindowStore(w);
        assistant
            .refine_folder_setup(&access as &dyn StoreAccess, &base, &req)
            .map_err(AiCmdError::from)
    })
    .await
    .map_err(|e| AiCmdError::new(AiErrorCode::Engine, format!("the AI task stopped unexpectedly ({e})")))
    .and_then(|r| r);
    win.ai.end_op(&flag);
    r
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, samples, test_app};
    use tauri::ipc::InvokeResponseBody;
    use tauri::Manager;

    fn sink() -> (Channel<SurveyProgress>, std::sync::Arc<Mutex<usize>>) {
        let seen = std::sync::Arc::new(Mutex::new(0usize));
        let s = seen.clone();
        let ch = Channel::new(move |_: InvokeResponseBody| {
            *s.lock().unwrap() += 1;
            Ok(())
        });
        (ch, seen)
    }

    #[test]
    fn survey_compose_and_cancel_over_ipc_state() {
        let app = test_app(None);
        let win = main_window(&app);
        win.store_write()
            .open(&[samples().join("serilog-clef"), samples().join("serilog-text")])
            .unwrap();
        let jobs = app.state::<SurveyJobs>();
        let (ch, _) = sink();
        let out = block(folder_survey(win.clone(), jobs.clone(), "s-1".into(), None, ch)).unwrap();
        let proposals = out["setup"]["proposals"].as_array().unwrap();
        let key = proposals
            .iter()
            .find(|p| p["id"] == "instance:MachineName")
            .expect("instance key proposal");
        assert_eq!(key["preview"]["kind"], "lanes");
        assert_eq!(jobs.lock().len(), 0, "the job is removed when the survey ends");
        // Compose the proposal's fragments.
        let fragments: Vec<Fragment> = serde_json::from_value(key["fragments"].clone()).unwrap();
        let composed = block(folder_setup_compose(win.clone(), fragments)).unwrap();
        assert!(composed.toml.contains("MachineName") && composed.added > 0);
        assert!(block(folder_setup_compose(win.clone(), vec![])).is_err());
        // Ids are validated; unknown surveys cannot be cancelled.
        let (ch, _) = sink();
        assert!(block(folder_survey(win.clone(), jobs.clone(), "bad id!".into(), None, ch)).is_err());
        assert!(!folder_survey_cancel(win.clone(), jobs.clone(), "nope".into()));
    }

    #[test]
    fn a_cancelled_survey_answers_cancelled() {
        let jobs = SurveyJobs::default();
        let (control, _guard) = jobs.start("main", "s-2").unwrap();
        assert!(jobs.cancel("main", "s-2"));
        assert!(control.is_cancelled());
        assert_eq!(outcome(Err(SurveyError::Cancelled)).unwrap(), json!({ "cancelled": true }));
        assert!(outcome(Err(SurveyError::NothingOpened)).is_err());
        assert!(jobs.start("main", "s-2").is_err(), "duplicate id");
        let _second = jobs.start("main", "s-3").unwrap();
        assert!(jobs.start("main", "s-4").is_err(), "per-window limit");
    }

    #[test]
    fn ai_refinement_is_unavailable_without_the_ai_runtime() {
        let app = test_app(None);
        let win = main_window(&app);
        let err = block(ai_refine_folder_setup(app.handle().clone(), win, None)).unwrap_err();
        assert_eq!(err.code, AiErrorCode::Unavailable);
    }
}
