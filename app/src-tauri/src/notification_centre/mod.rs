//! The notification centre: things that happened and may need attention (a long job finished, a live
//! alert, files not imported, a plugin quarantined, AI proposals to review, the AI paused, an update, a
//! crash recovered). The Jobs pane shows work; this shows outcomes.
//!
//! - [`centre`]: the pure list (dedupe with counts, caps, read state).
//! - [`persist`]: the versioned records: project-scoped messages travel with the project (UI state key
//!   `notifications`), app-scoped ones live in `notifications.json` in the app data folder.
//! - Here: the managed [`NotificationCentre`], [`post`] for producers (which also raises an OS notification
//!   through [`crate::notify`] when no Debrief window is focused) and the commands.
//!
//! Not to be confused with [`crate::notify`], the OS notification adapter it uses.
//! Events: `debrief://notifications` (the window's list and the message just posted, for its toast) and
//! `debrief://notifications-open` (an OS notification was clicked: open that message).

pub mod centre;
pub mod persist;

use crate::jobs::{Clock, SystemClock};
use crate::notify::NotificationKind;
use crate::windows::CurrentWindow;
use centre::{Centre, NewNotice, Notice, NoticeId, NoticeScope, Posted, APP_CAP, PROJECT_CAP};
use persist::NoticesRecord;
use serde::Serialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

pub const NOTIFICATIONS_EVENT: &str = "debrief://notifications";
pub const OPEN_EVENT: &str = "debrief://notifications-open";
/// An OS notification clicked (Debrief focused) within this time opens its message.
const OPEN_WITHIN_MS: i64 = 120_000;
const FILE_NAME: &str = "notifications.json";
/// How long changes to the app list wait before they are written (bursts are written once).
const SAVE_DELAY: Duration = Duration::from_millis(250);

/// A window's messages: its project's and the app's, newest first.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticesSnapshot {
    pub notices: Vec<Notice>,
    pub unread: usize,
}

/// What a window hears when its list changed.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticesEvent {
    #[serde(flatten)]
    pub snapshot: NoticesSnapshot,
    /// The message just posted or merged (the UI previews it as a toast).
    pub posted: Option<Notice>,
}

#[derive(Debug, Clone, PartialEq)]
struct PendingOpen {
    id: NoticeId,
    window: Option<String>,
    at: i64,
}

/// Managed state: the app's messages and each window's project messages.
pub struct NotificationCentre {
    app: Mutex<Centre>,
    projects: Mutex<HashMap<String, Centre>>,
    next: AtomicU64,
    /// Writes the app list to its file, debounced, off the centre's locks (none: in memory only).
    writer: Option<Arc<crate::config_store::Debounced<Vec<Notice>>>>,
    clock: Arc<dyn Clock>,
    pending: Mutex<Option<PendingOpen>>,
}

use crate::sync::lock;

impl NotificationCentre {
    /// Loads the app's messages from `file` (missing or damaged: none).
    pub fn new(file: Option<PathBuf>, clock: Arc<dyn Clock>) -> NotificationCentre {
        let saved = file.as_deref().map(persist::load_file).unwrap_or_default();
        let app = Centre::with(APP_CAP, saved);
        NotificationCentre {
            next: AtomicU64::new(app.max_id() + 1),
            app: Mutex::new(app),
            projects: Mutex::new(HashMap::new()),
            writer: file.map(|path| {
                crate::config_store::Debounced::new(SAVE_DELAY, move |notices: Vec<Notice>| {
                    if let Err(e) = persist::save_file(&path, &notices) {
                        log::warn!("The notifications could not be saved: {e}");
                    }
                })
            }),
            clock,
            pending: Mutex::new(None),
        }
    }

    fn id(&self) -> NoticeId {
        NoticeId(self.next.fetch_add(1, Ordering::SeqCst))
    }

    /// Queues the app list for its file (written by the debounced writer, never under the centre's locks).
    fn save_app(&self, app: &Centre) {
        if let Some(w) = &self.writer {
            w.schedule(app.notices().to_vec());
        }
    }

    /// Writes what is pending now (Debrief is closing; tests).
    pub fn flush(&self) {
        if let Some(w) = &self.writer {
            w.flush(Duration::from_secs(2));
        }
    }

