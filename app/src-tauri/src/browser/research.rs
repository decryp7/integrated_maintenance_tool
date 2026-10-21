//! Research jobs (research mode, `imt_ai::browser::research`): the AI searches with the user's engine in the
//! window's visible browser and reads the result pages by itself, read-only, as a job of kind `research` (one at
//! a time, cancellable from the Jobs pane and the Browser panel's Stop).
//!
//! Started three ways: the model's `web_research` tool in a chat turn ([`TurnResearcher`], the turn waits for
//! the job; the model writes the search text itself), "Research this" on an event or the chat's "Research…"
//! ([`ai_research_draft`] drafts the search text with the model and the user sees and edits it, then
//! [`ai_research`] runs it and the answer becomes a turn of the chat), and Retry. The search text is checked by the
//! leak guard first; with workspace data in it the user confirms (or edits) it before anything is searched.

use super::hub::BrowserHub;
use super::page::TauriPage;
use super::ActivityLog;
use crate::ai::error::{AiCmdError, AiErrorCode, AiResult};
use crate::jobs::{self, Controls, Done, JobCtx, JobId, JobKind, JobLink, JobProgress, JobResult, JobSpec};
use crate::windows::{CurrentWindow, WindowState};
use imt_ai::browser::policy::ScrollDirection;
use imt_ai::browser::research::{self, ResearchEnv, ResearchModel, ResearchStep, WithModel};
use imt_ai::browser::{
    draft_or, AgentLimits, BrowserError, ChatBrowser, CheckedQuery, Drafted, Element, PageDriver, QueryDrafter, QuerySource,
    ResearchReport, ResearchRequest, ResearchSettings, Researcher, WebUrl,
};
use imt_ai::{StoreAccess, StoreRef};
use serde::{Deserialize, Serialize};
use serde_json::json;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime};

const MAX_QUESTION_CHARS: usize = 1000;
const TITLE_CHARS: usize = 60;

static SLEEP: fn(Duration) = std::thread::sleep;
static NOW: fn() -> Instant = Instant::now;

fn snippet(s: &str) -> String {
    let one: String = s.split_whitespace().collect::<Vec<_>>().join(" ");
    match one.char_indices().nth(TITLE_CHARS) {
        Some((i, _)) => format!("{}…", &one[..i]),
        None => one,
    }
}

/// Research navigates only to the results page and its result links: every other driver action is refused,
/// and the browser window lets only the address being opened load while research drives it.
pub(super) struct ReadOnlyDriver<R: Runtime> {
    pub(super) page: TauriPage<R>,
    pub(super) control: Arc<ChatBrowser>,
}

impl<R: Runtime> PageDriver for ReadOnlyDriver<R> {
    fn navigate(&self, url: &WebUrl) -> Result<(), BrowserError> {
        self.control.allow_site(url.site());
        self.control.expect_load(url);
        let r = self.page.navigate(url);
        self.control.clear_expected_load();
        r
    }

    /// The engine's own redirect to a result: its site (the engine's, granted by the search) and then the one site it
    /// redirects to, until the page load finished.
    fn navigate_redirect(&self, url: &WebUrl) -> Result<(), BrowserError> {
        self.control.expect_redirect(url);
        let r = self.page.navigate(url);
        self.control.clear_expected_load();
        r
    }

    fn click(&self, _: &Element) -> Result<(), BrowserError> {
        Err(BrowserError::Failed("research is read-only".into()))
    }

    fn type_text(&self, _: &Element, _: &str, _: bool) -> Result<(), BrowserError> {
        Err(BrowserError::Failed("research is read-only".into()))
    }

    fn scroll(&self, _: ScrollDirection) -> Result<(), BrowserError> {
        Err(BrowserError::Failed("research is read-only".into()))
    }

    fn back(&self) -> Result<(), BrowserError> {
        Err(BrowserError::Failed("research is read-only".into()))
    }
}

/// The research settings (Settings ▸ Web browser).
pub fn settings<R: Runtime>(app: &AppHandle<R>) -> ResearchSettings {
    app.try_state::<super::BrowserState>()
        .map(|s| s.0.settings().research)
        .unwrap_or_default()
}

