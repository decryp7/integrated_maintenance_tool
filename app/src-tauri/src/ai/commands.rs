//! The AI commands. Thin: they validate the IPC arguments, run the library off the UI thread and turn its
//! errors into [`AiCmdError`]s. Use cases and chat turns are AI jobs ([`super::requests`]); model
//! downloads are download jobs. Set-up stages and chat answers stream as events.

use super::chats::SessionInfo;
use super::core::{AiCore, AiStatus, ModelDto, SetupEvent};
use super::error::{AiCmdError, AiErrorCode, AiResult};
use super::requests::{self, AiRequest};
use super::{AiState, CHAT_EVENT, SETUP_EVENT, STATUS_EVENT};
use crate::jobs::{self, Controls, Done, JobId, JobKind, JobLink, JobProgress, JobResult, JobSpec};
use crate::windows::{emit_to_window, CurrentWindow, WindowState};
use imt_ai::assistant::{FolderRefinement, Narrative, Proposal};
use imt_ai::chat::{export_markdown, suggested_prompts, ChatContext, ChatEnv, ChatEvent, ChatHistoryRecord, ChatRecord, Envelope};
use imt_ai::hardware::Hardware;
use imt_ai::recommend::{Recommendation, Recommendations};
use imt_ai::setup::{SetupPlan, SetupStage};
use imt_ai::{StoreAccess, StoreRef};
use imt_core::api::ContextRequest;
use imt_core::LogStore;
use serde::Serialize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Emitter, Manager, Runtime};

/// Lines on each side of an event used as the sample for profile, pipeline, marker and extractor proposals.
const SAMPLE_RADIUS: usize = 30;
const MAX_SAMPLE_CHARS: usize = 64 * 1024;
const MAX_ID_CHARS: usize = 64;

type Ai<T> = AiResult<T>;

pub(crate) fn core<R: Runtime>(app: &AppHandle<R>) -> Ai<Arc<AiCore>> {
    app.try_state::<AiState>()
        .map(|s| s.0.clone())
        .ok_or_else(|| AiCmdError::unavailable("The AI assistant is not available in this build of IMT."))
}

/// Runs `f` on the blocking pool; a panic becomes an engine error instead of taking the app down.
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> Ai<T> + Send + 'static) -> Ai<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| AiCmdError::new(AiErrorCode::Engine, format!("the AI task stopped unexpectedly ({e})")))?
}

fn emit_status<R: Runtime>(app: &AppHandle<R>, core: &AiCore) {
    let _ = app.emit(STATUS_EVENT, core.status());
}

fn check_session_id(id: &str) -> Ai<&str> {
    if id.is_empty() || id.len() > MAX_ID_CHARS {
        return Err(AiCmdError::invalid("unknown chat"));
    }
    Ok(id)
}

/// Runs a use case as an AI job of the calling window and waits for its answer.
async fn use_case<R: Runtime>(app: &AppHandle<R>, win: &CurrentWindow, req: AiRequest) -> Ai<requests::AiOutcome> {
    requests::run(app, win.arc(), req).await
}

/// A window's workspace for the assistant: every visit is one short read lock (see `imt_ai::StoreAccess`).
pub(crate) struct WindowStore(pub(crate) Arc<WindowState>);

impl StoreAccess for WindowStore {
    fn visit(&self, f: &mut dyn FnMut(&dyn LogStore)) {
        f(self.0.store_read().as_ref())
    }
}

