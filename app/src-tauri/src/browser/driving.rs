//! "AI is browsing" on the browser window itself (ADR 0013). While a chat's agent mode or a research job
//! drives a window's browser, the browser window says so: its title starts with "AI browsing — ", a banner of
//! IMT's own UI with "AI is browsing — Stop" covers the top of the window (a small window owned by the browser
//! window and kept over its top edge, never anything injected into the page), and on Windows 11 its frame
//! turns amber. Everything clears when the AI lets go: Stop, the turn or research ends (finished, cancelled or
//! failed), agent mode is turned off, the chat is deleted, or the browser or its IMT window closes.
//!
//! Who drives is the hub's state ([`BrowserHub::ai_driver`]); [`refresh`] announces a change to the IMT
//! window (`imt://browser` `driving` event, also to the banner) and applies it to the browser window.

use super::hub::{AiDriver, BrowserHub};
use super::window::browser_label;
use super::{BrowserEvent, BrowserState, BROWSER_EVENT};
use crate::windows::CurrentWindow;
use crate::CmdResult;
use std::sync::Mutex;
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, Runtime, WebviewUrl, WebviewWindow, WebviewWindowBuilder};

/// Labels of banner windows: `aib-<owner window>` (never `browser-…`: the banner is IMT's UI and has commands).
pub const BANNER_PREFIX: &str = "aib-";
/// Height of the banner in logical pixels.
pub const BANNER_HEIGHT: f64 = 40.0;
/// The browser window's title without a page.
pub const TITLE: &str = "IMT browser";
/// What the browser window's title starts with while the AI drives.
pub const DRIVING_PREFIX: &str = "AI browsing \u{2014} ";
const MAX_TITLE: usize = 200;
/// The frame colour while the AI drives (Windows 11), as a COLORREF (0x00BBGGRR): amber #F5B800.
#[cfg(windows)]
const FRAME_COLOR: u32 = 0x0000_B8F5;
/// DWMWA_COLOR_DEFAULT: the system's frame colour again.
#[cfg(windows)]
const FRAME_DEFAULT: u32 = 0xFFFF_FFFF;

/// One window change at a time: two quick changes must not build the banner twice.
static APPLYING: Mutex<()> = Mutex::new(());

pub fn banner_label(owner: &str) -> String {
    format!("{BANNER_PREFIX}{owner}")
}

/// The IMT window a banner belongs to.
pub fn banner_owner(label: &str) -> Option<&str> {
    label.strip_prefix(BANNER_PREFIX).filter(|o| !o.is_empty())
}

pub fn is_banner_label(label: &str) -> bool {
    banner_owner(label).is_some()
}

/// The URL the banner loads: the app, routed to the banner (`app/src/app/browser/ai-browsing-banner.ts`).
pub fn banner_url(owner: &str) -> String {
    format!("index.html#/ai-browsing?host={owner}")
}

/// The browser window's title for the page's title: "<page> — IMT browser" (one line, capped), with
/// "AI browsing — " in front while the AI drives.
pub fn window_title(page_title: &str, driving: bool) -> String {
    let page: String = page_title.chars().filter(|c| !c.is_control()).take(MAX_TITLE).collect();
    let base = if page.trim().is_empty() {
        TITLE.to_string()
    } else {
        format!("{} \u{2014} {TITLE}", page.trim())
    };
    if driving {
        format!("{DRIVING_PREFIX}{base}")
    } else {
        base
    }
}

/// Where the banner goes, in physical pixels: across the top of the browser's client area (`inner_*`).
pub fn banner_rect(inner_x: i32, inner_y: i32, inner_width: u32, scale: f64) -> (i32, i32, u32, u32) {
    let scale = if scale.is_finite() && scale > 0.0 { scale } else { 1.0 };
    let height = (BANNER_HEIGHT * scale).round().max(1.0) as u32;
    (inner_x, inner_y, inner_width.max(1), height)
}

fn hub<R: Runtime>(app: &AppHandle<R>) -> Option<std::sync::Arc<BrowserHub>> {
    app.try_state::<BrowserState>().map(|s| s.0.clone())
}

