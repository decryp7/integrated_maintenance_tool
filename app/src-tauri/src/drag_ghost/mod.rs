//! The drag ghost: while a panel's tab is dragged outside its window, a small native window follows the pointer
//! over other apps and monitors and shows where the popped-out window will open (its title bar with the panel's
//! icon and name, and a dimmed snapshot of the panel).
//!
//! The ghost is one hidden window (`drag-ghost`), created on the first drag and reused: undecorated, always on top,
//! not in the task bar, never focused, click-through and transparent where the platform allows. It loads a static
//! page (`drag-ghost.html`) that has no Debrief commands and no capabilities; the shell hands it what to show.
//!
//! A drag session belongs to the Debrief window that started it: `drag_ghost_start` (content and size), then
//! `drag_ghost_show` as the pointer leaves or re-enters the window, and `drag_ghost_end`, which hides the ghost and
//! answers where it was, so the pop-out opens exactly there. While a session is active a thread follows the global
//! cursor at about 60 Hz. The ghost is never left behind: it hides when the session ends, when the primary button is
//! released without an end (a lost release), when its window loses focus with the button up, when the window is
//! destroyed, and after a time limit.
//!
//! Platforms: Windows and X11 position and show it as described (transparency needs a compositor on X11). macOS
//! needs Tauri's `macos-private-api` for a transparent webview (enabled; see README). Wayland does not let an app
//! place its windows, so there `drag_ghost_start` answers `false` and the UI keeps its in-window preview.

mod geometry;
mod request;

pub use request::GhostRequest;

use crate::panel_windows::Bounds;
use crate::{err, CmdResult};
use geometry::{place, GhostSize, PhysRect, Placement, Screen};
use request::GhostContent;
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::webview::PageLoadEvent;
use tauri::{AppHandle, Manager, Runtime, WebviewUrl, WebviewWindowBuilder, Window, WindowEvent};

/// The ghost window's label (no capability pattern matches it, and no command accepts it).
pub const GHOST_LABEL: &str = "drag-ghost";
/// The page it shows (`app/public/drag-ghost.html`).
const GHOST_PAGE: &str = "drag-ghost.html";
/// How often the ghost follows the cursor.
const TICK: Duration = Duration::from_millis(16);
/// After this many ticks with the primary button up and no end from the UI, the drag is over: hide.
const RELEASED_TICKS: u32 = 15;
/// No drag lasts longer: the ghost hides and the session ends.
const MAX_DRAG: Duration = Duration::from_secs(120);

/// The drag session and the ghost's content. Managed as Tauri state.
#[derive(Default)]
pub struct DragGhosts {
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    session: Option<Session>,
    /// What the page shows (kept to hand it over again when the page reloads).
    content: Option<GhostContent>,
    /// The page has loaded and can take content.
    loaded: bool,
    next: u64,
}

#[derive(Debug, Clone)]
struct Session {
    id: u64,
    host: String,
    size: GhostSize,
    screens: Vec<Screen>,
    visible: bool,
    /// Hidden because the button was released without an end: the end still reports where the ghost was.
    released: bool,
    /// Where the ghost was put last.
    last: Option<Placement>,
}

/// What the tracker does on one tick.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Tick {
    /// Nothing to do this time.
    Idle,
    /// Move the ghost here.
    Move(Placement),
    /// Hide the ghost and stop following.
    Hide,
    /// The session is gone or was replaced: stop following.
    Stop,
}

/// What `drag_ghost_show` does.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Visibility {
    Show(Placement),
    Hide,
    /// Not this window's session, or nowhere to show it.
    Ignore,
}

impl State {
    /// A new session for `host` (replacing any other); its id, and whether the page can take the content now.
    fn begin(&mut self, host: &str, content: GhostContent, size: GhostSize, screens: Vec<Screen>) -> (u64, bool) {
        self.next += 1;
        self.session = Some(Session {
            id: self.next,
            host: host.to_string(),
            size,
            screens,
            visible: false,
            released: false,
            last: None,
        });
        self.content = Some(content);
        (self.next, self.loaded)
    }

    fn session_of(&mut self, host: &str) -> Option<&mut Session> {
        self.session.as_mut().filter(|s| s.host == host)
    }

