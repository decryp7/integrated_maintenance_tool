//! The page webview (ADR 0024): a child webview `browser-<owner>` on the system webview (WebView2, WKWebView) that
//! IMT keeps exactly over the Browser panel's page area, in the IMT window or in the pop-out window that hosts the
//! panel. It shows only remote pages, with its own profile (`web-profile` in IMT's data folder), no capability (the
//! capabilities name IMT's webviews, never `browser-*`) and no commands (see `guard`). Every navigation passes
//! `imt_ai::browser::check_navigation` and, while the AI drives, the chat's site rules; downloads are blocked; popups
//! never open, the user may open them in the system browser instead.
//!
//! Linux: WebKitGTK child webviews cannot be placed (tauri-runtime-wry packs them into the window's box, below the
//! IMT webview), so there the page gets a window of its own (`browser-<owner>`, the page its only webview) and the
//! panel keeps the address bar, the "AI is browsing" banner and the notices.

use super::geometry::{Placement, View};
use super::hub::PageState;
use super::keys::PageKey;
use super::{emit, BrowserEvent, NoticeKind};
use imt_ai::browser::{check_navigation, Navigation, WebUrl};
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent, WebviewBuilder};
use tauri::{AppHandle, Manager, PhysicalPosition, PhysicalSize, Runtime, Webview, WebviewUrl};

/// Labels of page webviews: `browser-<owner window>`. No capability names them.
pub const PREFIX: &str = "browser-";
/// The browser's profile folder in IMT's data folder (cookies, cache, history; never IMT's own).
pub const WEB_PROFILE_DIR: &str = "web-profile";
/// The browser's WebKit data store on macOS 14+ (WKWebView has no data folder): a fixed, random id.
pub const WEB_STORE_ID: [u8; 16] = [
    0x5b, 0x1e, 0x8c, 0x47, 0x0d, 0xa2, 0x4f, 0x31, 0x9e, 0x6a, 0x2c, 0x77, 0xb4, 0x03, 0xe9, 0x58,
];
/// The page is docked in the panel (a child webview of the window hosting it). Linux: a window of its own.
pub const DOCKED: bool = cfg!(any(windows, target_os = "macos"));
/// The page window's title on Linux.
pub const TITLE: &str = "IMT browser";
/// Longest page title kept for the panel.
const MAX_PAGE_TITLE: usize = 200;

pub fn browser_label(owner: &str) -> String {
    format!("{PREFIX}{owner}")
}

/// The IMT window a page webview belongs to.
pub fn owner_of(label: &str) -> Option<&str> {
    label.strip_prefix(PREFIX).filter(|o| !o.is_empty())
}

pub fn is_browser_label(label: &str) -> bool {
    owner_of(label).is_some()
}

fn hub<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::State<'_, super::BrowserState>> {
    app.try_state::<super::BrowserState>()
}

/// The page webview of `owner`'s browser, when it is open.
pub fn page_webview<R: Runtime>(app: &AppHandle<R>, owner: &str) -> Option<Webview<R>> {
    app.get_webview(&browser_label(owner))
}

fn notice<R: Runtime>(app: &AppHandle<R>, owner: &str, kind: NoticeKind, url: &str, message: String) {
    emit(
        app,
        owner,
        BrowserEvent::Notice {
            kind,
            url: url.chars().take(imt_ai::browser::url::MAX_URL_LEN).collect(),
            message,
        },
    );
}

fn page_changed<R: Runtime>(app: &AppHandle<R>, owner: &str, page: PageState) {
    emit(app, owner, BrowserEvent::State(page));
}

