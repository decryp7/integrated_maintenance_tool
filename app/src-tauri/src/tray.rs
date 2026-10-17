//! The system tray icon: show a window or the window list, new window, recent
//! projects (pushed by the UI, which owns that list), background work with
//! Stop actions, capture a diagnostic screenshot, quit. The tooltip shows how
//! many background tasks run.
//!
//! The menu is a pure model ([`menu_model`]) turned into a Tauri menu, and is
//! rebuilt only when that model changes. Without a tray (e.g. Linux without an
//! app indicator) IMT degrades: closing the last window quits.
//!
//! Command: `set_recent_projects`. Event: `imt://capture` (to the target window).

use crate::background::{BackgroundTasks, TaskInfo};
use crate::window_manager::{list, open_new, quit, reveal, target, WindowInfo};
use crate::windows::emit_to_window;
use crate::CmdResult;
use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::{AppHandle, Manager, Runtime};

const TRAY_ID: &str = "imt";
/// Event asking a window to capture a diagnostic screenshot.
pub const CAPTURE_EVENT: &str = "imt://capture";
const MAX_RECENTS: usize = 10;
const MAX_TEXT: usize = 80;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecentProject {
    pub path: String,
    pub name: String,
}

/// Managed state: whether the tray exists, the recent projects and the last menu shown.
#[derive(Default)]
pub struct TrayState {
    available: AtomicBool,
    recents: Mutex<Vec<RecentProject>>,
    shown: Mutex<Option<(Vec<MenuEntry>, String)>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MenuEntry {
    Item { id: String, text: String, enabled: bool },
    Separator,
    Submenu { text: String, items: Vec<MenuEntry> },
}

fn item(id: impl Into<String>, text: impl Into<String>) -> MenuEntry {
    MenuEntry::Item {
        id: id.into(),
        text: short(&text.into()),
        enabled: true,
    }
}

fn disabled(text: &str) -> MenuEntry {
    MenuEntry::Item {
        id: "none".into(),
        text: text.into(),
        enabled: false,
    }
}

fn short(text: &str) -> String {
    let t: String = text.chars().filter(|c| !c.is_control()).collect();
    if t.chars().count() > MAX_TEXT {
        format!("{}\u{2026}", t.chars().take(MAX_TEXT - 1).collect::<String>())
    } else {
        t
    }
}

/// The tray menu for the current windows, recent projects and background work.
pub fn menu_model(windows: &[WindowInfo], recents: &[RecentProject], tasks: &[TaskInfo]) -> Vec<MenuEntry> {
    let mut menu = Vec::new();
    match windows {
        [] => menu.push(item("new-window", "Show IMT")),
        [only] => menu.push(item(format!("window:{}", only.label), "Show IMT")),
        many => {
            for w in many {
                menu.push(item(format!("window:{}", w.label), format!("Show \u{201c}{}\u{201d}", w.title)));
            }
        }
    }
    menu.push(MenuEntry::Separator);
    menu.push(item("new-window", "New window"));
    let recent_items = if recents.is_empty() {
        vec![disabled("No recent projects")]
    } else {
        recents
            .iter()
            .enumerate()
            .map(|(i, r)| item(format!("recent:{i}"), r.name.clone()))
            .collect()
    };
    menu.push(MenuEntry::Submenu {
        text: "Recent projects".into(),
        items: recent_items,
    });
    let task_items = if tasks.is_empty() {
        vec![disabled("Nothing running")]
    } else {
        tasks
            .iter()
            .map(|t| {
                if t.stoppable {
                    item(format!("stop:{}", t.id), format!("Stop {}", t.label))
                } else {
                    disabled(&short(&t.label))
                }
            })
            .collect()
    };
    menu.push(MenuEntry::Submenu {
        text: format!("Background work ({})", tasks.len()),
        items: task_items,
    });
    menu.push(MenuEntry::Separator);
    menu.push(item("capture", "Capture diagnostic screenshot"));
    menu.push(MenuEntry::Separator);
    menu.push(item("quit", "Quit IMT"));
    menu
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrayAction {
    ShowWindow(String),
    NewWindow,
    Recent(usize),
    Stop(String),
    Capture,
    Quit,
}

/// The action of a menu item id (see [`menu_model`]).
pub fn parse_action(id: &str) -> Option<TrayAction> {
    match id {
        "new-window" => Some(TrayAction::NewWindow),
        "capture" => Some(TrayAction::Capture),
        "quit" => Some(TrayAction::Quit),
        _ => {
            let (kind, arg) = id.split_once(':')?;
            match kind {
                "window" if !arg.is_empty() => Some(TrayAction::ShowWindow(arg.into())),
                "recent" => arg.parse().ok().map(TrayAction::Recent),
                "stop" if !arg.is_empty() => Some(TrayAction::Stop(arg.into())),
                _ => None,
            }
        }
    }
}

pub fn tooltip(running: usize) -> String {
    match running {
        0 => "IMT".into(),
        1 => "IMT \u{2014} 1 background task running".into(),
        n => format!("IMT \u{2014} {n} background tasks running"),
    }
}

/// Whether a tray icon exists (closing the last window may keep IMT running in it).
pub fn available<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.try_state::<TrayState>().is_some_and(|t| t.available.load(Ordering::SeqCst))
}

#[cfg(test)]
pub fn set_available<R: Runtime>(app: &AppHandle<R>, on: bool) {
    if let Some(t) = app.try_state::<TrayState>() {
        t.available.store(on, Ordering::SeqCst);
    }
}

/// Creates the tray icon. A failure (no tray on this desktop) is logged and IMT runs without one.
pub fn create<R: Runtime>(app: &AppHandle<R>) {
    if let Err(e) = build(app) {
        log::warn!("no system tray icon: {e}");
        return;
    }
    if let Some(t) = app.try_state::<TrayState>() {
        t.available.store(true, Ordering::SeqCst);
    }
    refresh(app);
}

fn build<R: Runtime>(app: &AppHandle<R>) -> tauri::Result<()> {
    use tauri::tray::TrayIconBuilder;
    let builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip(tooltip(0))
        .menu(&to_menu(app, &menu_model(&[], &[], &[]))?)
        .show_menu_on_left_click(true)
        .on_menu_event(|app, event| {
            if let Some(action) = parse_action(event.id().as_ref()) {
                handle(app, action);
            }
        });
    // A template image follows the macOS menu bar's light/dark appearance.
    #[cfg(target_os = "macos")]
    let builder = builder
        .icon(tauri::image::Image::from_bytes(include_bytes!("../icons/tray-template.png"))?)
        .icon_as_template(true);
    #[cfg(not(target_os = "macos"))]
    let builder = match app.default_window_icon() {
        Some(icon) => builder.icon(icon.clone()),
        None => builder,
    };
    builder.build(app)?;
    Ok(())
}

fn to_menu<R: Runtime>(app: &AppHandle<R>, model: &[MenuEntry]) -> tauri::Result<tauri::menu::Menu<R>> {
    use tauri::menu::{Menu, MenuItem, PredefinedMenuItem, Submenu};
    let menu = Menu::new(app)?;
    for entry in model {
        match entry {
            MenuEntry::Item { id, text, enabled } => menu.append(&MenuItem::with_id(app, id, text, *enabled, None::<&str>)?)?,
            MenuEntry::Separator => menu.append(&PredefinedMenuItem::separator(app)?)?,
            MenuEntry::Submenu { text, items } => {
                let sub = Submenu::new(app, text, true)?;
                for i in items {
                    if let MenuEntry::Item { id, text, enabled } = i {
                        sub.append(&MenuItem::with_id(app, id, text, *enabled, None::<&str>)?)?;
                    }
                }
                menu.append(&sub)?;
            }
        }
    }
    Ok(menu)
}

/// Rebuilds the menu and tooltip when what they show changed. Never fails.
pub fn refresh<R: Runtime>(app: &AppHandle<R>) {
    let Some(state) = app.try_state::<TrayState>() else { return };
    if !state.available.load(Ordering::SeqCst) {
        return;
    }
    let tasks = app.try_state::<BackgroundTasks>().map(|t| t.list()).unwrap_or_default();
    let recents = state.recents.lock().unwrap_or_else(|p| p.into_inner()).clone();
    let model = menu_model(&list(app), &recents, &tasks);
    let tip = tooltip(tasks.len());
    {
        let mut shown = state.shown.lock().unwrap_or_else(|p| p.into_inner());
        if shown.as_ref().is_some_and(|(m, t)| *m == model && *t == tip) {
            return;
        }
        *shown = Some((model.clone(), tip.clone()));
    }
    let Some(tray) = app.tray_by_id(TRAY_ID) else { return };
    let _ = tray.set_tooltip(Some(&tip));
    match to_menu(app, &model) {
        Ok(menu) => {
            let _ = tray.set_menu(Some(menu));
        }
        Err(e) => log::warn!("could not update the tray menu: {e}"),
    }
}

/// Runs a tray action.
pub fn handle<R: Runtime>(app: &AppHandle<R>, action: TrayAction) {
    let result: CmdResult<()> = match action {
        TrayAction::ShowWindow(label) => reveal(app, &label),
        TrayAction::NewWindow => open_new(app, Vec::new()).map(|_| ()).map_err(crate::err),
        TrayAction::Recent(i) => {
            let recent = app
                .try_state::<TrayState>()
                .and_then(|t| t.recents.lock().unwrap_or_else(|p| p.into_inner()).get(i).cloned());
            match recent {
                Some(r) => {
                    crate::instance::open_files(app, crate::instance::existing(&[r.path.into()]), false);
                    Ok(())
                }
                None => Err("that recent project is no longer listed".into()),
            }
        }
        TrayAction::Stop(id) => {
            let handle = app.clone();
            // Stopping may block briefly (joins a worker, runs logman): off the UI thread.
            tauri::async_runtime::spawn_blocking(move || {
                if let Some(tasks) = handle.try_state::<BackgroundTasks>() {
                    if let Err(e) = tasks.stop(&id) {
                        log::warn!("could not stop background work: {e:#}");
                    }
                }
            });
            Ok(())
        }
        TrayAction::Capture => match target(app) {
            Some(label) => {
                let shown = reveal(app, &label);
                emit_to_window(app, &label, CAPTURE_EVENT, ());
                shown
            }
            None => Err("there is no window to capture".into()),
        },
        TrayAction::Quit => {
            quit(app);
            Ok(())
        }
    };
    if let Err(e) = result {
        log::warn!("tray action failed: {e}");
    }
}

/// The UI's recent projects (it owns that list), for the tray's Recent projects menu.
/// Only absolute `.imtproj` paths are kept; at most ten.
#[tauri::command]
pub fn set_recent_projects<R: Runtime>(app: AppHandle<R>, projects: Vec<RecentProject>) -> CmdResult<()> {
    let clean: Vec<RecentProject> = projects
        .into_iter()
        .filter(|p| {
            let path = std::path::Path::new(&p.path);
            p.path.len() <= 4096 && path.is_absolute() && path.extension().is_some_and(|e| e.eq_ignore_ascii_case("imtproj"))
        })
        .map(|p| RecentProject {
            name: if p.name.trim().is_empty() { short(&p.path) } else { short(&p.name) },
            path: p.path,
        })
        .take(MAX_RECENTS)
        .collect();
    let state = app.try_state::<TrayState>().ok_or("IMT is not ready yet")?;
    *state.recents.lock().unwrap_or_else(|p| p.into_inner()) = clean;
    refresh(&app);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::background::TaskKind;
    use crate::test_support::{test_app, window};
    use tauri::Listener;

    fn win(label: &str, title: &str) -> WindowInfo {
        WindowInfo {
            label: label.into(),
            title: title.into(),
            focused: false,
            visible: true,
        }
    }

    fn task(id: &str, stoppable: bool) -> TaskInfo {
        TaskInfo {
            id: id.into(),
            label: format!("Task {id}"),
            kind: TaskKind::LiveTail,
            window: None,
            progress: None,
            stoppable,
            started_at: 0,
        }
    }

    fn ids(menu: &[MenuEntry]) -> Vec<String> {
        menu.iter()
            .flat_map(|e| match e {
                MenuEntry::Item { id, .. } => vec![id.clone()],
                MenuEntry::Separator => vec!["-".into()],
                MenuEntry::Submenu { items, .. } => ids(items),
            })
            .collect()
    }

    #[test]
    fn menu_model_lists_windows_recents_and_tasks() {
        let empty = menu_model(&[], &[], &[]);
        assert_eq!(
            ids(&empty),
            ["new-window", "-", "new-window", "none", "none", "-", "capture", "-", "quit"]
        );
        let one = menu_model(&[win("main", "IMT")], &[], &[]);
        assert_eq!(ids(&one)[0], "window:main");
        let recents = [RecentProject {
            path: "/p/a.imtproj".into(),
            name: "Incident".into(),
        }];
        let many = menu_model(
            &[win("main", "A"), win("w-2", &"long".repeat(40))],
            &recents,
            &[task("live:main", true), task("x", false)],
        );
        assert_eq!(
            ids(&many),
            [
                "window:main",
                "window:w-2",
                "-",
                "new-window",
                "recent:0",
                "stop:live:main",
                "none",
                "-",
                "capture",
                "-",
                "quit"
            ]
        );
        match &many[1] {
            MenuEntry::Item { text, .. } => assert!(text.chars().count() <= MAX_TEXT && text.ends_with('\u{2026}')),
            other => panic!("{other:?}"),
        }
        assert!(many.contains(&MenuEntry::Submenu {
            text: "Background work (2)".into(),
            items: vec![item("stop:live:main", "Stop Task live:main"), disabled("Task x")],
        }));
    }

    #[test]
    fn actions_and_tooltip() {
        assert_eq!(parse_action("window:w-2"), Some(TrayAction::ShowWindow("w-2".into())));
        assert_eq!(parse_action("new-window"), Some(TrayAction::NewWindow));
        assert_eq!(parse_action("recent:3"), Some(TrayAction::Recent(3)));
        assert_eq!(parse_action("stop:live:main"), Some(TrayAction::Stop("live:main".into())));
        assert_eq!(parse_action("capture"), Some(TrayAction::Capture));
        assert_eq!(parse_action("quit"), Some(TrayAction::Quit));
        for bad in ["none", "recent:x", "window:", "stop:", "other:1", ""] {
            assert_eq!(parse_action(bad), None, "{bad}");
        }
        assert_eq!(tooltip(0), "IMT");
        assert_eq!(tooltip(1), "IMT \u{2014} 1 background task running");
        assert_eq!(tooltip(3), "IMT \u{2014} 3 background tasks running");
    }

    #[test]
    fn recents_are_validated_and_actions_run() {
        let app = test_app(None);
        let h = app.handle().clone();
        let main = window(&app, "main");
        let mut list: Vec<RecentProject> = (0..20)
            .map(|i| RecentProject {
                path: format!("/p/{i}.imtproj"),
                name: if i == 0 { " ".into() } else { format!("P{i}") },
            })
            .collect();
        list.push(RecentProject {
            path: "relative.imtproj".into(),
            name: "x".into(),
        });
        list.insert(
            1,
            RecentProject {
                path: "/p/not-a-project.txt".into(),
                name: "x".into(),
            },
        );
        set_recent_projects(h.clone(), list).unwrap();
        let recents = app.state::<TrayState>().recents.lock().unwrap().clone();
        assert_eq!(recents.len(), MAX_RECENTS);
        assert_eq!(recents[0].name, "/p/0.imtproj", "an empty name shows the path");
        assert_eq!(recents[1].name, "P1");

        let captured = std::sync::Arc::new(std::sync::Mutex::new(0));
        let c = captured.clone();
        main.listen(CAPTURE_EVENT, move |_| *c.lock().unwrap() += 1);
        handle(&h, TrayAction::Capture);
        assert_eq!(*captured.lock().unwrap(), 1);
        handle(&h, TrayAction::NewWindow);
        assert!(app.get_webview_window("w-2").is_some());
        handle(&h, TrayAction::ShowWindow("w-2".into()));
        handle(&h, TrayAction::Recent(99));
        // A listed project that does not exist opens nothing.
        handle(&h, TrayAction::Recent(0));
        handle(&h, TrayAction::Stop("nothing".into()));
        // Without a tray, refresh is a no-op; with one (mock), the model is remembered.
        assert!(!available(&h));
        refresh(&h);
        set_available(&h, true);
        refresh(&h);
        assert!(app.state::<TrayState>().shown.lock().unwrap().is_some());
        refresh(&h);
    }
}
