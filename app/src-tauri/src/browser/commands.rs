//! The browser commands. Thin: they validate every argument, resolve the caller's window (the owner of its
//! browser), run page work off the main thread and turn errors into messages.

use super::approvals::{Approvals, DecisionDto};
use super::hub::{BrowserHub, PageState};
use super::page::TauriPage;
use super::view::{self, open, page_webview};
use super::{emit, BrowserEvent, BrowserState, ChatStatusDto, SourceInfo};
use crate::windows::CurrentWindow;
use crate::{err, CmdResult};
use imt_ai::browser::draft::without_flagged;
use imt_ai::browser::{
    check_store_query, query::search_for_event, scripts, small_for_agent, PageReader, QuerySource, SearchEngine, WebUrl,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};

const MAX_INPUT_CHARS: usize = 2048;
const MAX_ID_CHARS: usize = 64;

fn hub<R: Runtime>(app: &AppHandle<R>) -> CmdResult<Arc<BrowserHub>> {
    app.try_state::<BrowserState>()
        .map(|s| s.0.clone())
        .ok_or_else(|| "the browser is not available in this build of IMT".into())
}

fn approvals<R: Runtime>(app: &AppHandle<R>) -> CmdResult<Arc<Approvals>> {
    app.try_state::<Arc<Approvals>>()
        .map(|s| s.inner().clone())
        .ok_or_else(|| "the browser is not available in this build of IMT".into())
}

/// A chat of the calling window (its id as the UI sent it, checked).
fn chat_of(win: &CurrentWindow, session_id: &str) -> CmdResult<String> {
    if session_id.is_empty() || session_id.len() > MAX_ID_CHARS || win.ai.chats.get(session_id).is_none() {
        return Err("this chat no longer exists".into());
    }
    Ok(session_id.to_string())
}

fn bounded(what: &str, s: &str) -> CmdResult<String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(crate::CmdError::from(format!("{what} is empty")));
    }
    if t.chars().count() > MAX_INPUT_CHARS {
        return Err(crate::CmdError::from(format!("{what} is longer than {MAX_INPUT_CHARS} characters")));
    }
    Ok(t.to_string())
}

/// Runs page work on the blocking pool (script replies arrive on the main thread).
async fn blocking<T: Send + 'static>(f: impl FnOnce() -> CmdResult<T> + Send + 'static) -> CmdResult<T> {
    crate::sync::blocking("the browser task", f).await
}

/// The Browser panel's view of its window's browser.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowserStateDto {
    pub page: PageState,
    /// The engine from Settings ▸ Web browser (the panel's picker starts there).
    pub engine: SearchEngine,
    /// A custom search address is set.
    pub custom_engine: bool,
    /// The page shows in the panel (Windows, macOS); false: in a window of its own (Linux, see `view`).
    pub docked: bool,
}

#[tauri::command]
pub async fn browser_state<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> CmdResult<BrowserStateDto> {
    let hub = hub(&app)?;
    let s = hub.settings();
    Ok(BrowserStateDto {
        page: hub.page(win.label()),
        engine: s.search.engine,
        custom_engine: s.search.custom.is_some(),
        docked: view::DOCKED,
    })
}

/// What to open: the address box's text (a page or a search) or a search.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum OpenTarget {
    Address { text: String, engine: Option<SearchEngine> },
    Search { query: String, engine: Option<SearchEngine> },
}

/// Opens the caller's browser (or loads the page in it). Async: a window must not be created from a
/// synchronous command (it deadlocks on Windows).
#[tauri::command]
pub async fn browser_open<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, target: OpenTarget) -> CmdResult<String> {
    let hub = hub(&app)?;
    let search = hub.settings().search;
    let url = match target {
        OpenTarget::Address { text, engine } => search.resolve_address(&bounded("the address", &text)?, engine.unwrap_or(search.engine)),
        OpenTarget::Search { query, engine } => search.search_url(engine.unwrap_or(search.engine), &bounded("the search", &query)?),
    }
    .map_err(err)?;
    open(&app, win.label(), &url, true)?;
    crate::activity::domain_event("browser.open", None, &[("Site", url.site().as_str())]);
    Ok(url.as_str().to_string())
}

