//! The AI's browser (ADR 0043, replacing the docked panel of ADR 0024): research and the chat's Read along and Agent
//! modes drive a real Chromium-based browser over the DevTools protocol ([`renderer`], built on `chromium_driver`).
//! There is no browser panel and no child web view in Debrief any more; links open in the user's default browser.
//! A chat reads or drives the page only through `debrief_ai::browser`.
//!
//! Indicators that the AI drives a page are all outside the page: Chrome's automation banner, the window frame colour,
//! the status chip in Debrief (activity, host, Stop) and the research log. Nothing is injected into a page.
//!
//! Commands: `browser_open`, `browser_event_query`, `browser_capture`, `browser_sources`, `browser_discard_sources`,
//! `browser_chat_status`, `browser_set_read_along`, `browser_set_agent`, `browser_agent_stop`, `browser_agent_continue`,
//! `browser_agent_respond`, `browser_open_suggested`, `browser_open_and_read`, `browser_stop`, `research_browser_close`, `browser_runtime_status`,
//! `browser_runtime_install`, `browser_runtime_remove`, `ai_research_draft`, `ai_research`, `browser_driving`.
//! Events: `debrief://browser` (to the window). Wire contract: `app/src/app/browser/browser.types.ts`.

mod approvals;
pub(crate) mod commands;
pub(crate) mod driving;
mod hub;
pub(crate) mod lifecycle;
#[cfg(test)]
mod live;
pub(crate) mod renderer;
pub(crate) mod research;
pub(crate) mod runtime;
mod settings_section;
#[cfg(test)]
mod tests;
mod turn;

pub use approvals::Approvals;
pub use hub::{AiDriver, BrowserHub};
pub use turn::TurnBrowser;

use debrief_ai::browser::{ActionEntry, ActionLog, ChatBrowserStatus, PageCapture};
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};

pub const BROWSER_EVENT: &str = "debrief://browser";
/// Id of the browser's settings section.
pub const SECTION: &str = "browser";

/// A page handed to a chat, as the UI lists it (never its text).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceInfo {
    pub title: String,
    pub url: String,
    pub chars: usize,
    pub truncated: bool,
    pub removed: Vec<String>,
}

impl From<&PageCapture> for SourceInfo {
    fn from(p: &PageCapture) -> Self {
        SourceInfo {
            title: p.title.clone(),
            url: p.url.clone(),
            chars: p.text.chars().count(),
            truncated: p.truncated,
            removed: p.removed.clone(),
        }
    }
}

/// A chat's browser access, as the UI shows it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatStatusDto {
    pub session_id: String,
    #[serde(flatten)]
    pub status: ChatBrowserStatus,
    /// The model may be too small for multi-step browsing (a hint, never a block).
    pub small_model: bool,
    /// Parameters of the active model in billions, when known.
    pub model_params_b: Option<f64>,
    /// The model researches the web by itself (Settings ▸ Web browser); off: it offers a button.
    pub research_auto: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum BrowserEvent {
    Sources {
        session_id: String,
        sources: Vec<SourceInfo>,
    },
    ChatStatus(ChatStatusDto),
    /// The AI started or stopped driving the browser (`by`: who; `None`: it let go).
    Driving {
        by: Option<AiDriver>,
    },
    /// Research ended but its browser window stays open because the user took part in it; the window offers to close it
    /// (`research_browser_close`).
    ResearchBrowserKept,
    /// The window's AI browser moved in its life cycle (launching, ready, closing, gone and why); `kept` while research
    /// left it open. A browser the user closed is `gone` (`windowClosed`), and the next research starts a new one.
    Lifecycle(lifecycle::Lifecycle),
    /// A line of what the AI is doing ("Reading 3 of 5 results"), for the status chip.
    Status {
        text: String,
    },
    /// What the AI's browser is doing, for the status chip (`title` is the window title it would carry).
    Activity {
        /// `reading`, `searching` or `waitingForYou`.
        activity: &'static str,
        host: String,
        title: String,
    },
}

/// Sends a browser event to the owner window.
pub fn emit<R: Runtime>(app: &AppHandle<R>, owner: &str, event: BrowserEvent) {
    crate::windows::emit_to_window(app, owner, BROWSER_EVENT, event);
}

/// The renderers as managed state.
pub fn renderers<R: Runtime>(app: &AppHandle<R>) -> Option<Arc<renderer::Renderers>> {
    app.try_state::<Arc<renderer::Renderers>>().map(|s| s.inner().clone())
}

fn product<R: Runtime>(app: &AppHandle<R>) -> String {
    app.config().product_name.clone().unwrap_or_else(|| "Debrief".into())
}