/// Every navigation of the page: web pages only, and while the AI drives, only what the chat allows. The user's own
/// browsing is not limited beyond the scheme rules.
fn allow_navigation<R: Runtime>(app: &AppHandle<R>, owner: &str, url: &tauri::Url) -> bool {
    if let Navigation::Block(reason) = check_navigation(url.as_str()) {
        notice(app, owner, NoticeKind::Blocked, url.as_str(), reason);
        return false;
    }
    let (Some(hub), Ok(web)) = (hub(app), WebUrl::parse(url.as_str())) else {
        return true;
    };
    match hub.driver(owner) {
        Some(c) if !c.allows_navigation(&web) => {
            notice(app, owner, NoticeKind::AgentBlocked, web.as_str(), web.site().to_string());
            log::info!(target: crate::activity::SOURCE_CONTEXT, SourceContext = crate::activity::SOURCE_CONTEXT, Operation = "browser.agent.blocked", Site = web.site().as_str(); "Agent navigation to a site the chat may not visit was blocked");
            false
        }
        _ => true,
    }
}

/// The page webview of `owner`, with every rule of the browser attached.
fn page_builder<R: Runtime>(app: &AppHandle<R>, owner: &str, url: &WebUrl) -> Result<WebviewBuilder<R>, String> {
    let profile = crate::app_dirs::data(app).map_err(crate::err)?.join(WEB_PROFILE_DIR);
    let (a1, a2, a3, a4, a5) = (app.clone(), app.clone(), app.clone(), app.clone(), app.clone());
    let (o1, o2, o3, o4, o5) = (
        owner.to_string(),
        owner.to_string(),
        owner.to_string(),
        owner.to_string(),
        owner.to_string(),
    );
    let builder = WebviewBuilder::new(browser_label(owner), WebviewUrl::External(url.url().clone()))
        // Never IMT's keyboard focus by itself: the user clicks into the page.
        .focused(false)
        // Windows (WebView2 user data folder) and Linux (WebKitGTK data manager)...
        .data_directory(profile)
        // ...and macOS 14 or later (WKWebsiteDataStore); older macOS shares the default store.
        .data_store_identifier(WEB_STORE_ID)
        .on_navigation(move |u| allow_navigation(&a1, &o1, u))
        .on_new_window(move |u, _features| {
            if let Some(hub) = hub(&a2) {
                hub.note_popup(&o2, u.as_str());
            }
            notice(&a2, &o2, NoticeKind::Popup, u.as_str(), String::new());
            NewWindowResponse::Deny
        })
        .on_download(move |_, event| {
            if let DownloadEvent::Requested { url, .. } = event {
                notice(&a3, &o3, NoticeKind::Download, url.as_str(), String::new());
            }
            false
        })
        .on_page_load(move |_, payload| {
            let Some(hub) = hub(&a4) else { return };
            let url = payload.url().as_str().to_string();
            let finished = payload.event() == PageLoadEvent::Finished;
            let page = hub.update_page(&o4, |p| {
                p.open = true;
                p.url = url;
                p.loading = !finished;
                if finished {
                    p.loads += 1;
                }
            });
            page_changed(&a4, &o4, page);
        })
        .on_document_title_changed(move |w, title| {
            let title: String = title.chars().filter(|c| !c.is_control()).take(MAX_PAGE_TITLE).collect();
            if !DOCKED {
                let _ = w.window().set_title(&window_title(&title));
            }
            if let Some(hub) = hub(&a5) {
                let page = hub.update_page(&o5, |p| p.title = title.trim().to_string());
                page_changed(&a5, &o5, page);
            }
        });
    // WebView2 turns SmartScreen off by default for apps; keep it on for the open web (only the page's environment,
    // which has its own data folder, gets these arguments).
    #[cfg(windows)]
    let builder = builder.additional_browser_args("--disable-features=msWebOOUI,msPdfOOUI");
    Ok(builder)
}

/// The Linux page window's title: "<page> — IMT browser" (one line, capped).
pub fn window_title(page_title: &str) -> String {
    let page: String = page_title.chars().filter(|c| !c.is_control()).take(MAX_PAGE_TITLE).collect();
    if page.trim().is_empty() {
        TITLE.to_string()
    } else {
        format!("{} \u{2014} {TITLE}", page.trim())
    }
}