    /// Posts a message: app-scoped ones to the app list, project-scoped ones to `window`'s (without a
    /// window they go to the app list too).
    pub fn post(&self, window: Option<&str>, mut new: NewNotice) -> Posted {
        let now = self.clock.now_ms();
        let id = self.id();
        if new.link.window.is_none() {
            new.link.window = window.map(str::to_string);
        }
        let posted = match (new.scope, window) {
            (NoticeScope::Project, Some(w)) => lock(&self.projects)
                .entry(w.to_string())
                .or_insert_with(|| Centre::new(PROJECT_CAP))
                .post(new, id, now),
            _ => {
                new.scope = NoticeScope::App;
                let mut app = lock(&self.app);
                let p = app.post(new, id, now);
                self.save_app(&app);
                p
            }
        };
        if posted.unchanged {
            return posted;
        }
        let (source, severity) = (format!("{:?}", posted.notice.source), format!("{:?}", posted.notice.severity));
        let count = posted.notice.count.to_string();
        crate::activity::domain_event(
            if posted.merged {
                "notification.merged"
            } else {
                "notification.posted"
            },
            None,
            &[("Source", &source), ("Severity", &severity), ("Count", &count)],
        );
        posted
    }

    /// Runs `f` on the list holding `id` (the window's project list, then the app list).
    fn with_list<T>(&self, window: &str, f: impl Fn(&mut Centre) -> Option<T>) -> Option<T> {
        if let Some(t) = lock(&self.projects).get_mut(window).and_then(&f) {
            return Some(t);
        }
        let mut app = lock(&self.app);
        let t = f(&mut app);
        if t.is_some() {
            self.save_app(&app);
        }
        t
    }

    pub fn snapshot(&self, window: &str) -> NoticesSnapshot {
        let mut notices: Vec<Notice> = lock(&self.app).notices().to_vec();
        if let Some(p) = lock(&self.projects).get(window) {
            notices.extend(p.notices().iter().cloned());
        }
        notices.sort_by(|a, b| b.at.cmp(&a.at).then(b.id.cmp(&a.id)));
        let unread = notices.iter().filter(|n| !n.read).count();
        NoticesSnapshot { notices, unread }
    }

    pub fn mark_read(&self, window: &str, id: NoticeId) -> bool {
        self.with_list(window, |c| c.mark_read(id).then_some(())).is_some()
    }

    pub fn dismiss(&self, window: &str, id: NoticeId) -> bool {
        self.with_list(window, |c| c.dismiss(id).then_some(())).is_some()
    }

    /// Marks the window's messages (its project's and the app's) read; returns how many were unread.
    pub fn mark_all_read(&self, window: &str) -> usize {
        let project = lock(&self.projects).get_mut(window).map_or(0, Centre::mark_all_read);
        let mut app = lock(&self.app);
        let n = app.mark_all_read();
        if n > 0 {
            self.save_app(&app);
        }
        project + n
    }

    /// The window's project messages, as saved with its project.
    pub fn record(&self, window: &str) -> NoticesRecord {
        NoticesRecord::new(lock(&self.projects).get(window).map(|c| c.notices().to_vec()).unwrap_or_default())
    }

    /// Replaces the window's project messages with saved ones (renumbered); returns why not all loaded.
    pub fn restore(&self, window: &str, value: Option<&Value>) -> Option<String> {
        let (saved, warning) = NoticesRecord::from_value(value);
        let renumbered = saved
            .into_iter()
            .map(|mut n| {
                n.id = self.id();
                n.scope = NoticeScope::Project;
                n.link.window = Some(window.to_string());
                n
            })
            .collect();
        lock(&self.projects).insert(window.to_string(), Centre::with(PROJECT_CAP, renumbered));
        warning
    }

    pub fn forget_window(&self, window: &str) {
        lock(&self.projects).remove(window);
    }

    /// An OS notification for `id` was shown: focusing Debrief soon after opens it.
    fn note_shown(&self, id: NoticeId, window: Option<&str>) {
        *lock(&self.pending) = Some(PendingOpen {
            id,
            window: window.map(str::to_string),
            at: self.clock.now_ms(),
        });
    }

    /// The message to open when Debrief got the focus (once), if an OS notification was shown recently.
    fn take_pending(&self) -> Option<(NoticeId, Option<String>)> {
        let p = lock(&self.pending).take()?;
        (self.clock.now_ms() - p.at <= OPEN_WITHIN_MS).then_some((p.id, p.window))
    }
}

/// Creates the centre (app messages from the data folder). Call from `setup`.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let file = crate::app_dirs::data(app).ok().map(|d| d.join(FILE_NAME));
    app.manage(NotificationCentre::new(file, Arc::new(SystemClock)));
}

/// Whether some Debrief window has the focus (the user sees in-app toasts).
fn debrief_focused<R: Runtime>(app: &AppHandle<R>) -> bool {
    app.windows().iter().any(|(_, w)| w.is_focused().unwrap_or(false))
}

