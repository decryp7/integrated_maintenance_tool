//! "Set up analysis for this folder…": the survey (engine, no AI; a background analysis job with progress and
//! Cancel) and composing the selected proposals into a validated profile. Applying the composed profile goes
//! through the normal `save_profile` (validated again, archived, undoable in the UI).
//!
//! Wire contract: `app/src/app/ai/folder-setup/folder-setup.types.ts`.

use crate::jobs::{Controls, Done, JobKind, JobProgress, JobResult, JobSpec};
use crate::windows::{read, CurrentWindow};
use crate::CmdResult;
use imt_core::survey::{self, ComposedProfile, Fragment, SurveyControl, SurveyError, SurveyProgress, SurveyRequest};
use serde_json::{json, Value};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::time::Duration;
use tauri::ipc::Channel;
use tauri::{AppHandle, Runtime};

/// How often survey progress is sent.
const PROGRESS_EVERY: Duration = Duration::from_millis(150);
/// Most fragments one compose request may carry.
const MAX_FRAGMENTS: usize = 200;

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

/// Starts surveying the window's files as a background analysis job and returns it at once. Progress goes to
/// `onProgress` while it runs; Cancel is the job's (`jobs_cancel`). The validated heuristic proposals are the
/// job's result (`jobs_wait` while the wizard is open; otherwise "Finished · View" reopens the wizard with them).
#[tauri::command]
pub fn folder_survey<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    request: Option<SurveyRequest>,
    on_progress: Channel<SurveyProgress>,
) -> CmdResult<u64> {
    let base = state.profile_doc().toml;
    let req = request.unwrap_or_default();
    let control = SurveyControl::new();
    let (stop, probe) = (control.clone(), control.clone());
    let controls = Controls::cancel(move || stop.cancel()).with_probe(move || {
        let p = probe.progress();
        Some(JobProgress::counts(p.done as u64, p.total as u64))
    });
    let spec = JobSpec::new(JobKind::Analysis, "Survey the folder").window(state.label());
    let win = state.arc().clone();
    crate::jobs::spawn(&app, spec, controls, move |_| {
        let ticker = Ticker::start(control.clone(), on_progress.clone());
        let out = crate::jobs::read_store(&win, |ws| outcome(ws.survey_folder(&base, &req, &control)));
        drop(ticker);
        let _ = on_progress.send(control.progress());
        let out = out.map_err(crate::err)?;
        let cancelled = out.get("cancelled").is_some();
        Ok::<_, String>(
            Done::new(())
                .with_result(JobResult::FolderSurvey { outcome: out })
                .cancelled_if(cancelled),
        )
    })
    .map(|id| id.0)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, samples, test_app};
    use std::sync::Mutex;
    use tauri::ipc::InvokeResponseBody;

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
    fn the_survey_runs_as_a_job_and_its_proposals_compose() {
        let app = test_app(None);
        let win = main_window(&app);
        win.store_write()
            .open(&[samples().join("serilog-clef"), samples().join("serilog-text")])
            .unwrap();
        let (ch, seen) = sink();
        // The command returns the job at once; the proposals are its result.
        let id = folder_survey(app.handle().clone(), win.clone(), None, ch).unwrap();
        let end = crate::test_support::job_end(&app, id);
        assert_eq!(end.state, crate::jobs::JobState::Done, "{end:?}");
        assert!(*seen.lock().unwrap() > 0, "progress was sent");
        let Some(JobResult::FolderSurvey { outcome: out }) = end.result else {
            panic!("{:?}", end.result)
        };
        let proposals = out["setup"]["proposals"].as_array().unwrap();
        let key = proposals
            .iter()
            .find(|p| p["id"] == "instance:MachineName")
            .expect("instance key proposal");
        assert_eq!(key["preview"]["kind"], "lanes");
        // Compose the proposal's fragments.
        let fragments: Vec<Fragment> = serde_json::from_value(key["fragments"].clone()).unwrap();
        let composed = block(folder_setup_compose(win.clone(), fragments)).unwrap();
        assert!(composed.toml.contains("MachineName") && composed.added > 0);
        assert!(block(folder_setup_compose(win.clone(), vec![])).is_err());
    }

    #[test]
    fn a_cancelled_survey_answers_cancelled() {
        assert_eq!(outcome(Err(SurveyError::Cancelled)).unwrap(), json!({ "cancelled": true }));
        assert!(outcome(Err(SurveyError::NothingOpened)).is_err());
    }
}