/// Announces a change of who drives `owner`'s browser and shows it on the browser window. Call after anything
/// that may start or end the AI's driving; nothing happens when it did not change.
pub fn refresh<R: Runtime>(app: &AppHandle<R>, owner: &str) {
    let Some(hub) = hub(app) else { return };
    let Some(by) = hub.driving_changed(owner) else { return };
    let kind = match &by {
        Some(AiDriver::Agent { .. }) => "agent",
        Some(AiDriver::Research) => "research",
        None => "none",
    };
    log::info!(target: crate::activity::SOURCE_CONTEXT, SourceContext = crate::activity::SOURCE_CONTEXT, Operation = "browser.driving", Window = owner, Driver = kind; "The AI's hold on the browser changed");
    super::emit(app, owner, BrowserEvent::Driving { by: by.clone() });
    let _ = app.emit_to(banner_label(owner).as_str(), BROWSER_EVENT, BrowserEvent::Driving { by });
    apply_later(app, owner, hub);
}

/// Shows the current driver on a browser window that just opened (the agent may be what opened it).
pub fn reapply<R: Runtime>(app: &AppHandle<R>, owner: &str) {
    if let Some(hub) = hub(app) {
        if hub.ai_driver(owner).is_some() {
            apply_later(app, owner, hub);
        }
    }
}

/// Window changes run off the caller's thread: a window must not be built from a synchronous command or an
/// event handler (it deadlocks on Windows), and the caller may be a turn holding its locks.
fn apply_later<R: Runtime>(app: &AppHandle<R>, owner: &str, hub: std::sync::Arc<BrowserHub>) {
    let (a, o) = (app.clone(), owner.to_string());
    tauri::async_runtime::spawn_blocking(move || apply(&a, &o, &hub));
}

/// Makes the browser window show the current driver (the latest state wins, whatever order changes come in).
fn apply<R: Runtime>(app: &AppHandle<R>, owner: &str, hub: &BrowserHub) {
    let _one = APPLYING.lock().unwrap_or_else(|p| p.into_inner());
    let driving = hub.ai_driver(owner).is_some();
    let Some(browser) = app.get_webview_window(&browser_label(owner)) else {
        close_banner(app, owner);
        return;
    };
    let _ = browser.set_title(&window_title(&hub.page(owner).title, driving));
    set_frame(&browser, driving);
    match (driving, app.get_webview_window(&banner_label(owner))) {
        (true, Some(banner)) => {
            place(&browser, &banner);
            let _ = banner.show();
        }
        (true, None) => match build_banner(app, owner, &browser) {
            Ok(banner) => {
                place(&browser, &banner);
                let _ = banner.show();
            }
            Err(e) => log::warn!(Window = owner; "The AI-is-browsing banner could not be shown: {e}"),
        },
        (false, Some(banner)) => {
            let _ = banner.hide();
        }
        (false, None) => {}
    }
}

fn build_banner<R: Runtime>(app: &AppHandle<R>, owner: &str, browser: &WebviewWindow<R>) -> tauri::Result<WebviewWindow<R>> {
    // Owned by the browser window: above it (not above other apps), hidden when it is minimised, destroyed with
    // it (Windows); a child window that moves with it (macOS); transient for it (Linux).
    WebviewWindowBuilder::new(app, banner_label(owner), WebviewUrl::App(banner_url(owner).into()))
        .title("AI is browsing")
        .parent(browser)?
        .decorations(false)
        .shadow(false)
        .resizable(false)
        .minimizable(false)
        .maximizable(false)
        .closable(false)
        .skip_taskbar(true)
        .focused(false)
        .visible(false)
        .inner_size(BANNER_HEIGHT * 10.0, BANNER_HEIGHT)
        .build()
}

/// Puts the banner over the top of the browser's client area.
fn place<R: Runtime>(browser: &WebviewWindow<R>, banner: &WebviewWindow<R>) {
    let (Ok(pos), Ok(size), Ok(scale)) = (browser.inner_position(), browser.inner_size(), browser.scale_factor()) else {
        return;
    };
    let (x, y, w, h) = banner_rect(pos.x, pos.y, size.width, scale);
    let _ = banner.set_size(PhysicalSize::new(w, h));
    let _ = banner.set_position(PhysicalPosition::new(x, y));
}