    fn set_visible(&mut self, host: &str, visible: bool, cursor: Option<(f64, f64)>) -> Visibility {
        let Some(s) = self.session_of(host) else {
            return Visibility::Ignore;
        };
        if !visible {
            s.visible = false;
            return Visibility::Hide;
        }
        match cursor.and_then(|c| place(c, s.size, &s.screens)).or(s.last) {
            Some(p) => {
                s.visible = true;
                s.released = false;
                s.last = Some(p);
                Visibility::Show(p)
            }
            None => Visibility::Ignore,
        }
    }

    /// Ends `host`'s session: where the ghost was if it was showing (or was hidden by a lost release).
    fn finish(&mut self, host: &str) -> Option<Placement> {
        let s = self.session.take_if(|s| s.host == host)?;
        if s.visible || s.released {
            s.last
        } else {
            None
        }
    }

    /// One step of the tracker of session `id`.
    fn tick(&mut self, id: u64, cursor: Option<(f64, f64)>, released: bool, expired: bool) -> Tick {
        let Some(s) = self.session.as_mut().filter(|s| s.id == id) else {
            return Tick::Stop;
        };
        if expired {
            self.session = None;
            return Tick::Hide;
        }
        if released {
            s.released = s.visible;
            s.visible = false;
            return Tick::Hide;
        }
        if !s.visible {
            return Tick::Idle;
        }
        match cursor.and_then(|c| place(c, s.size, &s.screens)) {
            Some(p) if s.last != Some(p) => {
                s.last = Some(p);
                Tick::Move(p)
            }
            _ => Tick::Idle,
        }
    }
}

impl DragGhosts {
    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        crate::sync::lock(&self.state)
    }
}

/// Can windows be placed on screen here? Not on Wayland (unless GTK runs on XWayland), where the compositor
/// decides and ignores the position an app asks for.
pub fn positioning_supported(os: &str, wayland_display: Option<&str>, gdk_backend: Option<&str>) -> bool {
    if os != "linux" {
        return true;
    }
    let forced_x11 = gdk_backend.is_some_and(|b| b.split(',').next().is_some_and(|first| first.trim() == "x11"));
    forced_x11 || wayland_display.is_none_or(str::is_empty)
}

fn positioning_supported_here() -> bool {
    let wayland = std::env::var("WAYLAND_DISPLAY").ok();
    let gdk = std::env::var("GDK_BACKEND").ok();
    positioning_supported(std::env::consts::OS, wayland.as_deref(), gdk.as_deref())
}

/// The monitors, physical px.
fn screens<R: Runtime>(app: &AppHandle<R>) -> Vec<Screen> {
    let rect = |p: tauri::PhysicalPosition<i32>, s: tauri::PhysicalSize<u32>| PhysRect {
        x: p.x,
        y: p.y,
        width: s.width,
        height: s.height,
    };
    app.available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|m| Screen {
            bounds: rect(*m.position(), *m.size()),
            work: rect(m.work_area().position, m.work_area().size),
            scale: m.scale_factor(),
        })
        .collect()
}

fn cursor<R: Runtime>(app: &AppHandle<R>) -> Option<(f64, f64)> {
    app.cursor_position().ok().map(|p| (p.x, p.y))
}

/// The script that hands the page its content: a JSON literal, never markup.
fn content_script(content: &GhostContent) -> Option<String> {
    let json = serde_json::to_string(content).ok()?;
    Some(format!("window.debriefGhost && window.debriefGhost({json});"))
}

fn push_content<R: Runtime>(app: &AppHandle<R>) {
    let Some(content) = app.try_state::<DragGhosts>().and_then(|g| g.lock().content.clone()) else {
        return;
    };
    if let (Some(webview), Some(script)) = (app.get_webview(GHOST_LABEL), content_script(&content)) {
        if let Err(e) = webview.eval(script) {
            log::warn!(Error = e.to_string().as_str(); "Could not hand the drag ghost its content");
        }
    }
}