/// The spec of a research job of `owner` (Retry runs it again).
fn spec(owner: &str, req: &ResearchRequest, session: Option<&str>) -> JobSpec {
    let mut spec = JobSpec::new(JobKind::Research, format!("Research: {}", snippet(&req.question)))
        .window(owner)
        .retry_with(
            json!({"op": "research", "question": req.question, "query": req.query, "maxResults": req.max_results, "sessionId": session}),
        );
    if let Some(s) = session {
        spec = spec.link(JobLink::Chat { session_id: s.to_string() });
    }
    spec
}

/// One research on the job's thread: holds the window's browser for the run (no chat may drive it meanwhile).
fn run_research<R: Runtime>(app: &AppHandle<R>, owner: &str, req: &ResearchRequest, ctx: &JobCtx) -> AiResult<ResearchReport> {
    let core = crate::ai::commands::core(app)?;
    let assistant = core.assistant(&crate::ai::knowledge::project_of(app, owner), ctx.cancel_flag())?;
    let hub: Arc<BrowserHub> = app
        .try_state::<super::BrowserState>()
        .map(|s| s.0.clone())
        .ok_or_else(|| AiCmdError::unavailable("the internal browser is not available"))?;
    let key = super::hub::research_key(ctx.id.0);
    let control = hub.chat(owner, &key);
    // Only the addresses research opens may load while it drives (agent rules with nothing else granted).
    control.set_agent(true, AgentLimits::default());
    if !hub.begin_driving(owner, &key) {
        hub.forget_chat(owner, &key);
        return Err(AiCmdError::busy("a chat is driving the browser now; stop its agent mode first"));
    }
    super::driving::refresh(app, owner);
    let page = |chat: Option<String>| TauriPage {
        app: app.clone(),
        owner: owner.to_string(),
        chat,
    };
    let reader = page(None);
    let driver = ReadOnlyDriver {
        page: page(Some(key.clone())),
        control: control.clone(),
    };
    let log = ActivityLog { window: owner.to_string() };
    let cancel = ctx.cancel_flag();
    let env = ResearchEnv {
        reader: &reader,
        results: &reader,
        driver: &driver,
        search: hub.settings().search,
        log: &log,
        id: key.clone(),
        cancel: &cancel,
        pace: research::PAGE_PACE,
        total: research::TOTAL_TIME,
        sleep: &SLEEP,
        now: &NOW,
    };
    let model = WithModel(&assistant);
    let r = research::research(&env, &model as &dyn ResearchModel, req, &mut |step| {
        ctx.message(&step.describe());
        if let ResearchStep::Reading { page, of, .. } = step {
            ctx.progress(JobProgress::counts(*page as u64, *of as u64));
        }
    });
    hub.end_driving(owner, &key);
    hub.forget_chat(owner, &key);
    // Finished, failed or cancelled: the browser is the user's again.
    super::driving::refresh(app, owner);
    r.map_err(AiCmdError::from)
}

/// A chat turn's `web_research`: runs the research as a job and waits for it (the turn shows the tool running;
/// the Jobs pane shows the research with its pages).
pub struct TurnResearcher<R: Runtime> {
    pub app: AppHandle<R>,
    pub owner: String,
    pub chat: String,
}

impl<R: Runtime> Researcher for TurnResearcher<R> {
    fn settings(&self) -> ResearchSettings {
        settings(&self.app)
    }

    fn research(&self, question: &str, query: &str, max_results: u8) -> imt_ai::Result<ResearchReport> {
        let req = ResearchRequest {
            question: question.to_string(),
            query: query.to_string(),
            max_results,
        };
        let jobs = self
            .app
            .try_state::<jobs::Jobs>()
            .ok_or_else(|| imt_ai::AiError::Engine("IMT is not ready yet".into()))?;
        let (app, owner) = (self.app.clone(), self.owner.clone());
        let r2 = req.clone();
        let handle = jobs
            .submit(
                spec(&self.owner, &req, Some(&self.chat)),
                Controls::default(),
                move |ctx: &JobCtx| -> AiResult<Done<ResearchReport>> {
                    let report = run_research(&app, &owner, &r2, ctx)?;
                    Ok(Done::new(report))
                },
            )
            .map_err(|e| imt_ai::AiError::Engine(e.to_string()))?;
        match handle.wait() {
            Ok(Ok(report)) => Ok(report),
            Ok(Err(e)) if e.code == AiErrorCode::Cancelled => Err(imt_ai::AiError::Cancelled),
            Ok(Err(e)) if e.code == AiErrorCode::Browser => Err(imt_ai::AiError::Browser(e.message)),
            Ok(Err(e)) => Err(imt_ai::AiError::Engine(e.message)),
            Err(jobs::Stopped::Cancelled | jobs::Stopped::Interrupted(_)) => Err(imt_ai::AiError::Cancelled),
            Err(e) => Err(imt_ai::AiError::Engine(e.to_string())),
        }
    }
}