fn emit_snapshot<R: Runtime>(app: &AppHandle<R>, centre: &NotificationCentre, label: &str, posted: Option<&Notice>) {
    let event = NoticesEvent {
        snapshot: centre.snapshot(label),
        posted: posted.cloned(),
    };
    crate::windows::emit_to_window(app, label, NOTIFICATIONS_EVENT, event);
}

fn labels<R: Runtime>(app: &AppHandle<R>) -> Vec<String> {
    app.try_state::<crate::windows::Windows>()
        .map(|w| w.all().iter().map(|s| s.label().to_string()).collect())
        .unwrap_or_default()
}

/// Posts a message for producers and tells the windows that show it. `os` raises an OS notification of
/// that kind when no Debrief window is focused (the user's notification setting and the rate limit apply).
/// Never fails: without the centre nothing happens.
pub fn post<R: Runtime>(app: &AppHandle<R>, window: Option<&str>, new: NewNotice, os: Option<NotificationKind>) -> Option<Notice> {
    let centre = app.try_state::<NotificationCentre>()?;
    let quiet = new.quiet;
    let posted = centre.post(window, new);
    let notice = posted.notice;
    if posted.unchanged {
        return Some(notice);
    }
    // A quiet message updates the list without a toast.
    let toast = (!quiet).then_some(&notice);
    match notice.scope {
        NoticeScope::App => {
            for label in labels(app) {
                emit_snapshot(app, &centre, &label, toast);
            }
        }
        NoticeScope::Project => emit_snapshot(app, &centre, window.unwrap_or_default(), toast),
    }
    if let Some(kind) = os {
        // Focus is a window getter, which waits for the main thread: decided there, never here.
        let (shown, window) = (notice.clone(), window.map(str::to_string));
        crate::ui_dispatch::run(app, move |app| {
            if crate::notify::notify(app, kind, debrief_focused(app), &shown.title, &shown.body) {
                if let Some(centre) = app.try_state::<NotificationCentre>() {
                    centre.note_shown(shown.id, window.as_deref());
                }
            }
        });
    }
    Some(notice)
}

/// A window got the focus: if it was an OS notification's click, the window opens that message.
pub fn on_focus<R: Runtime>(app: &AppHandle<R>, label: &str) {
    let Some(centre) = app.try_state::<NotificationCentre>() else {
        return;
    };
    if let Some((id, window)) = centre.take_pending() {
        let target = window.filter(|w| app.get_window(w).is_some()).unwrap_or_else(|| label.to_string());
        crate::windows::emit_to_window(app, &target, OPEN_EVENT, id);
    }
}

/// A window closed: its project messages go with it.
pub fn forget_window<R: Runtime>(app: &AppHandle<R>, label: &str) {
    if let Some(c) = app.try_state::<NotificationCentre>() {
        c.forget_window(label);
    }
}

fn centre<'a, R: Runtime>(app: &'a AppHandle<R>) -> crate::CmdResult<tauri::State<'a, NotificationCentre>> {
    app.try_state::<NotificationCentre>()
        .ok_or_else(|| crate::CmdError::unavailable("Debrief is not ready yet"))
}

fn changed<R: Runtime>(app: &AppHandle<R>, centre: &NotificationCentre, label: &str, app_wide: bool) {
    if app_wide {
        for l in labels(app) {
            emit_snapshot(app, centre, &l, None);
        }
    } else {
        emit_snapshot(app, centre, label, None);
    }
}

#[tauri::command]
pub async fn notifications_list<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> crate::CmdResult<NoticesSnapshot> {
    Ok(centre(&app)?.snapshot(win.label()))
}

#[tauri::command]
pub async fn notifications_mark_read<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, id: u64) -> crate::CmdResult<bool> {
    let c = centre(&app)?;
    let done = c.mark_read(win.label(), NoticeId(id));
    changed(&app, &c, win.label(), true);
    Ok(done)
}

#[tauri::command]
pub async fn notifications_mark_all_read<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> crate::CmdResult<usize> {
    let c = centre(&app)?;
    let n = c.mark_all_read(win.label());
    changed(&app, &c, win.label(), true);
    Ok(n)
}

#[tauri::command]
pub async fn notifications_dismiss<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, id: u64) -> crate::CmdResult<bool> {
    let c = centre(&app)?;
    let done = c.dismiss(win.label(), NoticeId(id));
    changed(&app, &c, win.label(), true);
    Ok(done)
}