/// The sample text and file name for a proposal: given by the UI, or the lines around `record_id`.
pub(super) fn sample_of(store: StoreRef<'_>, record_id: Option<u32>, sample: Option<String>, file: Option<String>) -> Ai<(String, String)> {
    if let Some(s) = sample.filter(|s| !s.trim().is_empty()) {
        if s.chars().count() > MAX_SAMPLE_CHARS {
            return Err(AiCmdError::invalid(format!(
                "the sample is longer than {MAX_SAMPLE_CHARS} characters"
            )));
        }
        let name = file
            .filter(|f| !f.trim().is_empty() && f.len() <= 255)
            .unwrap_or_else(|| "sample.log".into());
        return Ok((s, name));
    }
    let id = record_id.ok_or_else(|| AiCmdError::invalid("select an event in the file first"))?;
    let req = ContextRequest {
        record_id: id,
        before: SAMPLE_RADIUS,
        after: SAMPLE_RADIUS,
    };
    let ctx = store
        .with(|s| s.source_context(&req))
        .map_err(|e| AiCmdError::not_found(format!("cannot read the file of event {id}: {e:#}")))?;
    let text = ctx.lines.iter().map(|l| l.text.as_str()).collect::<Vec<_>>().join("\n");
    Ok((text, ctx.source_name))
}

// ---- status, hardware, catalog, set-up ----

#[tauri::command]
pub fn ai_status<R: Runtime>(app: AppHandle<R>) -> Ai<AiStatus> {
    Ok(core(&app)?.status())
}

#[tauri::command]
pub async fn ai_hardware<R: Runtime>(app: AppHandle<R>) -> Ai<Hardware> {
    let core = core(&app)?;
    blocking(move || Ok(core.hardware())).await
}

#[tauri::command]
pub async fn ai_recommend<R: Runtime>(app: AppHandle<R>) -> Ai<Recommendations> {
    let core = core(&app)?;
    blocking(move || {
        core.availability()?;
        Ok(core.recommendations())
    })
    .await
}

/// Every catalog model with its fit on this machine, best first.
#[tauri::command]
pub async fn ai_catalog<R: Runtime>(app: AppHandle<R>) -> Ai<Vec<Recommendation>> {
    let core = core(&app)?;
    blocking(move || Ok(core.recommendations().ranked)).await
}

#[tauri::command]
pub async fn ai_setup_plan<R: Runtime>(app: AppHandle<R>, model_id: Option<String>) -> Ai<SetupPlan> {
    let core = core(&app)?;
    blocking(move || core.plan(model_id.as_deref())).await
}

/// A set-up that downloads a model: shown as a download job (Cancel stops it, Retry runs it again).
struct Download {
    title: String,
    model_id: String,
    file_name: String,
}

type SetupWork = Box<dyn FnOnce(&AiCore, &imt_ai::models::Progress, &mut dyn FnMut(SetupStage)) -> Ai<()> + Send>;

/// Runs a set-up or activation off the UI thread, streaming `imt://ai-setup`. A set-up that downloads is a
/// download job; an activation (the model is on disk) runs on its own thread.
fn spawn_setup<R: Runtime>(app: AppHandle<R>, core: Arc<AiCore>, download: Option<Download>, work: SetupWork) -> Ai<()> {
    let progress = core.begin_setup()?;
    let run = {
        let (app, core) = (app.clone(), core.clone());
        move |on_progress: &dyn Fn(JobProgress)| -> Ai<()> {
            let a = app.clone();
            let mut on_stage = |st: SetupStage| {
                if let SetupStage::Downloading { bytes, total, .. } = &st {
                    on_progress(JobProgress::counts(*bytes, *total));
                }
                let _ = a.emit(SETUP_EVENT, SetupEvent::Stage(st));
            };
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&core, &progress, &mut on_stage)))
                .unwrap_or_else(|_| Err(AiCmdError::new(AiErrorCode::Engine, "the set-up stopped unexpectedly")));
            core.end_setup();
            if let Err(e) = &r {
                log::warn!(Code = format!("{:?}", e.code).as_str(); "AI set-up did not finish: {}", e.message);
            }
            if let Some(ev) = SetupEvent::from_result(&r) {
                let _ = app.emit(SETUP_EVENT, ev);
            }
            emit_status(&app, &core);
            r
        }
    };
    let Some(d) = download else {
        let spawned = std::thread::Builder::new().name("imt-ai-setup".into()).spawn(move || {
            let _ = run(&|_| {});
        });
        return spawned
            .map(|_| ())
            .map_err(|e| AiCmdError::new(AiErrorCode::Io, format!("cannot start the set-up: {e}")));
    };
    let spec = JobSpec::new(JobKind::Download, d.title)
        .link(JobLink::Model { id: d.model_id.clone() })
        .retry_with(json!({"op": "model", "modelId": d.model_id}));
    let c = core.clone();
    let controls = Controls::cancel(move || {
        c.cancel_setup();
    });
    let file_name = d.file_name;
    let queued = jobs::spawn(&app, spec, controls, move |ctx: &jobs::JobCtx| -> Ai<Done<()>> {
        run(&|p| ctx.progress(p))?;
        Ok(Done::new(()).with_result(JobResult::Model {
            file_name: Some(file_name.clone()),
        }))
    });
    if let Err(e) = queued {
        core.end_setup();
        return Err(AiCmdError::busy(e));
    }
    Ok(())
}