/// The answer of a research started from a button: a turn of the chat (a new chat when it is busy). When nothing was
/// found, the results page comes with it as a link card the user may open (noted as suggested in that chat).
fn record_in_chat<R: Runtime>(
    app: &AppHandle<R>,
    win: &Arc<WindowState>,
    emit: &crate::ai::commands::Emit,
    session: Option<&str>,
    question: &str,
    report: &ResearchReport,
) -> AiResult<String> {
    let chats = &win.ai.chats;
    let sid = match session {
        Some(s) if chats.get(s).is_some() => s.to_string(),
        _ => chats.create().map_err(AiCmdError::busy)?,
    };
    let (sid, turn) = match chats.begin_turn(&sid) {
        Ok(t) => (sid, t),
        Err(_) => {
            let fresh = chats.create().map_err(AiCmdError::busy)?;
            let t = chats.begin_turn(&fresh).map_err(AiCmdError::busy)?;
            (fresh, t)
        }
    };
    let mut s = crate::sync::lock(&turn.session);
    let cards: Vec<_> = report.results_page_card().into_iter().collect();
    if let (Some(url), Some(hub)) = (&report.results_page, app.try_state::<super::BrowserState>()) {
        hub.note_suggested(win.label(), &sid, url);
    }
    s.record_answer_with(
        &format!("Research: {question}"),
        &report.answer,
        report.sources(),
        cards,
        &mut |e| {
            chats.push_event(&sid, e);
            emit(&sid, e);
        },
    );
    let record = s.to_record();
    drop(s);
    chats.end_turn(&sid, record);
    Ok(sid)
}

/// Starts a research job of `win` that records its answer in a chat; returns the job.
pub(crate) fn start<R: Runtime>(
    app: &AppHandle<R>,
    win: &Arc<WindowState>,
    req: ResearchRequest,
    session: Option<String>,
) -> AiResult<JobId> {
    let spec = spec(win.label(), &req, session.as_deref());
    let (a, w) = (app.clone(), win.clone());
    let emit = crate::ai::commands::chat_emitter(app, win.label());
    // Cancel (Jobs are the one cancel authority) also clears the window's "AI is browsing" banner at once;
    // the job ends when it next looks at its cancel flag.
    let (h, owner) = (app.clone(), win.label().to_string());
    let controls = Controls::cancel(move || {
        if let Some(hub) = h.try_state::<super::BrowserState>() {
            hub.stop_research(&owner);
        }
        super::driving::refresh(&h, &owner);
    });
    jobs::spawn(app, spec, controls, move |ctx: &JobCtx| -> AiResult<Done<()>> {
        let report = run_research(&a, w.label(), &req, ctx)?;
        let sid = record_in_chat(&a, &w, &emit, session.as_deref(), &req.question, &report)?;
        Ok(Done::new(()).with_result(JobResult::Chat { session_id: sid }))
    })
    .map_err(AiCmdError::busy)
}

/// What "Research this" answers: started (a job), or the search text needs the user's confirmation.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase", tag = "status")]
pub enum ResearchStarted {
    #[serde(rename_all = "camelCase")]
    Started { job_id: u64 },
    /// Workspace data is in the search text: confirm (or edit) it, then call again with `confirmed`.
    Confirm { checked: CheckedQuery, question: String },
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchArgs {
    /// What to find out; defaults to the event's search text.
    pub question: Option<String>,
    /// Research this event (its redacted search text).
    pub record_id: Option<u32>,
    /// The chat the answer goes to (a new one when absent).
    pub session_id: Option<String>,
    /// Results to read (clamped to 1–10; the setting when absent).
    pub max_results: Option<i64>,
    /// The search text as the user confirmed or edited it.
    pub query: Option<String>,
    #[serde(default)]
    pub confirmed: bool,
}

fn bounded(what: &str, s: &str, max: usize) -> AiResult<String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(AiCmdError::invalid(format!("{what} is empty")));
    }
    if t.chars().count() > max {
        return Err(AiCmdError::invalid(format!("{what} is longer than {max} characters")));
    }
    Ok(t.to_string())
}

