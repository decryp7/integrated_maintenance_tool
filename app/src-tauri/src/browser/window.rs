//! The browser window: a plain window on the system webview (WebView2, WKWebView, WebKitGTK) that shows
//! only remote pages, with its own profile (`web-profile` in IMT's data folder), no capabilities and no
//! commands (see `guard`). Every navigation passes `imt_ai::browser::check_navigation` and, while agent mode
//! drives, the chat's site rules; downloads are blocked; popups never open, the user may open them in the
//! system browser instead.

use super::hub::PageState;
use super::{emit, BrowserEvent, NoticeKind};
use imt_ai::browser::{check_navigation, Navigation, WebUrl};
use tauri::webview::{DownloadEvent, NewWindowResponse, PageLoadEvent};
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder};

/// Labels of browser windows: `browser-<owner window>`.
pub const PREFIX: &str = "browser-";
/// The browser's profile folder in IMT's data folder (cookies, cache, history; never IMT's own).
pub const WEB_PROFILE_DIR: &str = "web-profile";
/// The browser's WebKit data store on macOS 14+ (WKWebView has no data folder): a fixed, random id.
pub const WEB_STORE_ID: [u8; 16] = [
    0x5b, 0x1e, 0x8c, 0x47, 0x0d, 0xa2, 0x4f, 0x31, 0x9e, 0x6a, 0x2c, 0x77, 0xb4, 0x03, 0xe9, 0x58,
];
const TITLE: &str = "IMT browser";
const MAX_TITLE: usize = 200;

pub fn browser_label(owner: &str) -> String {
    format!("{PREFIX}{owner}")
}

/// The IMT window a browser window belongs to.
pub fn owner_of(label: &str) -> Option<&str> {
    label.strip_prefix(PREFIX).filter(|o| !o.is_empty())
}

pub fn is_browser_label(label: &str) -> bool {
    owner_of(label).is_some()
}

fn hub<R: Runtime>(app: &AppHandle<R>) -> Option<tauri::State<'_, super::BrowserState>> {
    app.try_state::<super::BrowserState>()
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

/// Every navigation of the browser window: web pages only, and while agent mode drives, only what the chat
/// allows. The user's own browsing is not limited beyond the scheme rules.
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

/// Opens the browser of `owner` at `url`, or loads `url` in it; `focus` brings it to the front (the user's
/// own actions do, the agent's do not take the focus away from IMT).
pub fn open<R: Runtime>(app: &AppHandle<R>, owner: &str, url: &WebUrl, focus: bool) -> Result<(), String> {
    let label = browser_label(owner);
    if let Some(w) = app.get_webview_window(&label) {
        w.navigate(url.url().clone()).map_err(crate::err)?;
        if focus {
            let _ = w.unminimize();
            let _ = w.show();
            let _ = w.set_focus();
        }
        return Ok(());
    }
    let profile = crate::app_dirs::data(app).map_err(crate::err)?.join(WEB_PROFILE_DIR);
    let (a1, a2, a3, a4, a5) = (app.clone(), app.clone(), app.clone(), app.clone(), app.clone());
    let (o1, o2, o3, o4, o5) = (
        owner.to_string(),
        owner.to_string(),
        owner.to_string(),
        owner.to_string(),
        owner.to_string(),
    );
    let builder = WebviewWindowBuilder::new(app, &label, WebviewUrl::External(url.url().clone()))
        .title(TITLE)
        .inner_size(1100.0, 820.0)
        .min_inner_size(480.0, 360.0)
        .focused(focus)
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
            let title: String = title.chars().filter(|c| !c.is_control()).take(MAX_TITLE).collect();
            let _ = w.set_title(&if title.trim().is_empty() {
                TITLE.to_string()
            } else {
                format!("{} \u{2014} {TITLE}", title.trim())
            });
            if let Some(hub) = hub(&a5) {
                let page = hub.update_page(&o5, |p| p.title = title.trim().to_string());
                page_changed(&a5, &o5, page);
            }
        });
    // WebView2 turns SmartScreen off by default for apps; keep it on for the open web (only this window's
    // environment, which has its own data folder, gets these arguments).
    #[cfg(windows)]
    let builder = builder.additional_browser_args("--disable-features=msWebOOUI,msPdfOOUI");
    builder.build().map_err(|e| format!("could not open the browser: {e}"))?;
    if let Some(hub) = hub(app) {
        let page = hub.update_page(owner, |p| {
            p.open = true;
            p.url = url.as_str().to_string();
            p.loading = true;
        });
        page_changed(app, owner, page);
    }
    Ok(())
}

/// The browser window is gone (closed by the user or with its IMT window).
pub fn destroyed<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(owner) = owner_of(label) else { return };
    if let Some(hub) = hub(app) {
        let page = hub.closed(owner);
        page_changed(app, owner, page);
    }
}

/// An IMT window is gone: its browser closes too and everything about it is forgotten.
pub fn owner_destroyed<R: Runtime>(app: &AppHandle<R>, owner: &str) {
    if let Some(w) = app.get_webview_window(&browser_label(owner)) {
        let _ = w.destroy();
    }
    if let Some(hub) = hub(app) {
        hub.forget_owner(owner);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn browser_labels_name_their_owner() {
        assert_eq!(browser_label("w-2"), "browser-w-2");
        assert_eq!(owner_of("browser-w-2"), Some("w-2"));
        assert_eq!(owner_of("browser-"), None);
        assert!(is_browser_label("browser-main") && !is_browser_label("main") && !is_browser_label("p-main-1"));
    }
}