/// The window's project messages for saving (`NoticesRecord` v1).
#[tauri::command]
pub async fn notifications_record<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> crate::CmdResult<NoticesRecord> {
    Ok(centre(&app)?.record(win.label()))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RestoredNotices {
    pub snapshot: NoticesSnapshot,
    pub warning: Option<String>,
}

/// Replaces the window's project messages with saved ones (a damaged or newer record gives none).
#[tauri::command]
pub async fn notifications_restore<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    record: Option<Value>,
) -> crate::CmdResult<RestoredNotices> {
    let c = centre(&app)?;
    let warning = c.restore(win.label(), record.as_ref());
    changed(&app, &c, win.label(), false);
    Ok(RestoredNotices {
        snapshot: c.snapshot(win.label()),
        warning,
    })
}

#[cfg(test)]
mod tests {
    use super::centre::{NoticeAction, NoticeSeverity, NoticeSource};
    use super::*;
    use crate::jobs::ManualClock;
    use crate::test_support::{test_app_with_commands, window, TempDir};
    use crate::tools::tests::ipc;
    use serde_json::json;
    use tauri::Listener;

    fn alert(n: u32) -> NewNotice {
        NewNotice::new(
            NoticeSource::LiveAlert,
            NoticeSeverity::Error,
            format!("ERROR spike on WEB0{n}"),
            NoticeScope::Project,
        )
        .dedupe(format!("live:spike:WEB0{n}"))
    }

    #[test]
    fn app_and_project_lists_are_kept_apart_and_saved_where_they_belong() {
        let tmp = TempDir::new("centre-scopes");
        let file = tmp.0.join("notifications.json");
        let clock = Arc::new(ManualClock::default());
        let c = NotificationCentre::new(Some(file.clone()), clock.clone());
        c.post(Some("main"), alert(1));
        clock.advance(10);
        c.post(Some("main"), alert(1));
        c.post(Some("w-2"), alert(2));
        let update = c.post(
            Some("main"),
            NewNotice::new(
                NoticeSource::Update,
                NoticeSeverity::Info,
                "Debrief 2.0 is available",
                NoticeScope::App,
            )
            .primary(NoticeAction::InstallUpdate),
        );
        c.post(
            None,
            NewNotice::new(NoticeSource::Job, NoticeSeverity::Success, "Model downloaded", NoticeScope::Project),
        );
        let main = c.snapshot("main");
        assert_eq!(main.notices.len(), 3, "its project message and the two app messages");
        assert_eq!(main.unread, 3);
        assert_eq!(main.notices.iter().find(|n| n.source == NoticeSource::LiveAlert).unwrap().count, 2);
        assert_eq!(c.snapshot("w-2").notices.len(), 3);
        // App messages are in the data file; project ones are not.
        c.flush();
        let saved = persist::load_file(&file);
        assert_eq!(saved.len(), 2);
        assert!(saved.iter().all(|n| n.scope == NoticeScope::App));
        assert!(c.mark_read("main", update.notice.id), "app messages are reachable from any window");
        assert_eq!(c.snapshot("w-2").unread, 2);
        assert_eq!(c.mark_all_read("main"), 2);
        c.flush();
        assert!(persist::load_file(&file).iter().all(|n| n.read));
        assert!(c.dismiss("w-2", update.notice.id));
        assert!(!c.dismiss("w-2", update.notice.id));
        assert!(!c.mark_read("main", NoticeId(999)));
        c.flush();
        // A restart reads the app file back, with fresh ids after the saved ones.
        let again = NotificationCentre::new(Some(file), clock);
        assert_eq!(again.snapshot("main").notices.len(), 1);
        assert!(again.post(None, alert(3)).notice.id.0 > update.notice.id.0);
    }

    #[test]
    fn project_records_round_trip_and_damage_gives_an_empty_list() {
        let c = NotificationCentre::new(None, Arc::new(ManualClock::default()));
        c.post(Some("main"), alert(1));
        let record = serde_json::to_value(c.record("main")).unwrap();
        assert_eq!(record["version"], 1);
        assert_eq!(c.restore("w-2", Some(&record)), None);
        let restored = c.snapshot("w-2");
        assert_eq!(restored.notices[0].link.window.as_deref(), Some("w-2"));
        assert_ne!(restored.notices[0].id, c.snapshot("main").notices[0].id, "renumbered");
        assert!(c.restore("w-2", Some(&json!({"version": "x"}))).unwrap().contains("damaged"));
        assert!(c.snapshot("w-2").notices.is_empty());
        c.forget_window("main");
        assert!(c.record("main").notices.is_empty());
    }