/// The research request of `args` for `win` (validated; the query checked by the leak guard).
fn prepare<R: Runtime>(
    app: &AppHandle<R>,
    win: &Arc<WindowState>,
    args: &ResearchArgs,
) -> AiResult<Result<ResearchRequest, ResearchStarted>> {
    let access = crate::ai::commands::WindowStore(win.clone());
    let store: StoreRef<'_> = (&access as &dyn StoreAccess).into();
    let event_query = match args.record_id {
        Some(id) if args.query.is_none() => Some(
            store
                .with(|s| imt_ai::browser::query::search_for_event(s, id))
                .map_err(AiCmdError::not_found)?
                .query,
        ),
        _ => None,
    };
    let question = match (&args.question, &event_query) {
        (Some(q), _) => bounded("the question", q, MAX_QUESTION_CHARS)?,
        (None, Some(q)) => format!("What does this error mean and how can it be fixed: {q}"),
        (None, None) => return Err(AiCmdError::invalid("give the question to research")),
    };
    let raw = args.query.clone().or(event_query).unwrap_or_else(|| question.clone());
    let checked = imt_ai::browser::check_store_query(store, &raw).map_err(AiCmdError::invalid)?;
    if checked.needs_confirm && !args.confirmed {
        return Ok(Err(ResearchStarted::Confirm { checked, question }));
    }
    let max = args
        .max_results
        .map_or(settings(app).max_results, |n| imt_ai::browser::clamp_results(Some(n)));
    Ok(Ok(ResearchRequest {
        question,
        query: checked.query,
        max_results: max,
    }))
}

/// How long the model may take to draft a search text before the engine's text is used.
const DRAFT_TIMEOUT: Duration = Duration::from_secs(45);

/// The question to research and its search text: drafted by the model from the question and the event (the engine's
/// text when there is no model or its draft is unusable). Not yet checked by the leak guard. Runs the model: call off
/// the main thread.
pub(crate) fn draft_search<R: Runtime>(
    app: &AppHandle<R>,
    win: &Arc<WindowState>,
    question: Option<&str>,
    record_id: Option<u32>,
) -> AiResult<(String, Drafted)> {
    let access = crate::ai::commands::WindowStore(win.clone());
    let store: StoreRef<'_> = (&access as &dyn StoreAccess).into();
    let event = match record_id {
        Some(id) => Some(
            store
                .with(|s| imt_ai::browser::query::event_text_of(s, id))
                .ok_or_else(|| AiCmdError::not_found(format!("there is no event #{id} in the loaded logs")))?,
        ),
        None => None,
    };
    let engine = record_id.and_then(|id| {
        store
            .with(|s| imt_ai::browser::query::search_for_event(s, id))
            .ok()
            .map(|q| q.query)
    });
    let question = match (question, &engine) {
        (Some(q), _) => bounded("the question", q, MAX_QUESTION_CHARS)?,
        (None, Some(q)) => format!("What does this error mean and how can it be fixed: {q}"),
        (None, None) if event.is_some() => "What does this error mean and how can it be fixed?".to_string(),
        (None, None) => return Err(AiCmdError::invalid("give the question to research")),
    };
    // Without an event, the search text used to be the question itself: it still is when the model cannot draft.
    let fallback = engine.or_else(|| record_id.is_none().then(|| question.clone()));
    let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
    let assistant = crate::ai::commands::core(app)
        .ok()
        .and_then(|c| c.assistant(&win.ai.knowledge.snapshot(), cancel.clone()).ok());
    let model = assistant.as_ref().map(WithModel);
    let timer = cancel.clone();
    std::thread::spawn(move || {
        SLEEP(DRAFT_TIMEOUT);
        timer.store(true, std::sync::atomic::Ordering::Relaxed);
    });
    let drafted = draft_or(model.as_ref().map(|m| m as &dyn QueryDrafter), &question, event.as_ref(), || {
        fallback
    })
    .ok_or_else(|| AiCmdError::not_found("there is nothing to search for in this event"))?;
    Ok((question, drafted))
}

/// What "Research this" shows before anything is searched: the question and the drafted search text with what the
/// leak guard found in it. The user edits it, then `ai_research` runs it.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ResearchDraft {
    pub question: String,
    pub checked: CheckedQuery,
    /// Who wrote the search text.
    pub source: QuerySource,
}

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftArgs {
    /// What to find out; defaults to one about the event.
    pub question: Option<String>,
    /// The event to research.
    pub record_id: Option<u32>,
}