/// The ghost window, created hidden on first use.
fn ensure_window<R: Runtime>(app: &AppHandle<R>, size: GhostSize) -> tauri::Result<Window<R>> {
    if let Some(w) = app.get_window(GHOST_LABEL) {
        return Ok(w);
    }
    let builder = WebviewWindowBuilder::new(app, GHOST_LABEL, WebviewUrl::App(GHOST_PAGE.into()))
        .title("Debrief drag preview")
        .inner_size(size.width, size.height)
        .decorations(false)
        .transparent(true)
        .shadow(false)
        .always_on_top(true)
        .skip_taskbar(true)
        .resizable(false)
        .focused(false)
        .focusable(false)
        .visible(false)
        .on_page_load(|w, payload| {
            if payload.event() == PageLoadEvent::Finished {
                let app = w.app_handle();
                if let Some(g) = app.try_state::<DragGhosts>() {
                    g.lock().loaded = true;
                }
                push_content(app);
            }
        });
    let window = crate::native_feel::build(builder)?.as_ref().window();
    if let Err(e) = window.set_ignore_cursor_events(true) {
        log::warn!(Error = e.to_string().as_str(); "The drag ghost cannot be made click-through");
    }
    Ok(window)
}

fn apply<R: Runtime>(app: &AppHandle<R>, p: Placement) {
    let Some(w) = app.get_window(GHOST_LABEL) else { return };
    let size = tauri::PhysicalSize::new(p.rect.width, p.rect.height);
    if w.outer_size().ok() != Some(size) {
        let _ = w.set_size(size);
    }
    let _ = w.set_position(tauri::PhysicalPosition::new(p.rect.x, p.rect.y));
}

/// Shows the ghost without activating it (it must never take the focus from the window being dragged from).
fn show<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = app.get_window(GHOST_LABEL) else { return };
    #[cfg(windows)]
    if let Ok(hwnd) = w.hwnd() {
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_SHOWNOACTIVATE};
        // SAFETY: the handle is the live ghost window's; ShowWindow only changes its visibility.
        let _ = unsafe { ShowWindow(hwnd, SW_SHOWNOACTIVATE) };
        return;
    }
    let _ = w.show();
}

fn hide<R: Runtime>(app: &AppHandle<R>) {
    let Some(w) = app.get_window(GHOST_LABEL) else { return };
    #[cfg(windows)]
    if let Ok(hwnd) = w.hwnd() {
        use windows::Win32::UI::WindowsAndMessaging::{ShowWindow, SW_HIDE};
        // SAFETY: as in `show`.
        let _ = unsafe { ShowWindow(hwnd, SW_HIDE) };
        return;
    }
    let _ = w.hide();
}

/// Follows the cursor while session `id` lasts.
fn track<R: Runtime>(app: AppHandle<R>, id: u64) {
    let started = Instant::now();
    let mut up = 0u32;
    loop {
        std::thread::sleep(TICK);
        let Some(ghosts) = app.try_state::<DragGhosts>() else { return };
        up = if crate::pointer::primary_button_down() == Some(false) {
            up + 1
        } else {
            0
        };
        let step = ghosts
            .lock()
            .tick(id, cursor(&app), up >= RELEASED_TICKS, started.elapsed() > MAX_DRAG);
        match step {
            Tick::Idle => {}
            Tick::Move(p) => apply(&app, p),
            Tick::Hide => {
                hide(&app);
                return;
            }
            Tick::Stop => return,
        }
    }
}

/// Ends `host`'s session (if it has one) and hides the ghost.
pub fn end_for<R: Runtime>(app: &AppHandle<R>, host: &str) -> Option<Bounds> {
    let placed = app.try_state::<DragGhosts>()?.lock().finish(host);
    hide(app);
    placed.map(|p| p.logical())
}

/// Window events that end a drag: the dragging window lost the focus with the button up, or was destroyed (then,
/// with no Debrief window left, the hidden ghost goes too, so it never keeps Debrief running).
pub fn on_window_event<R: Runtime>(window: &Window<R>, event: &WindowEvent) {
    let app = window.app_handle();
    let label = window.label();
    match event {
        WindowEvent::Focused(false) if label != GHOST_LABEL && crate::pointer::primary_button_down() != Some(true) => {
            end_for(app, label);
        }
        WindowEvent::Destroyed if label == GHOST_LABEL => {
            if let Some(g) = app.try_state::<DragGhosts>() {
                let mut s = g.lock();
                s.loaded = false;
                s.session = None;
            }
        }
        WindowEvent::Destroyed if !crate::window_manager::is_auxiliary(label) => {
            end_for(app, label);
            let others = app
                .windows()
                .into_keys()
                .any(|l| l != label && !crate::window_manager::is_auxiliary(&l));
            if !others {
                if let Some(w) = app.get_window(GHOST_LABEL) {
                    let _ = w.destroy();
                }
            }
        }
        _ => {}
    }
}