/// Retry of a model or runtime download job: the download starts again as a new job.
pub(crate) fn retry_download<R: Runtime>(app: &AppHandle<R>, job: &jobs::model::Job) -> Result<Option<JobId>, String> {
    let req = job.request.clone().unwrap_or_default();
    let r = match (req["op"].as_str(), req["modelId"].as_str(), req["runtimeId"].as_str()) {
        (Some("model"), Some(id), _) => ai_setup_run(app.clone(), id.to_string()),
        (Some("runtime"), _, Some(id)) => super::runtime_commands::ai_runtime_install(app.clone(), id.to_string()),
        _ => return Err("this download cannot be run again".into()),
    };
    r.map(|_| None).map_err(|e| e.message)
}

/// Downloads (when needed), verifies, loads and warms up a catalog model, then makes it active.
/// Returns at once; progress arrives as `imt://ai-setup` events ending in `ready`, `failed` or `cancelled`.
#[tauri::command]
pub fn ai_setup_run<R: Runtime>(app: AppHandle<R>, model_id: String) -> Ai<()> {
    let core = core(&app)?;
    let entry = core
        .catalog()
        .find(&model_id)
        .map(|m| (m.name.clone(), m.file.clone()))
        .ok_or_else(|| AiCmdError::not_found(format!("'{model_id}' is not in the model list")))?;
    let installed = core.models().iter().any(|m| m.file_name == entry.1);
    let download = (!installed).then(|| Download {
        title: format!("Downloading AI model {}", entry.0),
        model_id: model_id.clone(),
        file_name: entry.1.clone(),
    });
    spawn_setup(app, core, download, Box::new(move |c, p, on| c.run_setup(&model_id, p, on)))
}

#[tauri::command]
pub fn ai_setup_cancel<R: Runtime>(app: AppHandle<R>) -> Ai<bool> {
    Ok(core(&app)?.cancel_setup())
}

#[tauri::command]
pub async fn ai_models<R: Runtime>(app: AppHandle<R>) -> Ai<Vec<ModelDto>> {
    let core = core(&app)?;
    blocking(move || Ok(core.models())).await
}

/// Loads an installed model and makes it active (same events as the set-up).
#[tauri::command]
pub fn ai_model_activate<R: Runtime>(app: AppHandle<R>, file_name: String) -> Ai<()> {
    let core = core(&app)?;
    if !imt_ai::catalog::is_safe_file_name(&file_name) {
        return Err(AiCmdError::invalid(format!("'{file_name}' is not a model file name")));
    }
    spawn_setup(app, core, None, Box::new(move |c, p, on| c.activate(&file_name, p, on)))
}

#[tauri::command]
pub async fn ai_model_delete<R: Runtime>(app: AppHandle<R>, file_name: String) -> Ai<()> {
    let core = core(&app)?;
    let c = core.clone();
    blocking(move || c.delete_model(&file_name)).await?;
    emit_status(&app, &core);
    Ok(())
}