/// The search text for an event: drafted by the model (what the leak guard flags in it is taken out), or built by the
/// engine with the privacy guard. The user may edit it before it opens.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EventQueryDto {
    pub query: String,
    /// What was taken out for the user's privacy ("2 file paths", "1 value from your logs").
    pub removed: Vec<String>,
    /// Who wrote the text.
    pub source: QuerySource,
}

/// What the model is asked when it drafts the search for "Search the web for this error".
const SEARCH_QUESTION: &str = "Find web pages that explain this error and how to fix it.";

#[tauri::command]
pub async fn browser_event_query<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, record_id: u32) -> CmdResult<EventQueryDto> {
    let w = win.arc().clone();
    blocking(move || {
        let engine = || {
            let store = w.engine.store()?;
            search_for_event(store.as_ref(), record_id)
                .map(|q| EventQueryDto {
                    query: q.query,
                    removed: q.removed,
                    source: QuerySource::Engine,
                })
                .map_err(|e| crate::CmdError::from(format!("nothing to search for: {e}")))
        };
        let (_, drafted) = super::research::draft_search(&app, &w, Some(SEARCH_QUESTION), Some(record_id))
            .map_err(|e| crate::CmdError::from(format!("nothing to search for: {}", e.message)))?;
        if drafted.source == QuerySource::Engine {
            return engine();
        }
        let access = crate::ai::commands::WindowStore(w.clone());
        let checked = check_store_query((&access as &dyn imt_ai::StoreAccess).into(), &drafted.query).map_err(crate::CmdError::from)?;
        match without_flagged(&checked) {
            (query, _) if query.is_empty() => engine(),
            (query, removed) => Ok(EventQueryDto {
                query,
                removed,
                source: QuerySource::Model,
            }),
        }
    })
    .await
}

/// Back, forward, reload or stop in the caller's browser.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NavCommand {
    Back,
    Forward,
    Reload,
    Stop,
}

#[tauri::command]
pub async fn browser_nav<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, command: NavCommand) -> CmdResult<()> {
    let w = page_webview(&app, win.label()).ok_or("the browser is not open")?;
    let js = match command {
        NavCommand::Back => "history.back()",
        NavCommand::Forward => "history.forward()",
        NavCommand::Reload => scripts::RELOAD,
        NavCommand::Stop => scripts::STOP_LOADING,
    };
    crate::error::lift(w.eval(js).map_err(err))
}

/// Closes the page (the panel stays); false when none was open.
#[tauri::command]
pub async fn browser_close<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> CmdResult<bool> {
    crate::error::lift(view::close(&app, win.label()))
}

/// Where the Browser panel's page area is, from the window the panel is in (the IMT window or its pop-out): the page
/// webview takes that place, or hides (ADR 0024). Validated here; frequent while the user resizes, so quiet.
#[tauri::command]
pub async fn browser_place<R: Runtime>(
    app: AppHandle<R>,
    window: tauri::Window<R>,
    win: CurrentWindow,
    placement: super::Placement,
) -> CmdResult<bool> {
    let placement = placement
        .validated()
        .map_err(|e| crate::CmdError::new(crate::ErrorCode::Invalid, e))?;
    let view = crate::error::lift(view::place(&app, win.label(), window.label(), placement))?;
    Ok(matches!(view, super::geometry::View::At(_)))
}

/// Opens the page shown (or a popup it asked for) in the system browser; any other address is refused.
#[tauri::command]
pub async fn browser_open_external<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, url: String) -> CmdResult<()> {
    let u = WebUrl::parse(&url).map_err(err)?;
    if !hub(&app)?.may_open_externally(win.label(), &url) {
        return Err("refused: only the page shown or a window it asked to open can be opened".into());
    }
    crate::activity::domain_event("browser.openExternal", None, &[("Site", u.site().as_str())]);
    crate::opener::open_url(u.as_str())
}

fn sources_event<R: Runtime>(app: &AppHandle<R>, owner: &str, chat: &str, hub: &BrowserHub) -> Vec<SourceInfo> {
    let sources: Vec<SourceInfo> = hub.attachments(owner, chat).iter().map(SourceInfo::from).collect();
    emit(
        app,
        owner,
        BrowserEvent::Sources {
            session_id: chat.to_string(),
            sources: sources.clone(),
        },
    );
    sources
}

