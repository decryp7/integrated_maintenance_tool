//! The internal web browser (ADR 0011): a separate window on the system webview that the user drives from
//! the Web panel of their IMT window, and that a chat may read (read-along) or drive (agent mode) only
//! through `imt_ai::browser`.
//!
//! Isolation: the browser window (`browser-<owner>`) has no Tauri capability, and [`guard`] refuses every
//! command from it before anything else runs, so a page has no IPC at all. Page results reach Rust only as
//! the return values of scripts IMT evaluates (`page::TauriPage`), checked by a one-shot gate (nonce, origin,
//! size). IMT's own UI and its CSP are untouched.
//!
//! Commands: `browser_state`, `browser_open`, `browser_event_query`, `browser_back`, `browser_forward`,
//! `browser_reload`, `browser_stop`, `browser_close`, `browser_open_external`, `browser_capture`,
//! `browser_sources`, `browser_discard_sources`, `browser_chat_status`, `browser_set_read_along`,
//! `browser_set_agent`, `browser_agent_stop`, `browser_agent_continue`, `browser_agent_respond`,
//! `browser_open_suggested`. Events: `imt://browser` (to the owner window and its pop-out panels).
//! Wire contract: `app/src/app/browser/browser.types.ts`.

mod approvals;
pub(crate) mod commands;
mod hub;
mod page;
mod settings_section;
#[cfg(test)]
mod tests;
mod turn;
mod window;

pub use approvals::Approvals;
pub use hub::{BrowserHub, PageState};
pub use turn::TurnBrowser;
pub use window::{destroyed, is_browser_label, owner_destroyed, WEB_PROFILE_DIR};

use imt_ai::browser::{ActionEntry, ActionLog, ChatBrowserStatus, PageCapture};
use serde::Serialize;
use std::sync::Arc;
use tauri::ipc::Invoke;
use tauri::{AppHandle, Manager, Runtime};

pub const BROWSER_EVENT: &str = "imt://browser";
/// Id of the browser's settings section.
pub const SECTION: &str = "browser";

/// What a notice in the Web panel is about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeKind {
    /// A navigation to something that is not a web page was blocked (`message` says why).
    Blocked,
    /// A download was blocked.
    Download,
    /// The page asked for a new window; the user may open `url` in the system browser.
    Popup,
    /// Agent mode tried to load a site the chat may not visit (`message` is the site).
    AgentBlocked,
}

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
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum BrowserEvent {
    State(PageState),
    Notice { kind: NoticeKind, url: String, message: String },
    Sources { session_id: String, sources: Vec<SourceInfo> },
    ChatStatus(ChatStatusDto),
}

/// Sends a browser event to the owner window and its pop-out panels (never to the browser window).
pub fn emit<R: Runtime>(app: &AppHandle<R>, owner: &str, event: BrowserEvent) {
    crate::windows::emit_to_window(app, owner, BROWSER_EVENT, event);
}

/// Wraps the invoke handler: a page in a browser window gets no command at all, whatever its origin. This
/// is in addition to the browser window having no capability (remote origins are refused by Tauri's ACL).
pub fn guard<R: Runtime>(inner: impl Fn(Invoke<R>) -> bool + Send + Sync + 'static) -> impl Fn(Invoke<R>) -> bool + Send + Sync + 'static {
    move |invoke| {
        let label = invoke.message.webview().label().to_string();
        if is_browser_label(&label) || is_browser_label(invoke.message.webview().window().label()) {
            log::warn!(Window = label.as_str(); "A command from the browser window was refused");
            invoke.resolver.reject("IMT commands are not available to web pages");
            return true;
        }
        inner(invoke)
    }
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
    if let Err(e) = crate::settings::register(
        app,
        Arc::new(settings_section::BrowserSection { hub }),
        crate::settings::SectionGroup::App,
    ) {
        log::warn!("Web browser settings not registered: {e:#}");
    }
}

/// The managed state alone (the hub and the approvals), for [`init`] and tests.
pub fn manage_state<R: Runtime>(app: &AppHandle<R>, hub: Arc<BrowserHub>) {
    app.manage(BrowserState(hub));
    app.manage(Arc::new(Approvals::default()));
}

/// The chat is deleted: its browser access, pages, links and waiting approvals end with it.
pub fn forget_chat<R: Runtime>(app: &AppHandle<R>, owner: &str, chat: &str) {
    deny_waiting(app, owner, chat);
    if let Some(hub) = app.try_state::<BrowserState>() {
        hub.forget_chat(owner, chat);
    }
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