/// Adds a `.gguf` the user picked in the open dialog to the models folder. A catalog model (a manual
/// download) is checked against its SHA-256 first, as a download job with progress that Cancel stops.
#[tauri::command]
pub async fn ai_model_add_local<R: Runtime>(app: AppHandle<R>, path: String) -> Ai<ModelDto> {
    let core = core(&app)?;
    core.availability()?;
    let name = Path::new(&path)
        .file_name()
        .map(|n| n.to_string_lossy().chars().take(120).collect::<String>())
        .unwrap_or_default();
    let progress = Arc::new(imt_ai::models::Progress::new());
    let (p, q) = (progress.clone(), progress.clone());
    let controls = Controls::cancel(move || p.cancel()).with_probe(move || {
        let s = q.snapshot();
        (s.total > 0).then(|| JobProgress::counts(s.bytes, s.total))
    });
    let spec = JobSpec::new(JobKind::Download, format!("Checking the model file {name}")).link(JobLink::Path { path: path.clone() });
    jobs::run(&app, spec, controls, move |_ctx: &jobs::JobCtx| -> Ai<Done<ModelDto>> {
        let m = core.add_local(&path, &progress)?;
        let file_name = Some(m.file_name.clone());
        Ok(Done::new(m).with_result(JobResult::Model { file_name }))
    })
    .await
}

/// Lets the engine run again after it was switched off for the session (quarantine) or a load failed.
#[tauri::command]
pub fn ai_reset<R: Runtime>(app: AppHandle<R>) -> Ai<AiStatus> {
    let core = core(&app)?;
    core.reset();
    // The paused AI queue goes on.
    if let Some(jobs) = app.try_state::<jobs::Jobs>() {
        jobs.pump();
    }
    emit_status(&app, &core);
    Ok(core.status())
}

// ---- use cases ----
// Each runs as an AI job (one at a time, queued FIFO); the command waits for its answer. Proposals go out
// as the tagged `Proposal` (`"kind": "pipeline"`, ...), never as the bare struct: the UI picks the evidence
// and the Apply action by `kind` (api contract: `Proposal` in app/src/app/ai/ai.types.ts).

#[tauri::command]
pub async fn ai_propose_regex<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    lines: Vec<String>,
    intent: Option<String>,
) -> Ai<Proposal> {
    use_case(&app, &win, AiRequest::Regex { lines, intent }).await?.proposal()
}

#[tauri::command]
pub async fn ai_propose_profile<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    record_id: Option<u32>,
    sample: Option<String>,
    file_name: Option<String>,
) -> Ai<Proposal> {
    use_case(
        &app,
        &win,
        AiRequest::Profile {
            record_id,
            sample,
            file_name,
        },
    )
    .await?
    .proposal()
}

#[tauri::command]
pub async fn ai_propose_pipeline<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    goal: String,
    record_id: Option<u32>,
    sample: Option<String>,
) -> Ai<Proposal> {
    use_case(&app, &win, AiRequest::Pipeline { goal, record_id, sample })
        .await?
        .proposal()
}

#[tauri::command]
pub async fn ai_propose_markers<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    goal: String,
    record_id: Option<u32>,
    sample: Option<String>,
) -> Ai<Proposal> {
    use_case(&app, &win, AiRequest::Markers { goal, record_id, sample })
        .await?
        .proposal()
}

#[tauri::command]
pub async fn ai_propose_extractors<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    goal: String,
    record_id: Option<u32>,
    sample: Option<String>,
) -> Ai<Proposal> {
    use_case(&app, &win, AiRequest::Extractors { goal, record_id, sample })
        .await?
        .proposal()
}

#[tauri::command]
pub async fn ai_nl_to_query<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, question: String) -> Ai<Proposal> {
    use_case(&app, &win, AiRequest::Query { question }).await?.proposal()
}

#[tauri::command]
pub async fn ai_explain_event<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, record_id: u32) -> Ai<Narrative> {
    use_case(&app, &win, AiRequest::Explain { record_id }).await?.narrative()
}

#[tauri::command]
pub async fn ai_summarize<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, from: i64, to: i64) -> Ai<Narrative> {
    use_case(&app, &win, AiRequest::Summarize { from, to }).await?.narrative()
}