    #[test]
    fn a_clicked_os_notification_opens_its_message_once_and_only_soon_after() {
        let clock = Arc::new(ManualClock::default());
        let c = NotificationCentre::new(None, clock.clone());
        c.note_shown(NoticeId(4), Some("main"));
        assert_eq!(c.take_pending(), Some((NoticeId(4), Some("main".to_string()))));
        assert_eq!(c.take_pending(), None, "once");
        c.note_shown(NoticeId(5), None);
        clock.advance(OPEN_WITHIN_MS + 1);
        assert_eq!(c.take_pending(), None, "too late: an ordinary focus");
    }

    #[test]
    fn commands_list_mark_dismiss_and_restore_through_ipc() {
        let app = test_app_with_commands(None);
        let main = window(&app, "main");
        let events = Arc::new(Mutex::new(Vec::<Value>::new()));
        let e = events.clone();
        main.listen(NOTIFICATIONS_EVENT, move |ev| {
            lock(&e).push(serde_json::from_str(ev.payload()).unwrap())
        });
        let first = post(app.handle(), Some("main"), alert(1), None).unwrap();
        post(app.handle(), Some("main"), alert(1), None).unwrap();
        assert_eq!(
            lock(&events).last().unwrap()["posted"]["count"],
            2,
            "the toast previews the merged message"
        );
        let list = ipc(&main, "notifications_list", json!({})).unwrap();
        assert_eq!((list["unread"].as_u64(), list["notices"].as_array().unwrap().len()), (Some(1), 1));
        assert_eq!(ipc(&main, "notifications_mark_read", json!({"id": first.id.0})).unwrap(), true);
        assert_eq!(ipc(&main, "notifications_mark_all_read", json!({})).unwrap(), 0);
        let record = ipc(&main, "notifications_record", json!({})).unwrap();
        assert_eq!(record["notices"][0]["read"], true);
        let restored = ipc(&main, "notifications_restore", json!({"record": {"version": 7, "notices": []}})).unwrap();
        assert!(restored["warning"].as_str().unwrap().contains("newer"));
        ipc(&main, "notifications_restore", json!({"record": record})).unwrap();
        let id = ipc(&main, "notifications_list", json!({})).unwrap()["notices"][0]["id"]
            .as_u64()
            .unwrap();
        assert_eq!(ipc(&main, "notifications_dismiss", json!({"id": id})).unwrap(), true);
        assert_eq!(
            ipc(&main, "notifications_list", json!({})).unwrap()["notices"]
                .as_array()
                .unwrap()
                .len(),
            0
        );
    }

    #[test]
    fn os_notifications_respect_focus_the_setting_and_the_rate_limit() {
        use crate::notify::tests::SHOWN;
        let app = test_app_with_commands(None);
        window(&app, "main");
        SHOWN.with(|s| s.borrow_mut().clear());
        let failed = || NewNotice::new(NoticeSource::Job, NoticeSeverity::Error, "Export failed", NoticeScope::Project);
        post(app.handle(), Some("main"), failed(), Some(NotificationKind::JobFailed));
        post(app.handle(), Some("main"), failed(), Some(NotificationKind::JobFailed));
        assert_eq!(SHOWN.with(|s| s.borrow().clone()), ["Export failed"], "the second is rate-limited");
        // The clicked notification opens its message in the window that gets the focus.
        let opened = Arc::new(Mutex::new(None::<Value>));
        let o = opened.clone();
        window(&app, "main").listen(OPEN_EVENT, move |ev| *lock(&o) = serde_json::from_str(ev.payload()).ok());
        on_focus(app.handle(), "main");
        assert!(lock(&opened).is_some());
        // Without the centre, producers are not affected.
        let bare = tauri::test::mock_builder()
            .build(tauri::test::mock_context(tauri::test::noop_assets()))
            .unwrap();
        assert!(post(bare.handle(), Some("main"), failed(), None).is_none());
        on_focus(bare.handle(), "main");
        forget_window(bare.handle(), "main");
    }

    #[test]
    fn a_recovered_crash_and_safe_mode_are_app_messages() {
        let app = crate::test_support::test_app(None);
        crate::session_notices(app.handle(), true, true);
        crate::session_notices(app.handle(), true, false);
        let list = app.state::<NotificationCentre>().snapshot("main");
        let titles: Vec<&str> = list.notices.iter().map(|n| n.title.as_str()).collect();
        assert_eq!(titles.len(), 2, "one per session: {titles:?}");
        assert!(titles.contains(&"Debrief did not close properly") && titles.contains(&"Debrief runs in safe mode"));
        assert!(list
            .notices
            .iter()
            .all(|n| n.scope == NoticeScope::App && n.source == NoticeSource::Session));
    }
}
