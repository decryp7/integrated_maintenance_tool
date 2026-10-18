//! Desktop notifications: which events notify (a pure policy over the user's
//! setting and whether the user can already see the event), a rate limiter so
//! a noisy log cannot flood the desktop, and the thin adapter over
//! `tauri-plugin-notification`. Notification text never contains file contents.

use crate::window_prefs::{NotificationLevel, SharedPrefs};
use std::collections::{HashMap, VecDeque};
use std::sync::Mutex;
use std::time::{Duration, Instant};
use tauri::{AppHandle, Manager, Runtime};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NotificationKind {
    /// A live tail alert in a window the user does not see.
    LiveAlert,
    /// The previous run ended uncleanly.
    CrashDetected,
    /// An import finished in a window that is not in front.
    ImportFinished,
    /// A download (e.g. an AI model) finished.
    #[allow(dead_code)] // kept for the policy table; downloads report as jobs (`JobFinished`)
    DownloadComplete,
    /// A long job finished (finished work: shown at `All`).
    JobFinished,
    /// A job failed.
    JobFailed,
    /// Something needs the user: a plugin or the AI was switched off after crashes.
    Attention,
    /// An update can be installed.
    UpdateAvailable,
}

impl NotificationKind {
    fn is_alert(self) -> bool {
        matches!(
            self,
            NotificationKind::LiveAlert | NotificationKind::CrashDetected | NotificationKind::JobFailed | NotificationKind::Attention
        )
    }
}

/// Whether an event notifies: never when the user already sees it in a window,
/// alerts from `Alerts` up, everything at `All`.
pub fn should_notify(level: NotificationLevel, kind: NotificationKind, seen_in_window: bool) -> bool {
    if seen_in_window {
        return false;
    }
    match level {
        NotificationLevel::Off => false,
        NotificationLevel::Alerts => kind.is_alert(),
        NotificationLevel::All => true,
    }
}

/// At most one notification per kind per `per_kind`, and `burst` in any `window`.
pub struct RateLimiter {
    per_kind: Duration,
    burst: usize,
    window: Duration,
    last: HashMap<NotificationKind, Instant>,
    recent: VecDeque<Instant>,
}

impl Default for RateLimiter {
    fn default() -> Self {
        RateLimiter::new(Duration::from_secs(30), 5, Duration::from_secs(300))
    }
}

impl RateLimiter {
    pub fn new(per_kind: Duration, burst: usize, window: Duration) -> RateLimiter {
        RateLimiter {
            per_kind,
            burst,
            window,
            last: HashMap::new(),
            recent: VecDeque::new(),
        }
    }

    /// Records and allows a notification at `now`, or refuses it.
    pub fn allow(&mut self, kind: NotificationKind, now: Instant) -> bool {
        while self
            .recent
            .front()
            .is_some_and(|t| now.saturating_duration_since(*t) >= self.window)
        {
            self.recent.pop_front();
        }
        if self
            .last
            .get(&kind)
            .is_some_and(|t| now.saturating_duration_since(*t) < self.per_kind)
            || self.recent.len() >= self.burst
        {
            return false;
        }
        self.last.insert(kind, now);
        self.recent.push_back(now);
        true
    }
}

/// Managed state: the limiter shared by every window.
#[derive(Default)]
pub struct Notifier(Mutex<RateLimiter>);

/// Shows a notification when the policy and the rate limit allow it; returns whether it was shown. Never fails.
pub fn notify<R: Runtime>(app: &AppHandle<R>, kind: NotificationKind, seen_in_window: bool, title: &str, body: &str) -> bool {
    let level = app.try_state::<SharedPrefs>().map(|p| p.get().notifications).unwrap_or_default();
    if !should_notify(level, kind, seen_in_window) {
        return false;
    }
    let Some(notifier) = app.try_state::<Notifier>() else {
        return false;
    };
    if !notifier.0.lock().unwrap_or_else(|p| p.into_inner()).allow(kind, Instant::now()) {
        log::debug!("notification rate-limited");
        return false;
    }
    show(app, title, body);
    true
}

#[cfg(not(test))]
fn show<R: Runtime>(app: &AppHandle<R>, title: &str, body: &str) {
    use tauri_plugin_notification::NotificationExt;
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::warn!("could not show a notification: {e}");
    }
}

#[cfg(test)]
fn show<R: Runtime>(_app: &AppHandle<R>, title: &str, _body: &str) {
    tests::SHOWN.with(|s| s.borrow_mut().push(title.to_string()));
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        pub(crate) static SHOWN: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    }

    #[test]
    fn policy() {
        use NotificationKind::*;
        use NotificationLevel::*;
        assert!(!should_notify(Off, CrashDetected, false));
        assert!(should_notify(Alerts, LiveAlert, false));
        assert!(should_notify(Alerts, CrashDetected, false));
        assert!(!should_notify(Alerts, ImportFinished, false));
        assert!(!should_notify(Alerts, DownloadComplete, false));
        assert!(should_notify(All, ImportFinished, false));
        assert!(should_notify(All, DownloadComplete, false));
        assert!(!should_notify(All, LiveAlert, true), "the window shows it already");
    }

    #[test]
    fn rate_limiting() {
        use NotificationKind::*;
        let t0 = Instant::now();
        let s = Duration::from_secs;
        let mut r = RateLimiter::new(s(30), 3, s(300));
        assert!(r.allow(LiveAlert, t0));
        assert!(!r.allow(LiveAlert, t0 + s(10)), "same kind within 30 s");
        assert!(r.allow(ImportFinished, t0 + s(10)), "another kind is independent");
        assert!(r.allow(LiveAlert, t0 + s(31)));
        assert!(!r.allow(CrashDetected, t0 + s(40)), "burst of 3 in 5 minutes reached");
        assert!(r.allow(CrashDetected, t0 + s(301)), "the window slid past the first one");
        // A clock that goes backwards does not panic.
        assert!(!r.allow(CrashDetected, t0));
    }

    #[test]
    fn notify_respects_the_setting_and_the_limit() {
        use tauri::test::{mock_builder, mock_context, noop_assets};
        let app = mock_builder().build(mock_context(noop_assets())).unwrap();
        let h = app.handle();
        SHOWN.with(|s| s.borrow_mut().clear());
        // Without state nothing is shown (and nothing fails).
        notify(h, NotificationKind::LiveAlert, false, "none", "");
        app.manage(Notifier::default());
        app.manage(SharedPrefs::default());
        notify(h, NotificationKind::LiveAlert, false, "first", "");
        notify(h, NotificationKind::LiveAlert, false, "limited", "");
        notify(h, NotificationKind::ImportFinished, false, "not an alert", "");
        notify(h, NotificationKind::CrashDetected, true, "seen", "");
        assert_eq!(SHOWN.with(|s| s.borrow().clone()), ["first"]);
    }
}