#[tauri::command]
pub async fn ai_root_causes<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> Ai<Narrative> {
    use_case(&app, &win, AiRequest::RootCauses).await?.narrative()
}

/// "Improve with AI" in the folder set-up, as an AI job (the survey request as the UI sent it).
pub(crate) async fn refine_folder_setup<R: Runtime>(
    app: &AppHandle<R>,
    win: &CurrentWindow,
    request: Option<Value>,
) -> Ai<FolderRefinement> {
    use_case(app, win, AiRequest::FolderSetup { request }).await?.folder()
}

/// Stops this window's use cases: the running ones fail with `cancelled`, the waiting ones are dropped
/// (chat turns have their own Stop). Returns how many.
#[tauri::command]
pub fn ai_cancel<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> Ai<usize> {
    core(&app)?;
    Ok(win.ai.cancel_ops() + requests::cancel_queued(&app, win.label()))
}

// ---- chat ----

/// A chat event as the UI receives it: the library's envelope plus the chat it belongs to.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatEventDto {
    pub session_id: String,
    #[serde(flatten)]
    pub envelope: Envelope,
}

#[tauri::command]
pub fn ai_chat_sessions<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> Ai<Vec<SessionInfo>> {
    core(&app)?;
    Ok(win.ai.chats.list())
}

#[tauri::command]
pub fn ai_chat_new<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> Ai<SessionInfo> {
    core(&app)?;
    let id = win.ai.chats.create().map_err(AiCmdError::busy)?;
    Ok(SessionInfo {
        id,
        title: String::new(),
        turns: 0,
        running: false,
    })
}

#[tauri::command]
pub fn ai_chat_get<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String) -> Ai<ChatRecord> {
    core(&app)?;
    win.ai
        .chats
        .get(check_session_id(&session_id)?)
        .ok_or_else(|| AiCmdError::not_found("this chat no longer exists"))
}

#[tauri::command]
pub fn ai_chat_delete<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String) -> Ai<bool> {
    core(&app)?;
    // The chat's browser access, pages and waiting approvals end with it.
    crate::browser::forget_chat(&app, win.label(), &session_id);
    Ok(win.ai.chats.delete(&session_id))
}

/// Streams a chat's events to its window.
pub(crate) fn chat_emitter<R: Runtime>(app: &AppHandle<R>, label: &str) -> Emit {
    let (a, label) = (app.clone(), label.to_string());
    Arc::new(move |sid: &str, env: &Envelope| {
        emit_to_window(
            &a,
            &label,
            CHAT_EVENT,
            ChatEventDto {
                session_id: sid.to_string(),
                envelope: env.clone(),
            },
        )
    })
}

/// Starts a turn as an AI job and returns the job's id at once. The answer streams as `imt://ai-chat`
/// events to this window once the job runs (while it waits, the Jobs pane shows its place in the queue).
#[tauri::command]
pub fn ai_chat_send<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    session_id: String,
    text: String,
    context: Option<ChatContext>,
) -> Ai<u64> {
    core(&app)?;
    let sid = check_session_id(&session_id)?.to_string();
    let emit = chat_emitter(&app, win.label());
    requests::start_turn(&app, win.arc(), &sid, text, context.unwrap_or_default(), emit).map(|id| id.0)
}

pub(crate) type Emit = Arc<dyn Fn(&str, &Envelope) + Send + Sync>;