/// Opens the browser of `owner` at `url`, or loads `url` in it, and asks the owner's UI to show the Browser panel
/// (`activate`: as the active tab; the AI's own navigations show it without taking the focus away). Call off the main
/// thread: building a webview there deadlocks on Windows.
pub fn open<R: Runtime>(app: &AppHandle<R>, owner: &str, url: &WebUrl, activate: bool) -> Result<(), String> {
    if let Some(w) = page_webview(app, owner) {
        w.navigate(url.url().clone()).map_err(crate::err)?;
        reveal(app, owner, activate);
        return Ok(());
    }
    let builder = page_builder(app, owner, url)?;
    if DOCKED {
        build_docked(app, owner, builder)?;
    } else {
        build_window(app, owner, builder, activate)?;
    }
    if let Some(hub) = hub(app) {
        let page = hub.update_page(owner, |p| {
            p.open = true;
            p.url = url.as_str().to_string();
            p.loading = true;
        });
        page_changed(app, owner, page);
    }
    reveal(app, owner, activate);
    Ok(())
}

fn reveal<R: Runtime>(app: &AppHandle<R>, owner: &str, activate: bool) {
    emit(app, owner, BrowserEvent::Reveal { activate });
}

/// The page as a child of the window hosting the panel, where the panel last said it is (hidden until it says).
fn build_docked<R: Runtime>(app: &AppHandle<R>, owner: &str, builder: WebviewBuilder<R>) -> Result<(), String> {
    let (host, view) = hub(app).map(|h| h.placement(owner)).unwrap_or((None, View::Hidden));
    let host = host.unwrap_or_else(|| owner.to_string());
    let window = app
        .get_window(&host)
        .or_else(|| app.get_window(owner))
        .ok_or("the window of this browser was closed")?;
    let (pos, size) = match view {
        View::At(r) => (PhysicalPosition::new(r.x, r.y), PhysicalSize::new(r.width, r.height)),
        View::Hidden => (PhysicalPosition::new(0, 0), PhysicalSize::new(1, 1)),
    };
    let webview = window
        .add_child(builder, pos, size)
        .map_err(|e| format!("could not open the browser: {e}"))?;
    apply(&webview, view);
    watch_keys(app, owner, &webview);
    Ok(())
}

/// Linux: the page in a window of its own (its only webview, filling it).
fn build_window<R: Runtime>(app: &AppHandle<R>, owner: &str, builder: WebviewBuilder<R>, focus: bool) -> Result<(), String> {
    let window = tauri::window::WindowBuilder::new(app, browser_label(owner))
        .title(TITLE)
        .inner_size(1100.0, 820.0)
        .min_inner_size(480.0, 360.0)
        .focused(focus)
        .build()
        .map_err(|e| format!("could not open the browser: {e}"))?;
    let size = window.inner_size().map_err(crate::err)?;
    window
        .add_child(builder.auto_resize(), PhysicalPosition::new(0, 0), size)
        .map_err(|e| format!("could not open the browser: {e}"))?;
    Ok(())
}

/// Shows the page webview at its place, or hides it.
fn apply<R: Runtime>(webview: &Webview<R>, view: View) {
    match view {
        View::Hidden => {
            let _ = webview.hide();
        }
        View::At(r) => {
            let _ = webview.set_bounds(tauri::Rect {
                position: PhysicalPosition::new(r.x, r.y).into(),
                size: PhysicalSize::new(r.width, r.height).into(),
            });
            let _ = webview.show();
        }
    }
}

