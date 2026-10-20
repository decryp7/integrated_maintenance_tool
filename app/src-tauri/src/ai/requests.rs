//! AI requests as jobs: every use case (explain, summarise, root causes, the proposals, "Improve with AI")
//! and every chat turn runs as an AI job, one at a time (one model in memory), queued FIFO with the
//! chance to move or cancel waiting ones. Starting one never blocks the UI: the command waits for its
//! job, the user may close the dialog and get the result later from the Jobs pane or the notification
//! centre.
//!
//! The request is saved with the job (parsed again, and re-validated, on Retry), so a failed or
//! interrupted request can run again, also after a restart.

use super::commands::{core, sample_of, Emit, WindowStore};
use super::error::{AiCmdError, AiResult};
use crate::jobs::{self, Controls, Done, JobCtx, JobId, JobKind, JobLink, JobResult, JobSpec};
use crate::windows::WindowState;
use imt_ai::assistant::{Assistant, FolderRefinement, Narrative, Proposal};
use imt_ai::llm::LlmProvider;
use imt_ai::{StoreAccess, StoreRef};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};

pub(super) const MAX_LINES: usize = 50;
pub(super) const MAX_TEXT_CHARS: usize = 8000;
const MAX_SAMPLE_CHARS: usize = 64 * 1024;
const TITLE_SNIPPET: usize = 60;

type Llm = Arc<dyn LlmProvider>;

pub(super) fn bounded(what: &str, s: &str, max: usize) -> AiResult<String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(AiCmdError::invalid(format!("{what} is empty")));
    }
    if t.chars().count() > max {
        return Err(AiCmdError::invalid(format!("{what} is longer than {max} characters")));
    }
    Ok(t.to_string())
}

fn snippet(s: &str) -> String {
    let one_line: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match one_line.char_indices().nth(TITLE_SNIPPET) {
        Some((i, _)) => format!("{}…", &one_line[..i]),
        None => one_line,
    }
}