/// One chat turn (blocking). Every outcome ends with a `done` or `error` event and frees the chat. Returns
/// how it ended, for its job (a failure with its reason, `cancelled` when stopped).
pub(crate) fn run_turn(
    core: &AiCore,
    win: &Arc<WindowState>,
    sid: &str,
    (text, ctx): (&str, &ChatContext),
    turn: super::chats::TurnHandle,
    emit: &Emit,
    browser: Option<&crate::browser::TurnBrowser>,
) -> Ai<()> {
    /// Where a turn failed: before the session ran (tell the chat), or in it (the session streamed it).
    enum TurnError {
        Before(AiCmdError),
        Streamed(AiCmdError),
    }
    let chats = &win.ai.chats;
    let session = turn.session.clone();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| -> Result<(), TurnError> {
        let rounds = browser.map_or(6, |b| b.rounds());
        let assistant = core.assistant_with_rounds(turn.cancel.clone(), rounds).map_err(TurnError::Before)?;
        // Each tool call takes a short read lock; live tail and imports never wait for the model.
        let access = WindowStore(win.clone());
        let browser_env = browser.map(|b| b.env());
        let mut env = ChatEnv::new(&assistant, (&access as &dyn StoreAccess).into());
        if let (Some(b), Some(e)) = (browser, browser_env.as_ref()) {
            env = env.with_browser(e).with_pages(b.pages.clone());
        }
        let mut s = session.lock().unwrap_or_else(|p| p.into_inner());
        s.send(&env, text, ctx, &mut |e| {
            if let (Some(b), ChatEvent::LinkSuggestion { url, .. }) = (browser, &e.event) {
                b.note_suggested(url);
            }
            chats.push_event(sid, e);
            emit(sid, e);
        })
        .map_err(|e| TurnError::Streamed(e.into()))
    }));
    let outcome = match result {
        Ok(Ok(())) => Ok(()),
        Ok(Err(TurnError::Streamed(e))) => Err(e),
        Ok(Err(TurnError::Before(e))) => {
            emit_error(emit, sid, &e.message);
            Err(e)
        }
        Err(_) => {
            let message = "The assistant stopped unexpectedly. Ask again to continue.";
            emit_error(emit, sid, message);
            Err(AiCmdError::new(AiErrorCode::Engine, message))
        }
    };
    let record = session.lock().unwrap_or_else(|p| p.into_inner()).to_record();
    chats.end_turn(sid, record);
    outcome
}

fn emit_error(emit: &Emit, sid: &str, message: &str) {
    let env = Envelope {
        seq: 0,
        event: ChatEvent::Error {
            message: message.to_string(),
            cancelled: false,
        },
    };
    emit(sid, &env);
}

#[tauri::command]
pub fn ai_chat_cancel<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String) -> Ai<bool> {
    core(&app)?;
    // A browser step waiting for approval is denied: nothing runs after Stop.
    crate::browser::deny_waiting(&app, win.label(), &session_id);
    // A turn still waiting in the AI queue is dropped (the chat is told), a running one stops.
    let queued = requests::cancel_queued_turn(&app, win.label(), &session_id);
    Ok(win.ai.chats.cancel(&session_id) || queued)
}

/// Events after `seq` (resume after a reload); null when too old to resume (reload the chat with `ai_chat_get`).
#[tauri::command]
pub fn ai_chat_events_since<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String, seq: u64) -> Ai<Option<Vec<Envelope>>> {
    core(&app)?;
    Ok(win.ai.chats.events_since(&session_id, seq))
}

/// Every chat of this window, as saved in projects and sessions (`ChatHistoryRecord` v1).
#[tauri::command]
pub fn ai_chat_history<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> Ai<ChatHistoryRecord> {
    core(&app)?;
    Ok(win.ai.chats.history())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoreResult {
    pub sessions: Vec<SessionInfo>,
    /// Why saved chats were not loaded (damaged, from a newer IMT).
    pub warning: Option<String>,
}

/// Replaces this window's chats with saved ones (project or session); damaged or newer records give none.
#[tauri::command]
pub fn ai_chat_restore<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, record: Option<Value>) -> Ai<RestoreResult> {
    core(&app)?;
    let (history, warning) = ChatHistoryRecord::from_value(record.as_ref());
    win.ai.chats.restore(history);
    Ok(RestoreResult {
        sessions: win.ai.chats.list(),
        warning,
    })
}