/// The Browser panel of `owner` says where its page area is, from window `host` (the owner itself, or the pop-out
/// window the panel is in): the page webview moves into that window when it is elsewhere, and takes its place or hides.
pub fn place<R: Runtime>(app: &AppHandle<R>, owner: &str, host: &str, placement: Placement) -> Result<View, String> {
    let view = placement.view();
    if let Some(hub) = hub(app) {
        hub.set_placement(owner, host, view);
    }
    if !DOCKED {
        return Ok(view);
    }
    let Some(webview) = page_webview(app, owner) else {
        return Ok(view);
    };
    if webview.window().label() != host {
        let window = app.get_window(host).ok_or("this window was closed")?;
        // Hidden while it moves, so it never shows at its old place in the new window.
        let _ = webview.hide();
        webview.reparent(&window).map_err(|e| format!("could not move the browser: {e}"))?;
    }
    apply(&webview, view);
    Ok(view)
}

/// Window `label` is about to close: when the Browser panel was popped out into it, the remembered place goes back to
/// the owner window, hidden (the panel docks back and places it again; on every platform, also with no page open), and
/// a page webview it hosts (docked platforms) moves back with it, so the page survives the pop-out.
pub fn host_closing<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(owner) = crate::panel_windows::host_of(label) else {
        return;
    };
    if let Some(hub) = hub(app) {
        if hub.placement(owner).0.as_deref() == Some(label) {
            hub.set_placement(owner, owner, View::Hidden);
        }
    }
    let Some(webview) = page_webview(app, owner) else { return };
    if webview.window().label() != label {
        return;
    }
    let _ = webview.hide();
    if let Some(window) = app.get_window(owner) {
        if let Err(e) = webview.reparent(&window) {
            log::warn!(Window = owner; "The browser could not move back from a closing pop-out: {e}");
        }
    }
}

/// Closes the browser of `owner`; false when it was not open.
pub fn close<R: Runtime>(app: &AppHandle<R>, owner: &str) -> Result<bool, String> {
    let Some(webview) = page_webview(app, owner) else {
        return Ok(false);
    };
    if DOCKED {
        webview.close().map_err(crate::err)?;
    } else {
        // Destroy, not close: `close` only asks the page window to close (it would still be there when the panel
        // already shows no page, and a second Open would find it). The page window has nothing to save.
        webview.window().destroy().map_err(crate::err)?;
    }
    closed(app, owner);
    Ok(true)
}

/// The page is gone: the panel shows no page, a driving agent stops, popups are forgotten.
fn closed<R: Runtime>(app: &AppHandle<R>, owner: &str) {
    if let Some(hub) = hub(app) {
        let page = hub.closed(owner);
        page_changed(app, owner, page);
    }
    super::driving::refresh(app, owner);
}

/// A window is gone: when it was the Linux page window, the browser closed.
pub fn destroyed<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(owner) = owner_of(label) {
        closed(app, owner);
    }
}

/// An IMT window is gone: its page webview goes too (it may live in one of its pop-outs) and everything about its
/// browser is forgotten.
pub fn owner_destroyed<R: Runtime>(app: &AppHandle<R>, owner: &str) {
    if let Some(w) = page_webview(app, owner) {
        let _ = if DOCKED { w.close() } else { w.window().destroy() };
    }
    if let Some(hub) = hub(app) {
        hub.forget_owner(owner);
    }
}

/// Gives the keyboard back to IMT: the IMT webview of the window hosting the page (Esc in the page).
#[cfg_attr(not(windows), allow(dead_code))]
fn focus_imt<R: Runtime>(app: &AppHandle<R>, owner: &str) {
    let host = page_webview(app, owner).map_or_else(|| owner.to_string(), |w| w.window().label().to_string());
    if let Some(w) = app.get_webview(&host) {
        let _ = w.set_focus();
    }
}

/// A key IMT takes from the page: Esc gives the keyboard back to IMT; the panel focuses its address bar or replays
/// the shortcut (`key` event). Called by the WebView2 key hook only.
#[cfg_attr(not(windows), allow(dead_code))]
pub fn key_pressed<R: Runtime>(app: &AppHandle<R>, owner: &str, key: PageKey) {
    focus_imt(app, owner);
    emit(app, owner, BrowserEvent::Key(key));
}