/// Starts a drag ghost for the calling window: `false` (and nothing happens) where it cannot follow the pointer,
/// so the UI keeps its in-window preview.
#[tauri::command]
pub async fn drag_ghost_start<R: Runtime>(app: AppHandle<R>, window: Window<R>, request: GhostRequest) -> CmdResult<bool> {
    let ghost = request.parse()?;
    if !positioning_supported_here() {
        return Ok(false);
    }
    let screens = screens(&app);
    start(&app, window.label(), ghost, screens)
}

/// Starts `host`'s session on these monitors (none: no ghost).
fn start<R: Runtime>(app: &AppHandle<R>, host: &str, ghost: request::Ghost, screens: Vec<Screen>) -> CmdResult<bool> {
    let Some(ghosts) = app.try_state::<DragGhosts>() else {
        return Ok(false);
    };
    if screens.is_empty() {
        return Ok(false);
    }
    // Async: a window must not be created from a synchronous command (it deadlocks on Windows).
    if let Err(e) = ensure_window(app, ghost.size) {
        log::warn!(Error = e.to_string().as_str(); "Could not create the drag ghost; the window shows its own preview");
        return Ok(false);
    }
    hide(app);
    let (id, loaded) = ghosts.lock().begin(host, ghost.content, ghost.size, screens);
    if loaded {
        push_content(app);
    }
    let tracker = app.clone();
    if let Err(e) = std::thread::Builder::new()
        .name("debrief-drag-ghost".into())
        .spawn(move || track(tracker, id))
    {
        ghosts.lock().finish(host);
        return Err(err(e).into());
    }
    Ok(true)
}

/// Shows the calling window's ghost under the pointer (the drag left the window), or hides it (it came back).
#[tauri::command]
pub async fn drag_ghost_show<R: Runtime>(app: AppHandle<R>, window: Window<R>, visible: bool) -> CmdResult<()> {
    let Some(ghosts) = app.try_state::<DragGhosts>() else {
        return Ok(());
    };
    let at = cursor(&app);
    let step = ghosts.lock().set_visible(window.label(), visible, at);
    match step {
        Visibility::Show(p) => {
            apply(&app, p);
            show(&app);
        }
        Visibility::Hide => hide(&app),
        Visibility::Ignore => {}
    }
    Ok(())
}