/// The browser window moved or was resized: the banner follows (when it is shown).
pub fn follow<R: Runtime>(app: &AppHandle<R>, owner: &str) {
    let (Some(browser), Some(banner)) = (
        app.get_webview_window(&browser_label(owner)),
        app.get_webview_window(&banner_label(owner)),
    ) else {
        return;
    };
    if banner.is_visible().unwrap_or(false) {
        place(&browser, &banner);
    }
}

/// The browser (or its IMT window) is gone: so is the banner.
pub fn close_banner<R: Runtime>(app: &AppHandle<R>, owner: &str) {
    if let Some(w) = app.get_webview_window(&banner_label(owner)) {
        let _ = w.destroy();
    }
}

/// A coloured frame on Windows 11 (DWMWA_BORDER_COLOR); older Windows refuses the attribute, and the title and
/// banner still say it. Other platforms have no such frame.
#[cfg(windows)]
fn set_frame<R: Runtime>(browser: &WebviewWindow<R>, driving: bool) {
    let w = browser.clone();
    let _ = browser.run_on_main_thread(move || {
        use windows::Win32::Foundation::HWND;
        use windows::Win32::Graphics::Dwm::{DwmSetWindowAttribute, DWMWA_BORDER_COLOR};
        let Ok(hwnd) = w.hwnd() else { return };
        let color: u32 = if driving { FRAME_COLOR } else { FRAME_DEFAULT };
        // SAFETY: a live window handle of this process and a u32 of the size given, read during the call only.
        let _ = unsafe {
            DwmSetWindowAttribute(
                HWND(hwnd.0),
                DWMWA_BORDER_COLOR,
                (&color as *const u32).cast(),
                std::mem::size_of::<u32>() as u32,
            )
        };
    });
}

#[cfg(not(windows))]
fn set_frame<R: Runtime>(_browser: &WebviewWindow<R>, _driving: bool) {}

/// Who drives the caller's browser now (the banner and the Web panel start from it, then follow the events).
#[tauri::command]
pub async fn browser_driving<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> CmdResult<Option<AiDriver>> {
    Ok(hub(&app).and_then(|h| h.ai_driver(win.label())))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_title_says_when_the_ai_browses() {
        assert_eq!(window_title("", false), "IMT browser");
        assert_eq!(window_title("  ", true), "AI browsing \u{2014} IMT browser");
        assert_eq!(window_title(" Timeouts ", false), "Timeouts \u{2014} IMT browser");
        assert_eq!(window_title("Timeouts", true), "AI browsing \u{2014} Timeouts \u{2014} IMT browser");
        assert_eq!(window_title("a\nb\u{7}", false), "ab \u{2014} IMT browser", "one line");
        let long = window_title(&"x".repeat(500), true);
        assert!(long.starts_with(DRIVING_PREFIX) && long.chars().count() < 250, "capped");
    }

    #[test]
    fn banner_labels_name_their_owner_and_are_not_browser_windows() {
        assert_eq!(banner_label("w-2"), "aib-w-2");
        assert_eq!(banner_owner("aib-w-2"), Some("w-2"));
        assert_eq!(banner_owner("aib-"), None);
        assert!(is_banner_label("aib-main") && !is_banner_label("main") && !is_banner_label("browser-main"));
        assert!(
            !super::super::is_browser_label(&banner_label("main")),
            "the banner keeps its commands"
        );
        assert_eq!(banner_url("w-2"), "index.html#/ai-browsing?host=w-2");
    }

    #[test]
    fn the_banner_spans_the_top_of_the_client_area() {
        assert_eq!(banner_rect(100, 50, 1100, 1.0), (100, 50, 1100, 40));
        assert_eq!(banner_rect(-8, 0, 2200, 1.5), (-8, 0, 2200, 60));
        assert_eq!(banner_rect(0, 0, 0, f64::NAN), (0, 0, 1, 40), "nonsense scale and size");
        assert_eq!(banner_rect(0, 0, 10, -2.0).3, 40);
    }
}