/// What an AI job does: the request as the UI made it (and as Retry makes it again).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "op")]
pub enum AiRequest {
    #[serde(rename_all = "camelCase")]
    Explain {
        record_id: u32,
    },
    Summarize {
        from: i64,
        to: i64,
    },
    RootCauses,
    Regex {
        lines: Vec<String>,
        intent: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Profile {
        record_id: Option<u32>,
        sample: Option<String>,
        file_name: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Pipeline {
        goal: String,
        record_id: Option<u32>,
        sample: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Markers {
        goal: String,
        record_id: Option<u32>,
        sample: Option<String>,
    },
    #[serde(rename_all = "camelCase")]
    Extractors {
        goal: String,
        record_id: Option<u32>,
        sample: Option<String>,
    },
    Query {
        question: String,
    },
    /// "Improve with AI" in the folder set-up (the survey request as the UI sent it).
    FolderSetup {
        request: Option<Value>,
    },
    /// A chat turn (Retry asks the question again).
    #[serde(rename_all = "camelCase")]
    Chat {
        session_id: String,
        text: String,
    },
}

/// What a use case answered.
#[derive(Debug, Clone)]
pub enum AiOutcome {
    Narrative(Narrative),
    Proposal(Proposal),
    Folder(FolderRefinement),
}

impl AiOutcome {
    fn result(&self) -> Option<JobResult> {
        let v = match self {
            AiOutcome::Narrative(n) => serde_json::to_value(n).ok().map(|narrative| JobResult::Narrative { narrative }),
            AiOutcome::Proposal(p) => serde_json::to_value(p).ok().map(|proposal| JobResult::Proposal { proposal }),
            AiOutcome::Folder(f) => serde_json::to_value(f).ok().map(|mut proposal| {
                if let Some(o) = proposal.as_object_mut() {
                    o.insert("kind".into(), Value::String("folderSetup".into()));
                }
                JobResult::Proposal { proposal }
            }),
        };
        v
    }

    #[cfg(test)]
    pub fn narrative(self) -> AiResult<Narrative> {
        match self {
            AiOutcome::Narrative(n) => Ok(n),
            _ => Err(AiCmdError::new(
                super::error::AiErrorCode::Engine,
                "the assistant answered something else",
            )),
        }
    }

    #[cfg(test)]
    pub fn proposal(self) -> AiResult<Proposal> {
        match self {
            AiOutcome::Proposal(p) => Ok(p),
            _ => Err(AiCmdError::new(
                super::error::AiErrorCode::Engine,
                "the assistant answered something else",
            )),
        }
    }

    #[cfg(test)]
    pub fn folder(self) -> AiResult<FolderRefinement> {
        match self {
            AiOutcome::Folder(f) => Ok(f),
            _ => Err(AiCmdError::new(
                super::error::AiErrorCode::Engine,
                "the assistant answered something else",
            )),
        }
    }
}

/// A proposal needs a sample: given, or the lines around a selected event (checked before the job is queued).
fn needs_source(record_id: Option<u32>, sample: &Option<String>) -> AiResult<()> {
    if record_id.is_none() && sample.as_deref().is_none_or(|s| s.trim().is_empty()) {
        return Err(AiCmdError::invalid("select an event in the file first"));
    }
    Ok(())
}

fn check_sample(sample: &Option<String>) -> AiResult<()> {
    match sample {
        Some(s) if s.chars().count() > MAX_SAMPLE_CHARS => Err(AiCmdError::invalid(format!(
            "the sample is longer than {MAX_SAMPLE_CHARS} characters"
        ))),
        _ => Ok(()),
    }
}

impl AiRequest {
    /// Checks the arguments (from the webview, or a saved project on Retry) before anything runs.
    pub fn validated(self) -> AiResult<AiRequest> {
        use AiRequest::*;
        Ok(match self {
            Regex { lines, intent } => {
                if lines.is_empty() || lines.len() > MAX_LINES || lines.iter().any(|l| l.chars().count() > MAX_TEXT_CHARS) {
                    return Err(AiCmdError::invalid(format!("give 1 to {MAX_LINES} example lines")));
                }
                let intent = intent.map(|i| i.chars().take(MAX_TEXT_CHARS).collect());
                Regex { lines, intent }
            }
            Profile {
                record_id,
                sample,
                file_name,
            } => {
                check_sample(&sample)?;
                needs_source(record_id, &sample)?;
                Profile {
                    record_id,
                    sample,
                    file_name,
                }
            }
            Pipeline { goal, record_id, sample } => {
                check_sample(&sample)?;
                needs_source(record_id, &sample)?;
                Pipeline {
                    goal: bounded("the goal", &goal, MAX_TEXT_CHARS)?,
                    record_id,
                    sample,
                }
            }
            Markers { goal, record_id, sample } => {
                check_sample(&sample)?;
                needs_source(record_id, &sample)?;
                Markers {
                    goal: bounded("the goal", &goal, MAX_TEXT_CHARS)?,
                    record_id,
                    sample,
                }
            }
            Extractors { goal, record_id, sample } => {
                check_sample(&sample)?;
                needs_source(record_id, &sample)?;
                Extractors {
                    goal: bounded("the goal", &goal, MAX_TEXT_CHARS)?,
                    record_id,
                    sample,
                }
            }
            Query { question } => Query {
                question: bounded("the question", &question, MAX_TEXT_CHARS)?,
            },
            Chat { session_id, text } => {
                if session_id.is_empty() || session_id.len() > 64 {
                    return Err(AiCmdError::invalid("unknown chat"));
                }
                Chat {
                    session_id,
                    text: bounded("the question", &text, MAX_TEXT_CHARS)?,
                }
            }
            other => other,
        })
    }

    pub fn title(&self) -> String {
        use AiRequest::*;
        match self {
            Explain { record_id } => format!("Explain event {record_id}"),
            Summarize { .. } => "Summarize a time range".into(),
            RootCauses => "Find likely root causes".into(),
            Regex { .. } => "Suggest a regex".into(),
            Profile { .. } => "Propose a parsing profile".into(),
            Pipeline { goal, .. } => format!("Propose pipeline steps: {}", snippet(goal)),
            Markers { goal, .. } => format!("Propose markers: {}", snippet(goal)),
            Extractors { goal, .. } => format!("Propose extractors: {}", snippet(goal)),
            Query { question } => format!("Write a query: {}", snippet(question)),
            FolderSetup { .. } => "Improve the folder set-up with AI".into(),
            Chat { text, .. } => format!("Chat: {}", snippet(text)),
        }
    }

    fn link(&self) -> Option<JobLink> {
        use AiRequest::*;
        match self {
            Explain { record_id } => Some(JobLink::Event { record_id: *record_id }),
            Summarize { from, to } => Some(JobLink::Range { from: *from, to: *to }),
            Profile { record_id: Some(id), .. }
            | Pipeline { record_id: Some(id), .. }
            | Markers { record_id: Some(id), .. }
            | Extractors { record_id: Some(id), .. } => Some(JobLink::Event { record_id: *id }),
            Chat { session_id, .. } => Some(JobLink::Chat {
                session_id: session_id.clone(),
            }),
            _ => None,
        }
    }

    pub fn spec(&self, window: &str) -> JobSpec {
        let mut spec = JobSpec::new(JobKind::Ai, self.title()).window(window);
        if let Some(l) = self.link() {
            spec = spec.link(l);
        }
        match serde_json::to_value(self) {
            Ok(v) => spec.retry_with(v),
            Err(_) => spec,
        }
    }

    /// Runs the use case (blocking). Chat turns are not use cases (see [`start_turn`]).
    fn execute(self, a: &Assistant<Llm>, store: StoreRef<'_>, base_profile: &str) -> AiResult<AiOutcome> {
        use AiRequest::*;
        Ok(match self {
            Explain { record_id } => AiOutcome::Narrative(a.explain_event(store, record_id)?),
            Summarize { from, to } => AiOutcome::Narrative(a.summarize(store, from, to)?),
            RootCauses => AiOutcome::Narrative(a.root_causes(store)?),
            Regex { lines, intent } => AiOutcome::Proposal(Proposal::Regex(a.propose_regex(&lines, intent.as_deref())?)),
            Profile {
                record_id,
                sample,
                file_name,
            } => {
                let (sample, file) = sample_of(store, record_id, sample, file_name)?;
                AiOutcome::Proposal(Proposal::Profile(a.propose_profile(store, &sample, &file)?))
            }
            Pipeline { goal, record_id, sample } => {
                let (sample, _) = sample_of(store, record_id, sample, None)?;
                AiOutcome::Proposal(Proposal::Pipeline(a.propose_pipeline(store, &goal, &sample, None)?))
            }
            Markers { goal, record_id, sample } => {
                let (sample, _) = sample_of(store, record_id, sample, None)?;
                AiOutcome::Proposal(Proposal::Markers(a.propose_markers(store, &goal, &sample, None)?))
            }
            Extractors { goal, record_id, sample } => {
                let (sample, _) = sample_of(store, record_id, sample, None)?;
                AiOutcome::Proposal(Proposal::Extractors(a.propose_extractors(store, &goal, &sample, None)?))
            }
            Query { question } => AiOutcome::Proposal(Proposal::Query(a.nl_to_query(store, &question)?)),
            FolderSetup { request } => {
                let req: imt_core::survey::SurveyRequest = match request {
                    Some(v) => {
                        serde_json::from_value(v).map_err(|e| AiCmdError::invalid(format!("the survey request is not valid: {e}")))?
                    }
                    None => Default::default(),
                };
                AiOutcome::Folder(a.refine_folder_setup(store, base_profile, &req)?)
            }
            Chat { .. } => return Err(AiCmdError::invalid("a chat turn is not a use case")),
        })
    }
}

/// The work of a use-case job: the window's store, the active model and the job's cancel flag (which
/// `ai_cancel` reaches too).
fn use_case_work(
    core: Arc<super::core::AiCore>,
    win: Arc<WindowState>,
    req: AiRequest,
) -> impl FnOnce(&JobCtx) -> AiResult<Done<AiOutcome>> + Send + 'static {
    move |ctx| {
        let flag = ctx.cancel_flag();
        win.ai.track_op(flag.clone());
        let r = (|| {
            let assistant = core.assistant(flag.clone())?;
            let base = win.profile_doc().toml;
            // The store is borrowed per engine step, never across the model's generation.
            let access = WindowStore(win.clone());
            req.execute(&assistant, (&access as &dyn StoreAccess).into(), &base)
        })();
        win.ai.end_op(&flag);
        r.map(|o| {
            let result = o.result();
            let done = Done::new(o);
            match result {
                Some(r) => done.with_result(r),
                None => done,
            }
        })
    }
}

/// Starts a chat turn as a job; the answer streams as `imt://ai-chat` events. Returns the job.
pub(crate) fn start_turn<R: Runtime>(
    app: &AppHandle<R>,
    win: &Arc<WindowState>,
    sid: &str,
    text: String,
    context: imt_ai::chat::ChatContext,
    emit: Emit,
) -> AiResult<JobId> {
    let core = core(app)?;
    core.provider()?;
    let req = AiRequest::Chat {
        session_id: sid.to_string(),
        text,
    }
    .validated()?;
    let AiRequest::Chat { text, .. } = &req else {
        return Err(AiCmdError::invalid("not a chat turn"));
    };
    let text = text.clone();
    let turn = win.ai.chats.begin_turn(sid).map_err(|e| {
        if e.contains("no longer") {
            AiCmdError::not_found(e)
        } else {
            AiCmdError::busy(e)
        }
    })?;
    let spec = req.spec(win.label());
    let cancel = turn.cancel.clone();
    let pending = PendingTurn {
        turn: Some(turn),
        win: win.clone(),
        sid: sid.to_string(),
        emit: emit.clone(),
    };
    let browser = crate::browser::TurnBrowser::begin(app, win.label(), sid);
    let (w, session) = (win.clone(), sid.to_string());
    let work = move |_: &JobCtx| -> AiResult<Done<()>> {
        let mut pending = pending;
        let Some(turn) = pending.turn.take() else {
            return Err(AiCmdError::invalid("the turn already ended"));
        };
        super::commands::run_turn(&core, &w, &session, (&text, &context), turn, &emit, browser.as_ref())?;
        Ok(Done::new(()).with_result(JobResult::Chat {
            session_id: session.clone(),
        }))
    };
    // The job's Cancel stops the turn like the chat's Stop.
    let controls = Controls::cancel(move || cancel.store(true, std::sync::atomic::Ordering::Relaxed));
    jobs::spawn(app, spec, controls, work).map_err(AiCmdError::busy)
}

/// A turn that was queued but never ran (cancelled while waiting, IMT closing, the queue refused it):
/// the chat is freed and told, so it never stays "answering".
struct PendingTurn {
    turn: Option<super::chats::TurnHandle>,
    win: Arc<WindowState>,
    sid: String,
    emit: Emit,
}

impl Drop for PendingTurn {
    fn drop(&mut self) {
        let Some(turn) = self.turn.take() else { return };
        let env = imt_ai::chat::Envelope {
            seq: 0,
            event: imt_ai::chat::ChatEvent::Error {
                message: "Stopped before it started.".into(),
                cancelled: true,
            },
        };
        (self.emit)(&self.sid, &env);
        let record = turn.session.lock().unwrap_or_else(|p| p.into_inner()).to_record();
        self.win.ai.chats.end_turn(&self.sid, record);
    }
}

/// Retry of an AI job: its saved request runs again as a new job of the same window.
pub(super) fn retry<R: Runtime>(app: &AppHandle<R>, job: &jobs::model::Job) -> Result<Option<JobId>, String> {
    let req: AiRequest = job
        .request
        .clone()
        .and_then(|v| serde_json::from_value(v).ok())
        .ok_or("this AI request cannot be run again")?;
    let label = job.window().ok_or("this AI request has no window")?;
    let win = app
        .try_state::<crate::windows::Windows>()
        .and_then(|w| w.get(label))
        .ok_or("the window of this request is closed")?;
    let core = core(app).map_err(|e| e.message)?;
    core.provider().map_err(|e| e.message)?;
    match req {
        AiRequest::Chat { session_id, text } => {
            let emit = super::commands::chat_emitter(app, label);
            start_turn(app, &win, &session_id, text, Default::default(), emit)
                .map(Some)
                .map_err(|e| e.message)
        }
        other => start_use_case(app, &win, other).map(Some).map_err(|e| e.message),
    }
}

/// Starts a use case as a background AI job of `win` (nobody waits for it: its result is the job's); returns it.
pub(crate) fn start_use_case<R: Runtime>(app: &AppHandle<R>, win: &Arc<WindowState>, req: AiRequest) -> AiResult<JobId> {
    let core = core(app)?;
    // Fail fast (no model, AI off) instead of queueing a request that cannot run.
    core.provider()?;
    let req = req.validated()?;
    let spec = req.spec(win.label());
    jobs::spawn(app, spec, Controls::default(), use_case_work(core, win.clone(), req)).map_err(AiCmdError::busy)
}

/// Cancels the window's use cases, running and waiting (not its chat turns, which have their own Stop). Returns how many.
pub(super) fn cancel_use_cases<R: Runtime>(app: &AppHandle<R>, window: &str) -> usize {
    let Some(jobs) = app.try_state::<jobs::Jobs>() else { return 0 };
    let ids = jobs.find_active(|j| {
        j.kind == JobKind::Ai
            && j.window() == Some(window)
            && !j.state().is_finished()
            && !matches!(j.source.link, Some(JobLink::Chat { .. }))
    });
    ids.into_iter().filter(|id| jobs.cancel(*id).is_ok()).count()
}

/// Cancels the chat's waiting turn, if any. True when one was waiting.
pub(super) fn cancel_queued_turn<R: Runtime>(app: &AppHandle<R>, window: &str, sid: &str) -> bool {
    let Some(jobs) = app.try_state::<jobs::Jobs>() else { return false };
    let ids = jobs.find_active(|j| {
        j.window() == Some(window)
            && j.state() == jobs::JobState::Queued
            && matches!(&j.source.link, Some(JobLink::Chat { session_id }) if session_id == sid)
    });
    ids.into_iter().any(|id| jobs.cancel(id).is_ok())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn requests_validate_title_and_link_themselves() {
        assert!(AiRequest::Regex {
            lines: vec![],
            intent: None
        }
        .validated()
        .is_err());
        assert!(AiRequest::Query { question: " ".into() }.validated().is_err());
        assert!(AiRequest::Pipeline {
            goal: "g".into(),
            record_id: None,
            sample: Some("x".repeat(MAX_SAMPLE_CHARS + 1)),
        }
        .validated()
        .is_err());
        assert!(AiRequest::Chat {
            session_id: String::new(),
            text: "hi".into()
        }
        .validated()
        .is_err());
        let chat = AiRequest::Chat {
            session_id: "c-1".into(),
            text: "why did the payment service fail at 10:42 yesterday afternoon and what should we do".into(),
        };
        assert_eq!(chat.title(), "Chat: why did the payment service fail at 10:42 yesterday afternoo…");
        let spec = chat.spec("main");
        assert_eq!(spec.source.link, Some(JobLink::Chat { session_id: "c-1".into() }));
        let saved = spec.request.unwrap();
        assert_eq!(saved["op"], "chat");
        assert_eq!(serde_json::from_value::<AiRequest>(saved).unwrap(), chat);
        let explain = AiRequest::Explain { record_id: 7 }.spec("main");
        assert_eq!(explain.title, "Explain event 7");
        assert_eq!(explain.request.unwrap(), serde_json::json!({"op": "explain", "recordId": 7}));
        assert_eq!(AiRequest::RootCauses.validated().unwrap().link(), None);
        assert_eq!(
            AiRequest::Markers {
                goal: "g".into(),
                record_id: Some(3),
                sample: None
            }
            .link(),
            Some(JobLink::Event { record_id: 3 })
        );
        for r in [
            AiRequest::Summarize { from: 0, to: 1 },
            AiRequest::Regex {
                lines: vec!["a".into()],
                intent: Some("i".into()),
            },
            AiRequest::Profile {
                record_id: Some(1),
                sample: None,
                file_name: None,
            },
            AiRequest::Extractors {
                goal: "g".into(),
                record_id: None,
                sample: Some("a line".into()),
            },
            AiRequest::FolderSetup { request: None },
        ] {
            assert!(!r.title().is_empty());
            assert!(r.clone().validated().is_ok());
        }
    }

    #[test]
    fn outcomes_become_results_view_can_open() {
        let p = Proposal::Query(imt_ai::assistant::QueryProposal {
            ql: "level >= error".into(),
            explanation: "Errors.".into(),
            estimated_matches: None,
            repaired: false,
        });
        let o = AiOutcome::Proposal(p);
        assert!(matches!(o.result(), Some(JobResult::Proposal { proposal }) if proposal["kind"] == "query"));
        assert!(o.clone().narrative().is_err());
        assert!(o.clone().folder().is_err());
        assert!(o.proposal().is_ok());
        let f = AiOutcome::Folder(FolderRefinement::default());
        assert!(matches!(f.result(), Some(JobResult::Proposal { proposal }) if proposal["kind"] == "folderSetup"));
        assert!(f.proposal().is_err());
    }
}