/// "Research this": the model drafts the search text from the question and the event (fallback: the engine's text),
/// the leak guard checks it, and the user sees and edits it before the research runs.
#[tauri::command]
pub async fn ai_research_draft<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, args: DraftArgs) -> AiResult<ResearchDraft> {
    if let Some(q) = &args.question {
        bounded("the question", q, MAX_QUESTION_CHARS)?;
    }
    let w = win.arc().clone();
    tauri::async_runtime::spawn_blocking(move || {
        let (question, drafted) = draft_search(&app, &w, args.question.as_deref(), args.record_id)?;
        let access = crate::ai::commands::WindowStore(w.clone());
        let checked =
            imt_ai::browser::check_store_query((&access as &dyn StoreAccess).into(), &drafted.query).map_err(AiCmdError::invalid)?;
        Ok(ResearchDraft {
            question,
            checked,
            source: drafted.source,
        })
    })
    .await
    .map_err(|e| AiCmdError::new(AiErrorCode::Engine, format!("the search text was not drafted ({e})")))?
}

/// "Research this" (an event, the inspector) and the chat's "Research…": starts a research job whose answer
/// becomes a turn of the chat, or asks to confirm a search text with workspace data in it.
#[tauri::command]
pub async fn ai_research<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, args: ResearchArgs) -> AiResult<ResearchStarted> {
    let core = crate::ai::commands::core(&app)?;
    core.provider()?;
    if let Some(s) = &args.session_id {
        if s.is_empty() || s.len() > 64 || win.ai.chats.get(s).is_none() {
            return Err(AiCmdError::not_found("this chat no longer exists"));
        }
    }
    let w = win.arc().clone();
    let a = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let req = match prepare(&a, &w, &args)? {
            Ok(r) => r,
            Err(confirm) => return Ok(confirm),
        };
        start(&a, &w, req, args.session_id.clone()).map(|id| ResearchStarted::Started { job_id: id.0 })
    })
    .await
    .map_err(|e| AiCmdError::new(AiErrorCode::Engine, format!("the research did not start ({e})")))?
}

/// Retry of a research job.
pub(crate) fn retry<R: Runtime>(app: &AppHandle<R>, job: &jobs::model::Job) -> Result<Option<JobId>, String> {
    let r = job.request.clone().unwrap_or_default();
    let label = job.window().ok_or("this research has no window")?;
    let win = app
        .try_state::<crate::windows::Windows>()
        .and_then(|w| w.get(label))
        .ok_or("the window of this research is closed")?;
    let question = r["question"].as_str().ok_or("this research cannot be run again")?;
    let query = r["query"].as_str().unwrap_or(question);
    let args = ResearchArgs {
        question: Some(question.to_string()),
        query: Some(query.to_string()),
        max_results: r["maxResults"].as_i64(),
        session_id: r["sessionId"].as_str().map(str::to_string),
        // The user confirmed this search text when it first ran.
        confirmed: true,
        record_id: None,
    };
    let req = match prepare(app, &win, &args).map_err(|e| e.message)? {
        Ok(req) => req,
        Err(_) => return Err("the search text needs confirming; start the research again".into()),
    };
    start(app, &win, req, args.session_id).map(Some).map_err(|e| e.message)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn specs_name_the_question_and_keep_the_request_for_retry() {
        let req = ResearchRequest {
            question: "what does HRESULT 0x80070005 mean for IIS when the app pool starts after a deploy?".into(),
            query: "HRESULT 0x80070005 IIS".into(),
            max_results: 3,
        };
        let s = spec("main", &req, Some("c-1"));
        assert_eq!(s.kind, JobKind::Research);
        assert!(s.title.starts_with("Research: what does HRESULT") && s.title.ends_with('…'));
        assert_eq!(s.source.link, Some(JobLink::Chat { session_id: "c-1".into() }));
        let r = s.request.unwrap();
        assert_eq!(
            (r["maxResults"].as_u64(), r["query"].as_str()),
            (Some(3), Some("HRESULT 0x80070005 IIS"))
        );
        assert!(spec("main", &req, None).source.link.is_none());
        assert!(bounded("q", " ", 5).is_err() && bounded("q", "toolong", 3).is_err());
    }
}