/// "Add page to AI context": captures the visible page (redacted, capped) for the chat's next question.
#[tauri::command]
pub async fn browser_capture<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String) -> CmdResult<Vec<SourceInfo>> {
    let chat = chat_of(&win, &session_id)?;
    let hub = hub(&app)?;
    let owner = win.label().to_string();
    let page = TauriPage {
        app: app.clone(),
        owner: owner.clone(),
        chat: None,
    };
    let captured = blocking(move || crate::error::lift(page.capture().map_err(err))).await?;
    if captured.text.trim().is_empty() {
        return Err("the page has no readable text yet; wait for it to load and try again".into());
    }
    hub.attach(&owner, &chat, captured);
    crate::activity::domain_event("browser.capture", None, &[("Chat", chat.as_str())]);
    Ok(sources_event(&app, &owner, &chat, &hub))
}

#[tauri::command]
pub async fn browser_sources<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String) -> CmdResult<Vec<SourceInfo>> {
    let chat = chat_of(&win, &session_id)?;
    Ok(hub(&app)?.attachments(win.label(), &chat).iter().map(SourceInfo::from).collect())
}

#[tauri::command]
pub async fn browser_discard_sources<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String) -> CmdResult<Vec<SourceInfo>> {
    let chat = chat_of(&win, &session_id)?;
    let hub = hub(&app)?;
    hub.discard_attachments(win.label(), &chat);
    Ok(sources_event(&app, win.label(), &chat, &hub))
}

/// Parameters of the active model in billions, when a model is set up.
fn active_params_b<R: Runtime>(app: &AppHandle<R>) -> Option<f64> {
    let core = crate::ai::commands::core(app).ok()?;
    core.models()
        .into_iter()
        .find(|m| m.active)
        .and_then(|m| m.info)
        .map(|i| i.params_b)
}

pub fn chat_status<R: Runtime>(app: &AppHandle<R>, owner: &str, chat: &str, hub: &BrowserHub) -> ChatStatusDto {
    let params = active_params_b(app);
    ChatStatusDto {
        session_id: chat.to_string(),
        status: hub.chat(owner, chat).status(),
        small_model: small_for_agent(params),
        model_params_b: params,
        research_auto: hub.settings().research.auto,
    }
}

fn status_changed<R: Runtime>(app: &AppHandle<R>, owner: &str, chat: &str, hub: &BrowserHub) -> ChatStatusDto {
    let s = chat_status(app, owner, chat, hub);
    emit(app, owner, BrowserEvent::ChatStatus(s.clone()));
    s
}

#[tauri::command]
pub async fn browser_chat_status<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String) -> CmdResult<ChatStatusDto> {
    let chat = chat_of(&win, &session_id)?;
    let hub = hub(&app)?;
    Ok(chat_status(&app, win.label(), &chat, &hub))
}

/// "Let AI read the browser" for one chat (off by default; ends with the chat).
#[tauri::command]
pub async fn browser_set_read_along<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    session_id: String,
    on: bool,
) -> CmdResult<ChatStatusDto> {
    let chat = chat_of(&win, &session_id)?;
    let hub = hub(&app)?;
    hub.chat(win.label(), &chat).set_read_along(on);
    crate::activity::domain_event(
        "browser.readAlong",
        None,
        &[("Chat", chat.as_str()), ("On", if on { "true" } else { "false" })],
    );
    Ok(status_changed(&app, win.label(), &chat, &hub))
}

/// Agent mode for one chat (off by default; ends with the chat). Turning it on starts a new task.
#[tauri::command]
pub async fn browser_set_agent<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    session_id: String,
    on: bool,
) -> CmdResult<ChatStatusDto> {
    let chat = chat_of(&win, &session_id)?;
    let hub = hub(&app)?;
    let c = hub.chat(win.label(), &chat);
    if on {
        c.set_agent(true, hub.settings().agent);
    } else {
        c.stop();
        approvals(&app)?.deny_chat(win.label(), &chat);
        super::driving::refresh(&app, win.label());
    }
    crate::activity::domain_event(
        "browser.agent.mode",
        None,
        &[("Chat", chat.as_str()), ("On", if on { "true" } else { "false" })],
    );
    Ok(status_changed(&app, win.label(), &chat, &hub))
}