/// WebView2 tells IMT about accelerator keys before the page sees them (`AcceleratorKeyPressed`).
#[cfg(windows)]
fn watch_keys<R: Runtime>(app: &AppHandle<R>, owner: &str, webview: &Webview<R>) {
    let (a, o) = (app.clone(), owner.to_string());
    let result = webview.with_webview(move |platform| {
        if let Err(e) = add_key_handler(&platform, a, o) {
            log::warn!(Error = e.to_string().as_str(); "The browser's keys could not be watched");
        }
    });
    if let Err(e) = result {
        log::warn!(Error = e.to_string().as_str(); "The browser's webview could not be reached to watch its keys");
    }
}

#[cfg(windows)]
fn add_key_handler<R: Runtime>(platform: &tauri::webview::PlatformWebview, app: AppHandle<R>, owner: String) -> windows::core::Result<()> {
    use super::keys::{page_key, Mods};
    use webview2_com::AcceleratorKeyPressedEventHandler;
    use webview2_com::Microsoft::Web::WebView2::Win32::{
        COREWEBVIEW2_KEY_EVENT_KIND, COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN, COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN,
    };
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetKeyState, VIRTUAL_KEY, VK_CONTROL, VK_LWIN, VK_MENU, VK_RWIN, VK_SHIFT};
    // SAFETY: GetKeyState reads this thread's keyboard state (the UI thread the handler runs on).
    let down = |k: VIRTUAL_KEY| unsafe { GetKeyState(i32::from(k.0)) } < 0;
    let handler = AcceleratorKeyPressedEventHandler::create(Box::new(move |_, args| {
        let Some(args) = args else { return Ok(()) };
        let mut kind = COREWEBVIEW2_KEY_EVENT_KIND::default();
        let mut vk = 0u32;
        // SAFETY: COM calls on the event's own arguments, during the event, on the UI thread.
        unsafe {
            args.KeyEventKind(&mut kind)?;
            args.VirtualKey(&mut vk)?;
        }
        if kind != COREWEBVIEW2_KEY_EVENT_KIND_KEY_DOWN && kind != COREWEBVIEW2_KEY_EVENT_KIND_SYSTEM_KEY_DOWN {
            return Ok(());
        }
        let mods = Mods {
            ctrl: down(VK_CONTROL),
            shift: down(VK_SHIFT),
            alt: down(VK_MENU),
            meta: down(VK_LWIN) || down(VK_RWIN),
        };
        if let Some(key) = page_key(vk, mods) {
            // SAFETY: as above.
            unsafe { args.SetHandled(true)? };
            key_pressed(&app, &owner, key);
        }
        Ok(())
    }));
    let mut token = 0i64;
    // SAFETY: COM call on the webview's own controller, on the thread `with_webview` runs it on (the UI thread).
    unsafe { platform.controller().add_AcceleratorKeyPressed(&handler, &mut token) }
}

#[cfg(not(windows))]
fn watch_keys<R: Runtime>(_app: &AppHandle<R>, _owner: &str, _webview: &Webview<R>) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn page_labels_name_their_owner() {
        assert_eq!(browser_label("w-2"), "browser-w-2");
        assert_eq!(owner_of("browser-w-2"), Some("w-2"));
        assert_eq!(owner_of("browser-"), None);
        assert!(is_browser_label("browser-main") && !is_browser_label("main") && !is_browser_label("p-main-1"));
    }

    #[test]
    fn the_linux_page_window_is_titled_by_its_page() {
        assert_eq!(window_title(""), "IMT browser");
        assert_eq!(window_title(" Timeouts "), "Timeouts \u{2014} IMT browser");
        assert_eq!(window_title("a\nb\u{7}"), "ab \u{2014} IMT browser", "one line");
        assert!(window_title(&"x".repeat(500)).chars().count() < 250, "capped");
    }

    #[test]
    fn the_page_is_docked_where_child_webviews_can_be_placed() {
        assert_eq!(DOCKED, cfg!(any(windows, target_os = "macos")));
    }
}