/// A page of the window's renderer for the chat ports (the browser window is always a normal one on screen).
pub(crate) fn page_for<R: Runtime>(
    app: &AppHandle<R>,
    owner: &str,
    may_drive: Option<Box<dyn Fn() -> bool + Send + Sync>>,
    rules: Option<Arc<debrief_ai::browser::ChatBrowser>>,
) -> Option<renderer::ChromiumPage> {
    let (a, o) = (app.clone(), owner.to_string());
    Some(renderer::ChromiumPage {
        renderers: renderers(app)?,
        owner: owner.to_string(),
        may_drive,
        rules,
        product: product(app),
        notify: Arc::new(move |cue| {
            use chromium_driver::cue::Activity;
            let activity = match cue.activity {
                Activity::Reading => "reading",
                Activity::Searching => "searching",
                Activity::WaitingForYou => "waitingForYou",
            };
            emit(
                &a,
                &o,
                BrowserEvent::Activity {
                    activity,
                    host: cue.host,
                    title: cue.title,
                },
            );
        }),
        lease: renderer::Lease::default(),
    })
}

/// The window's running renderer, read only (nothing is started).
pub(crate) fn reader_page<R: Runtime>(app: &AppHandle<R>, owner: &str) -> Option<renderer::ChromiumPage> {
    renderers(app)?.current(owner)?;
    page_for(app, owner, None, None)
}

/// A page that starts the renderer on screen when none runs (the user asked to open something).
pub(crate) fn visible_page<R: Runtime>(app: &AppHandle<R>, owner: &str) -> Option<renderer::ChromiumPage> {
    page_for(app, owner, Some(Box::new(|| true)), None)
}

/// Writes every agent action to the activity log (diagnostics reports show what the AI did). Never page
/// text or typed text: the action, the site, the outcome and the step.
pub struct ActivityLog {
    pub window: String,
}

impl ActionLog for ActivityLog {
    fn record(&self, e: &ActionEntry) {
        let step = e.step.to_string();
        let site = e.site.clone().unwrap_or_default();
        crate::activity::domain_event(
            "browser.agent",
            None,
            &[
                ("Window", self.window.as_str()),
                ("Chat", e.chat.as_str()),
                ("Action", e.action),
                ("Site", site.as_str()),
                ("Outcome", e.outcome.as_str()),
                ("Step", step.as_str()),
            ],
        );
    }
}

/// Manages the hub and the approvals and registers Settings ▸ Web browser. Call from `setup` after
/// `settings::init`.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let hub = Arc::new(BrowserHub::default());
    manage_state(app, hub.clone());
    let renderers = renderers(app).unwrap_or_default();
    let a = app.clone();
    renderers.listen(Arc::new(move |owner, life| emit(&a, owner, BrowserEvent::Lifecycle(life))));
    runtime::refresh_sources(app, &hub);
    if let Err(e) = crate::settings::register(
        app,
        Arc::new(settings_section::BrowserSection {
            hub,
            renderers: Some(renderers),
        }),
        crate::settings::SectionGroup::App,
    ) {
        log::warn!("Web browser settings not registered: {e:#}");
    }
}

/// The managed state alone (the hub and the approvals), for [`init`] and tests.
pub fn manage_state<R: Runtime>(app: &AppHandle<R>, hub: Arc<BrowserHub>) {
    app.manage(BrowserState(hub));
    app.manage(Arc::new(renderer::Renderers::default()));
    app.manage(Arc::new(Approvals::default()));
}

/// A window is gone: its browser process ends and its holds are dropped.
pub fn owner_destroyed<R: Runtime>(app: &AppHandle<R>, owner: &str) {
    if let Some(r) = renderers(app) {
        r.kill(owner);
    }
    if let Some(hub) = app.try_state::<BrowserState>() {
        hub.forget_owner(owner);
    }
}

/// Debrief exits: every browser is closed (gracefully, then by force) and its profile removed, even one kept open for the user.
pub fn shutdown<R: Runtime>(app: &AppHandle<R>) {
    if let Some(r) = renderers(app) {
        r.close_all(renderer::QUIT_GRACE);
    }
}

/// The chat is deleted: its browser access, pages, links and waiting approvals end with it.
pub fn forget_chat<R: Runtime>(app: &AppHandle<R>, owner: &str, chat: &str) {
    deny_waiting(app, owner, chat);
    if let Some(hub) = app.try_state::<BrowserState>() {
        hub.forget_chat(owner, chat);
    }
    driving::refresh(app, owner);
}

/// Denies every approval a chat waits for (its turn was cancelled).
pub fn deny_waiting<R: Runtime>(app: &AppHandle<R>, owner: &str, chat: &str) {
    if let Some(a) = app.try_state::<Arc<Approvals>>() {
        a.deny_chat(owner, chat);
    }
}

/// The hub as managed state.
pub struct BrowserState(pub Arc<BrowserHub>);

impl std::ops::Deref for BrowserState {
    type Target = BrowserHub;
    fn deref(&self) -> &BrowserHub {
        &self.0
    }
}