/// Stop: agent mode ends at once, a waiting approval is denied and a page load is stopped. The user has
/// the browser to themselves again.
#[tauri::command]
pub async fn browser_agent_stop<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String) -> CmdResult<ChatStatusDto> {
    let chat = chat_of(&win, &session_id)?;
    let hub = hub(&app)?;
    hub.chat(win.label(), &chat).stop();
    approvals(&app)?.deny_chat(win.label(), &chat);
    if hub.is_driving(win.label(), &chat) {
        if let Some(w) = page_webview(&app, win.label()) {
            let _ = w.eval(scripts::STOP_LOADING);
        }
    }
    super::driving::refresh(&app, win.label());
    crate::activity::domain_event("browser.agent.stop", None, &[("Chat", chat.as_str())]);
    Ok(status_changed(&app, win.label(), &chat, &hub))
}

/// "Continue" after the step cap: another batch of steps for the same task.
#[tauri::command]
pub async fn browser_agent_continue<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String) -> CmdResult<ChatStatusDto> {
    let chat = chat_of(&win, &session_id)?;
    let hub = hub(&app)?;
    let c = hub.chat(win.label(), &chat);
    if !c.agent() {
        return Err("agent mode is off in this chat".into());
    }
    c.continue_task();
    Ok(status_changed(&app, win.label(), &chat, &hub))
}

/// The user's answer on an approval card.
#[tauri::command]
pub async fn browser_agent_respond<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    session_id: String,
    request_id: u64,
    decision: DecisionDto,
) -> CmdResult<bool> {
    let chat = chat_of(&win, &session_id)?;
    Ok(approvals(&app)?.respond(win.label(), &chat, request_id, decision.into()))
}

/// "Open and let AI read it" on a suggested link: opens it, waits for the page, adds it to the chat as an
/// untrusted source and lets the chat go on with it at once (a new turn). Returns that turn's job.
#[tauri::command]
pub async fn browser_open_and_read<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String, url: String) -> CmdResult<u64> {
    let chat = chat_of(&win, &session_id)?;
    let u = WebUrl::parse(&url).map_err(err)?;
    let h = hub(&app)?;
    if !h.was_suggested(win.label(), &chat, u.as_str()) {
        return Err("refused: this link was not suggested in this chat".into());
    }
    let (a, w) = (app.clone(), win.arc().clone());
    crate::error::lift(
        tauri::async_runtime::spawn_blocking(move || {
            let owner = w.label().to_string();
            let before = h.page(&owner).loads;
            open(&a, &owner, &u, true)?;
            h.wait_loaded(&owner, before, std::time::Duration::from_secs(15));
            let page = super::page::TauriPage {
                app: a.clone(),
                owner: owner.clone(),
                chat: None,
            };
            let capture = imt_ai::browser::PageReader::capture(&page).map_err(err)?;
            let title = capture.title.clone();
            h.attach(&owner, &chat, capture);
            crate::activity::domain_event("browser.openAndRead", None, &[("Site", u.site().as_str())]);
            let emit = crate::ai::commands::chat_emitter(&a, &owner);
            let text = format!(
                "I opened \"{}\" for you: read it and go on with my question.",
                title.chars().take(120).collect::<String>()
            );
            crate::ai::requests::start_turn(&a, &w, &chat, text, Default::default(), emit)
                .map(|id| id.0)
                .map_err(|e| e.message)
        })
        .await
        .map_err(err)?,
    )
}

/// Opens a link a chat suggested, in the internal browser. Only links that chat suggested are opened.
#[tauri::command]
pub async fn browser_open_suggested<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, session_id: String, url: String) -> CmdResult<()> {
    let chat = chat_of(&win, &session_id)?;
    let u = WebUrl::parse(&url).map_err(err)?;
    if !hub(&app)?.was_suggested(win.label(), &chat, u.as_str()) {
        return Err("refused: this link was not suggested in this chat".into());
    }
    open(&app, win.label(), &u, true)?;
    crate::activity::domain_event("browser.openSuggested", None, &[("Site", u.site().as_str())]);
    Ok(())
}