/// Ends the calling window's drag: hides the ghost and answers where it was (logical px of its monitor), or `null`
/// when it was not showing. The pop-out then opens exactly there.
#[tauri::command]
pub async fn drag_ghost_end<R: Runtime>(app: AppHandle<R>, window: Window<R>) -> Option<Bounds> {
    end_for(&app, window.label())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Now as _;
    use crate::test_support::{test_app, window};
    use crate::windows::MAIN;

    const SIZE: GhostSize = GhostSize {
        width: 400.0,
        height: 300.0,
        grab_x: 100.0,
        grab_y: 10.0,
    };

    fn content() -> GhostContent {
        GhostContent {
            title: "Inspector".into(),
            icon: None,
            snapshot: None,
            dark: false,
        }
    }

    fn screens() -> Vec<Screen> {
        vec![Screen {
            bounds: PhysRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1080,
            },
            work: PhysRect {
                x: 0,
                y: 0,
                width: 1920,
                height: 1040,
            },
            scale: 1.0,
        }]
    }

    fn started() -> (State, u64) {
        let mut s = State::default();
        let (id, loaded) = s.begin(MAIN, content(), SIZE, screens());
        assert!(!loaded);
        (s, id)
    }

    #[test]
    fn the_ghost_follows_the_cursor_only_while_shown() {
        let (mut s, id) = started();
        assert_eq!(s.tick(id, Some((500.0, 500.0)), false, false), Tick::Idle, "hidden: not moved");
        let Visibility::Show(p) = s.set_visible(MAIN, true, Some((500.0, 500.0))) else {
            panic!("not shown")
        };
        assert_eq!((p.rect.x, p.rect.y), (400, 490));
        assert_eq!(s.tick(id, Some((500.0, 500.0)), false, false), Tick::Idle, "unchanged: not moved");
        let Tick::Move(q) = s.tick(id, Some((800.0, 600.0)), false, false) else {
            panic!("not moved")
        };
        assert_eq!((q.rect.x, q.rect.y), (700, 590));
        assert_eq!(s.tick(id, None, false, false), Tick::Idle, "no cursor: stays");
        assert_eq!(s.set_visible(MAIN, false, None), Visibility::Hide);
        assert_eq!(s.tick(id, Some((10.0, 10.0)), false, false), Tick::Idle);
    }

    #[test]
    fn the_end_reports_where_the_ghost_was_only_when_it_was_showing() {
        let (mut s, _) = started();
        s.set_visible(MAIN, true, Some((500.0, 500.0)));
        let b = s.finish(MAIN).unwrap().logical();
        assert_eq!((b.x, b.y, b.width, b.height), (400.0, 490.0, 400.0, 300.0));
        assert!(s.session.is_none());
        assert_eq!(s.finish(MAIN), None, "ended once");
        let (mut s, _) = started();
        s.set_visible(MAIN, true, Some((500.0, 500.0)));
        s.set_visible(MAIN, false, None);
        assert_eq!(s.finish(MAIN), None, "back over the window: no pop-out position");
    }

    #[test]
    fn a_session_belongs_to_its_window() {
        let (mut s, id) = started();
        assert_eq!(s.set_visible("w-2", true, Some((1.0, 1.0))), Visibility::Ignore);
        assert_eq!(s.finish("w-2"), None);
        assert!(s.session.is_some(), "another window cannot end it");
        // A new drag replaces the old one, whose tracker stops.
        let (next, _) = s.begin("w-2", content(), SIZE, screens());
        assert_eq!(s.tick(id, Some((1.0, 1.0)), false, false), Tick::Stop);
        assert_ne!(s.tick(next, Some((1.0, 1.0)), false, false), Tick::Stop);
    }

    #[test]
    fn a_lost_release_hides_the_ghost_and_the_end_still_says_where() {
        let (mut s, id) = started();
        s.set_visible(MAIN, true, Some((500.0, 500.0)));
        assert_eq!(s.tick(id, Some((500.0, 500.0)), true, false), Tick::Hide);
        assert!(s.finish(MAIN).is_some());
        // Released while hidden: nothing to report.
        let (mut s, id) = started();
        assert_eq!(s.tick(id, None, true, false), Tick::Hide);
        assert_eq!(s.finish(MAIN), None);
    }

    #[test]
    fn a_drag_that_never_ends_is_given_up() {
        let (mut s, id) = started();
        s.set_visible(MAIN, true, Some((500.0, 500.0)));
        assert_eq!(s.tick(id, Some((500.0, 500.0)), false, true), Tick::Hide);
        assert!(s.session.is_none());
        assert_eq!(s.tick(id, None, false, false), Tick::Stop);
    }

    #[test]
    fn without_a_cursor_or_monitors_nothing_is_shown() {
        let mut s = State::default();
        s.begin(MAIN, content(), SIZE, Vec::new());
        assert_eq!(s.set_visible(MAIN, true, Some((1.0, 1.0))), Visibility::Ignore);
        let (mut s, _) = started();
        assert_eq!(s.set_visible(MAIN, true, None), Visibility::Ignore);
        // Shown before, the last place is kept when the cursor cannot be read.
        s.set_visible(MAIN, true, Some((500.0, 500.0)));
        assert!(matches!(s.set_visible(MAIN, true, None), Visibility::Show(_)));
    }

    #[test]
    fn wayland_cannot_place_windows() {
        assert!(positioning_supported("windows", None, None));
        assert!(positioning_supported("macos", Some("wayland-0"), None));
        assert!(positioning_supported("linux", None, None), "X11");
        assert!(positioning_supported("linux", Some(""), None));
        assert!(!positioning_supported("linux", Some("wayland-0"), None));
        assert!(!positioning_supported("linux", Some("wayland-0"), Some("wayland,x11")));
        assert!(positioning_supported("linux", Some("wayland-0"), Some("x11")), "XWayland");
        assert!(positioning_supported("linux", Some("wayland-0"), Some(" x11 ,wayland")));
    }

    #[test]
    fn the_page_gets_json_never_markup() {
        let script = content_script(&GhostContent {
            title: "</script><img src=x onerror=alert(1)>\"".into(),
            icon: Some("M1 1".into()),
            snapshot: None,
            dark: true,
        })
        .unwrap();
        assert!(script.starts_with("window.debriefGhost && window.debriefGhost({"));
        let json = script
            .trim_start_matches("window.debriefGhost && window.debriefGhost(")
            .trim_end_matches(");");
        let v: serde_json::Value = serde_json::from_str(json).unwrap();
        assert_eq!(v["title"], "</script><img src=x onerror=alert(1)>\"");
        assert_eq!(v["snapshot"], serde_json::Value::Null);
    }

    /// The ghost's webview gets no capability (no permission at all) and no command of the table.
    #[test]
    fn the_ghost_has_no_permissions_and_no_commands() {
        let matches = |pattern: &str, label: &str| match pattern.strip_suffix('*') {
            Some(prefix) => label.starts_with(prefix),
            None => pattern == label,
        };
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            let v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
            for p in v["webviews"].as_array().unwrap().iter().filter_map(|p| p.as_str()) {
                assert!(!matches(p, GHOST_LABEL), "{}: {p} reaches the drag ghost", path.display());
            }
        }
        assert_eq!(crate::commands::class_of(GHOST_LABEL), None);
        for s in crate::COMMANDS {
            assert!(crate::commands::check(s.name(), GHOST_LABEL, None, 10).is_err(), "{}", s.name());
        }
        // Only Debrief windows start a ghost; a pop-out has its own drag.
        for cmd in ["drag_ghost_start", "drag_ghost_show", "drag_ghost_end"] {
            assert!(crate::commands::check(cmd, MAIN, None, 10).is_ok());
            assert!(crate::commands::check(cmd, "p-main-1", None, 10).is_err());
        }
        assert!(crate::window_manager::is_auxiliary(GHOST_LABEL));
    }

    /// In the (mock) app: the session's commands; the ghost window is created hidden and reused; the commands of a
    /// window without a session do nothing.
    #[test]
    fn commands_drive_one_window_s_session() {
        let app = test_app(None);
        app.manage(DragGhosts::default());
        let main = window(&app, MAIN);
        let w = main.as_ref().window();
        let request = || GhostRequest {
            title: "Inspector".into(),
            icon: None,
            snapshot: None,
            dark: false,
            width: 400.0,
            height: 300.0,
            grab_x: 100.0,
            grab_y: 10.0,
        };
        let ghost = || request().parse().unwrap();
        // Without monitors (a platform that cannot tell) there is no ghost.
        assert!(!start(app.handle(), MAIN, ghost(), Vec::new()).unwrap());
        assert!(app.get_window(GHOST_LABEL).is_none());
        assert_eq!(drag_ghost_end(app.handle().clone(), w.clone()).now(), None);
        assert!(start(app.handle(), MAIN, ghost(), screens()).unwrap());
        assert!(app.get_window(GHOST_LABEL).is_some(), "created hidden");
        // The mock cursor is at 0,0: the ghost sits in the corner of the work area.
        drag_ghost_show(app.handle().clone(), w.clone(), true).now().unwrap();
        drag_ghost_show(app.handle().clone(), w.clone(), false).now().unwrap();
        assert_eq!(
            drag_ghost_end(app.handle().clone(), w.clone()).now(),
            None,
            "back inside: no position"
        );
        assert!(start(app.handle(), MAIN, ghost(), screens()).unwrap(), "reused");
        drag_ghost_show(app.handle().clone(), w.clone(), true).now().unwrap();
        let at = drag_ghost_end(app.handle().clone(), w.clone()).now().unwrap();
        assert_eq!((at.x, at.y, at.width, at.height), (0.0, 0.0, 400.0, 300.0));
        // Another window's calls do nothing to this window's drag.
        assert!(start(app.handle(), MAIN, ghost(), screens()).unwrap());
        let other = window(&app, "w-2").as_ref().window();
        drag_ghost_show(app.handle().clone(), other.clone(), true).now().unwrap();
        assert_eq!(drag_ghost_end(app.handle().clone(), other.clone()).now(), None);
        assert!(app.state::<DragGhosts>().lock().session.is_some());
        // The dragging window goes: its drag ends; with another Debrief window left the ghost stays for later.
        on_window_event(&w, &WindowEvent::Destroyed);
        assert!(app.state::<DragGhosts>().lock().session.is_none());
        assert!(app.get_window(GHOST_LABEL).is_some());
    }

    #[test]
    fn a_malformed_request_is_refused_before_anything_happens() {
        let app = test_app(None);
        app.manage(DragGhosts::default());
        let w = window(&app, MAIN).as_ref().window();
        let bad = GhostRequest {
            title: "x".into(),
            icon: None,
            snapshot: Some("javascript:alert(1)".into()),
            dark: false,
            width: 400.0,
            height: 300.0,
            grab_x: 0.0,
            grab_y: 0.0,
        };
        assert!(drag_ghost_start(app.handle().clone(), w, bad).now().is_err());
        assert!(app.get_window(GHOST_LABEL).is_none());
    }

    #[test]
    fn blur_with_the_button_up_ends_the_drag() {
        let app = test_app(None);
        app.manage(DragGhosts::default());
        let w = window(&app, MAIN).as_ref().window();
        {
            let g = app.state::<DragGhosts>();
            g.lock().begin(MAIN, content(), SIZE, screens());
            g.lock().set_visible(MAIN, true, Some((10.0, 10.0)));
        }
        on_window_event(&w, &WindowEvent::Focused(false));
        let ended = app.state::<DragGhosts>().lock().session.is_none();
        // Nobody holds the mouse while the tests run (where the platform can tell; else the drag ends too).
        assert!(ended);
        // The ghost's own window going away resets the page state.
        app.state::<DragGhosts>().lock().loaded = true;
        let ghost = window(&app, GHOST_LABEL).as_ref().window();
        on_window_event(&ghost, &WindowEvent::Destroyed);
        assert!(!app.state::<DragGhosts>().lock().loaded);
    }

    /// The ghost is always on top on purpose, so its session must end, and the ghost hide, whenever the drag does: when the
    /// UI ends it, when Debrief loses focus with the button up, and when the dragging window goes away. (A mock window
    /// reports itself visible whatever happens, so this checks that the session that keeps the ghost on screen is gone;
    /// `hide` itself runs on each of these paths and the tick loop stops with the session.)
    #[test]
    fn the_ghost_session_ends_with_the_drag_so_the_ghost_never_stays() {
        let app = test_app(None);
        app.manage(DragGhosts::default());
        let main = window(&app, MAIN);
        let w = main.as_ref().window();
        let ghost = || GhostRequest {
            title: "Inspector".into(),
            icon: None,
            snapshot: None,
            dark: false,
            width: 400.0,
            height: 300.0,
            grab_x: 100.0,
            grab_y: 10.0,
        };
        let live = || app.state::<DragGhosts>().lock().session.is_some();
        // Ends with the drag.
        assert!(start(app.handle(), MAIN, ghost().parse().unwrap(), screens()).unwrap());
        drag_ghost_show(app.handle().clone(), w.clone(), true).now().unwrap();
        assert!(live());
        drag_ghost_end(app.handle().clone(), w.clone()).now();
        assert!(!live(), "the drag ended");
        // Ends when the dragging window loses focus (button up).
        assert!(start(app.handle(), MAIN, ghost().parse().unwrap(), screens()).unwrap());
        drag_ghost_show(app.handle().clone(), w.clone(), true).now().unwrap();
        on_window_event(&w, &WindowEvent::Focused(false));
        assert!(!live(), "Debrief lost focus");
        // Ends when the dragging window goes away.
        assert!(start(app.handle(), MAIN, ghost().parse().unwrap(), screens()).unwrap());
        drag_ghost_show(app.handle().clone(), w.clone(), true).now().unwrap();
        on_window_event(&w, &WindowEvent::Destroyed);
        assert!(!live(), "the dragging window was destroyed");
    }
}