/// A Markdown file the user chose: absolute, `.md`, in an existing folder; never replaced without `overwrite`.
pub(super) fn markdown_target(path: &str, overwrite: bool) -> Ai<PathBuf> {
    if path.is_empty() || path.len() > 4096 || path.contains('\0') {
        return Err(AiCmdError::invalid("choose a file to export to"));
    }
    let p = Path::new(path);
    if !p.is_absolute() {
        return Err(AiCmdError::invalid("the path must be absolute"));
    }
    if !p
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md") || e.eq_ignore_ascii_case("markdown"))
    {
        return Err(AiCmdError::invalid("chats are exported as .md files"));
    }
    let name = p.file_name().ok_or_else(|| AiCmdError::invalid("choose a file name"))?;
    let parent = p
        .parent()
        .and_then(|d| d.canonicalize().ok())
        .ok_or_else(|| AiCmdError::not_found("the folder does not exist"))?;
    let target = parent.join(name);
    if target.is_dir() {
        return Err(AiCmdError::invalid(format!("{} is a folder", target.display())));
    }
    imt_core::overwrite::check_overwrite(&target, overwrite)?;
    Ok(target)
}

#[tauri::command]
pub async fn ai_chat_export_markdown<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    session_id: String,
    path: String,
    overwrite: Option<bool>,
) -> Ai<String> {
    core(&app)?;
    let record = win
        .ai
        .chats
        .get(&session_id)
        .ok_or_else(|| AiCmdError::not_found("this chat no longer exists"))?;
    let target = markdown_target(&path, overwrite.unwrap_or(false))?;
    blocking(move || {
        let tmp = target.with_extension("md.tmp");
        std::fs::write(&tmp, export_markdown(&record))
            .and_then(|_| std::fs::rename(&tmp, &target))
            .map_err(|e| {
                let _ = std::fs::remove_file(&tmp);
                AiCmdError::new(AiErrorCode::Io, format!("cannot write {}: {e}", target.display()))
            })?;
        Ok(target.display().to_string())
    })
    .await
}

/// Starter questions for an empty chat, from the health check of the loaded logs.
#[tauri::command]
pub async fn ai_suggested_prompts<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> Ai<Vec<String>> {
    let core = core(&app)?;
    core.availability()?;
    let w = win.arc().clone();
    blocking(move || Ok(suggested_prompts(&WindowStore(w) as &dyn StoreAccess))).await
}

/// Opens a web source of a chat answer in the system browser. Only https links the chat actually showed
/// (a web citation or a suggested search) are opened; anything else from the webview is refused.
#[tauri::command]
pub fn ai_open_link<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, url: String) -> Ai<()> {
    core(&app)?;
    check_link(&url)?;
    if !win.ai.chats.knows_url(&url) {
        return Err(AiCmdError::new(
            AiErrorCode::Refused,
            "refused: this link was not part of an answer",
        ));
    }
    open::that_detached(&url).map_err(|e| AiCmdError::new(AiErrorCode::Io, format!("cannot open the browser: {e}")))
}

/// An https URL to a public host, of reasonable length.
pub(super) fn check_link(url: &str) -> Ai<()> {
    let refused = || AiCmdError::new(AiErrorCode::Refused, "refused: only https links to public sites are opened");
    if url.len() > 2048 || url.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(refused());
    }
    let rest = url.strip_prefix("https://").ok_or_else(refused)?;
    let authority = rest.split(['/', '?', '#']).next().unwrap_or("");
    if authority.is_empty() || authority.contains('@') {
        return Err(refused());
    }
    let host = match authority.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => authority.split(':').next().unwrap_or(""),
    };
    if host.is_empty() {
        return Err(refused());
    }
    if imt_ai::web::is_private_host(host) || imt_ai::web::is_loopback_host(host) {
        return Err(refused());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_ai::assistant::QueryProposal;

    #[test]
    fn proposals_reach_the_ui_tagged_with_their_kind() {
        let p = Proposal::Query(QueryProposal {
            ql: "level >= error".into(),
            explanation: "Errors.".into(),
            estimated_matches: Some(3),
            repaired: false,
        });
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["kind"], "query");
        assert_eq!(v["estimatedMatches"], 3);
    }
}
