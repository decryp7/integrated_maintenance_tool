//! "AI is browsing" (ADR 0013, amended by ADR 0043). While a chat's agent mode or a research job drives a window's
//! browser, Debrief's status chip says so (with Stop); nothing is ever put into the page. It clears when the AI lets go: Stop, the turn or research ends (finished, cancelled or failed),
//! agent mode is turned off, the chat is deleted, or the page or its Debrief window closes.
//!
//! Who drives is the hub's state ([`BrowserHub::ai_driver`]); [`refresh`] announces a change to the Debrief window (`debrief://browser` `driving` event); `browser_driving` answers the current driver.

use super::hub::{AiDriver, BrowserHub};
use super::{BrowserEvent, BrowserState};
use crate::windows::CurrentWindow;
use crate::CmdResult;
use tauri::{AppHandle, Manager, Runtime};

fn hub<R: Runtime>(app: &AppHandle<R>) -> Option<std::sync::Arc<BrowserHub>> {
    app.try_state::<BrowserState>().map(|s| s.0.clone())
}

/// Announces a change of who drives `owner`'s browser. Call after anything that may start or end the AI's driving;
/// nothing happens when it did not change.
pub fn refresh<R: Runtime>(app: &AppHandle<R>, owner: &str) {
    let Some(hub) = hub(app) else { return };
    let Some(by) = hub.driving_changed(owner) else { return };
    let kind = match &by {
        Some(AiDriver::Agent { .. }) => "agent",
        Some(AiDriver::Research) => "research",
        None => "none",
    };
    log::info!(target: crate::activity::SOURCE_CONTEXT, SourceContext = crate::activity::SOURCE_CONTEXT, Operation = "browser.driving", Window = owner, Driver = kind; "The AI's hold on the browser changed");
    super::emit(app, owner, BrowserEvent::Driving { by });
}

/// Who drives the caller's browser now (the status chip starts from it, then follows the events).
#[tauri::command]
pub async fn browser_driving<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> CmdResult<Option<AiDriver>> {
    Ok(hub(&app).and_then(|h| h.ai_driver(win.label())))
}
