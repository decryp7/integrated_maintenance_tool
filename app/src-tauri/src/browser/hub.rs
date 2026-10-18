//! The browsers of the open IMT windows (one per window, opened on demand) and what the shell keeps about
//! them: the page shown, popups waiting for the user's decision, pages handed to a chat, links a chat
//! suggested, and each chat's hold on the browser (`imt_ai::browser::ChatBrowser`). Plain Rust, no Tauri.

use imt_ai::browser::{AgentLimits, ChatBrowser, PageCapture, SearchSettings};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant};

/// Pages a chat may hold for its next question.
pub const MAX_ATTACHMENTS: usize = 3;
/// Popups remembered for "open in your browser" (the newest win).
const MAX_POPUPS: usize = 5;
/// Links remembered per chat for "Open".
const MAX_SUGGESTED: usize = 50;

/// The browser settings (Settings ▸ Web browser).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowserSettings {
    pub search: SearchSettings,
    pub agent: AgentLimits,
}

/// The page a window's browser shows, for the Web panel.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageState {
    pub open: bool,
    pub url: String,
    pub title: String,
    pub loading: bool,
    /// Increments with every finished page load.
    #[serde(skip)]
    pub loads: u64,
}

#[derive(Default)]
struct Owner {
    page: PageState,
    chats: HashMap<String, Arc<ChatBrowser>>,
    /// The chat whose agent turn drives the browser now.
    driving: Option<String>,
    popups: VecDeque<String>,
    attachments: HashMap<String, Vec<PageCapture>>,
    suggested: HashMap<String, VecDeque<String>>,
}

#[derive(Default)]
pub struct BrowserHub {
    owners: Mutex<HashMap<String, Owner>>,
    loaded: Condvar,
    settings: RwLock<BrowserSettings>,
}

impl BrowserHub {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Owner>> {
        self.owners.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn with<T>(&self, owner: &str, f: impl FnOnce(&mut Owner) -> T) -> T {
        f(self.lock().entry(owner.to_string()).or_default())
    }

    pub fn settings(&self) -> BrowserSettings {
        self.settings.read().unwrap_or_else(|p| p.into_inner()).clone()
    }

    pub fn set_settings(&self, s: BrowserSettings) {
        *self.settings.write().unwrap_or_else(|p| p.into_inner()) = s;
    }

    // ---- the page ----

    pub fn page(&self, owner: &str) -> PageState {
        self.lock().get(owner).map(|o| o.page.clone()).unwrap_or_default()
    }

    /// Updates the page state; returns the new state.
    pub fn update_page(&self, owner: &str, f: impl FnOnce(&mut PageState)) -> PageState {
        let state = self.with(owner, |o| {
            let before = o.page.loads;
            f(&mut o.page);
            (o.page.clone(), o.page.loads != before)
        });
        if state.1 {
            self.loaded.notify_all();
        }
        state.0
    }

    /// The browser window was closed: the page is gone, a driving agent stops, popups are forgotten.
    pub fn closed(&self, owner: &str) -> PageState {
        let page = self.update_page(owner, |p| {
            *p = PageState {
                loads: p.loads + 1,
                ..PageState::default()
            }
        });
        self.with(owner, |o| {
            o.popups.clear();
            if let Some(c) = o.driving.as_ref().and_then(|id| o.chats.get(id)) {
                c.stop();
            }
        });
        page
    }

    /// Waits until a page load finishes after `since` loads (or `timeout` passes); true when one did.
    pub fn wait_loaded(&self, owner: &str, since: u64, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut guard = self.lock();
        loop {
            let loads = guard.get(owner).map_or(since, |o| o.page.loads);
            if loads > since {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return false;
            }
            guard = self.loaded.wait_timeout(guard, left).unwrap_or_else(|p| p.into_inner()).0;
        }
    }

    // ---- popups ----

    pub fn note_popup(&self, owner: &str, url: &str) {
        self.with(owner, |o| {
            o.popups.retain(|u| u != url);
            o.popups.push_back(url.to_string());
            while o.popups.len() > MAX_POPUPS {
                o.popups.pop_front();
            }
        });
    }

    /// Whether `url` may open in the system browser: the page shown, or a popup it asked for.
    pub fn may_open_externally(&self, owner: &str, url: &str) -> bool {
        self.lock()
            .get(owner)
            .is_some_and(|o| o.page.url == url || o.popups.iter().any(|u| u == url))
    }

    // ---- chats ----

    /// The chat's hold on the browser (created on first use, off by default).
    pub fn chat(&self, owner: &str, chat: &str) -> Arc<ChatBrowser> {
        let limits = self.settings().agent;
        self.with(owner, |o| {
            o.chats
                .entry(chat.to_string())
                .or_insert_with(|| Arc::new(ChatBrowser::new(limits)))
                .clone()
        })
    }

    /// The chat is deleted: its access, pages and links end with it.
    pub fn forget_chat(&self, owner: &str, chat: &str) {
        self.with(owner, |o| {
            if let Some(c) = o.chats.remove(chat) {
                c.stop();
            }
            o.attachments.remove(chat);
            o.suggested.remove(chat);
            if o.driving.as_deref() == Some(chat) {
                o.driving = None;
            }
        });
    }

    /// A window is gone: everything of it is dropped (its chats stop).
    pub fn forget_owner(&self, owner: &str) {
        if let Some(o) = self.lock().remove(owner) {
            for c in o.chats.values() {
                c.stop();
            }
        }
        self.loaded.notify_all();
    }

    /// The chat that drives the browser now: Ok when it may (no other chat drives it).
    pub fn begin_driving(&self, owner: &str, chat: &str) -> bool {
        self.with(owner, |o| match &o.driving {
            Some(other) if other != chat => false,
            _ => {
                o.driving = Some(chat.to_string());
                true
            }
        })
    }

    pub fn end_driving(&self, owner: &str, chat: &str) {
        self.with(owner, |o| {
            if o.driving.as_deref() == Some(chat) {
                o.driving = None;
            }
        });
    }

    /// The driving chat's hold, if a chat drives the browser.
    pub fn driver(&self, owner: &str) -> Option<Arc<ChatBrowser>> {
        self.lock()
            .get(owner)
            .and_then(|o| o.driving.as_ref().and_then(|id| o.chats.get(id)).cloned())
    }

    pub fn is_driving(&self, owner: &str, chat: &str) -> bool {
        self.lock().get(owner).is_some_and(|o| o.driving.as_deref() == Some(chat))
    }

    // ---- pages handed to a chat ----

    /// Adds a captured page to the chat's next question; the oldest goes when there are too many.
    pub fn attach(&self, owner: &str, chat: &str, page: PageCapture) -> usize {
        self.with(owner, |o| {
            let list = o.attachments.entry(chat.to_string()).or_default();
            list.retain(|p| p.url != page.url);
            list.push(page);
            while list.len() > MAX_ATTACHMENTS {
                list.remove(0);
            }
            list.len()
        })
    }

    pub fn attachments(&self, owner: &str, chat: &str) -> Vec<PageCapture> {
        self.lock()
            .get(owner)
            .and_then(|o| o.attachments.get(chat).cloned())
            .unwrap_or_default()
    }

    /// The pages for the turn that starts now (they go with it and are cleared).
    pub fn take_attachments(&self, owner: &str, chat: &str) -> Vec<PageCapture> {
        self.with(owner, |o| o.attachments.remove(chat).unwrap_or_default())
    }

    pub fn discard_attachments(&self, owner: &str, chat: &str) {
        self.with(owner, |o| {
            o.attachments.remove(chat);
        });
    }

    // ---- links a chat suggested ----

    pub fn note_suggested(&self, owner: &str, chat: &str, url: &str) {
        self.with(owner, |o| {
            let list = o.suggested.entry(chat.to_string()).or_default();
            list.retain(|u| u != url);
            list.push_back(url.to_string());
            while list.len() > MAX_SUGGESTED {
                list.pop_front();
            }
        });
    }

    pub fn was_suggested(&self, owner: &str, chat: &str, url: &str) -> bool {
        self.lock()
            .get(owner)
            .and_then(|o| o.suggested.get(chat))
            .is_some_and(|l| l.iter().any(|u| u == url))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn page(url: &str) -> PageCapture {
        PageCapture {
            title: "T".into(),
            url: url.into(),
            text: "x".into(),
            truncated: false,
            removed: vec![],
        }
    }

    #[test]
    fn page_loads_wake_waiters_and_closing_resets_the_page() {
        let hub = Arc::new(BrowserHub::default());
        assert_eq!(hub.page("main"), PageState::default());
        let since = hub.page("main").loads;
        let h = hub.clone();
        let t = std::thread::spawn(move || h.wait_loaded("main", since, Duration::from_secs(5)));
        std::thread::sleep(Duration::from_millis(20));
        let s = hub.update_page("main", |p| {
            p.open = true;
            p.url = "https://e.org/".into();
            p.loads += 1;
        });
        assert!(t.join().unwrap());
        assert!(s.open && s.url == "https://e.org/");
        assert!(
            !hub.wait_loaded("main", s.loads, Duration::from_millis(10)),
            "no new load: times out"
        );
        let c = hub.chat("main", "chat-1");
        c.set_agent(true, AgentLimits::default());
        assert!(hub.begin_driving("main", "chat-1"));
        let closed = hub.closed("main");
        assert!(!closed.open && closed.url.is_empty());
        assert!(!c.may_act(), "closing the browser stops the agent");
    }

    #[test]
    fn only_the_shown_page_or_a_popup_opens_externally() {
        let hub = BrowserHub::default();
        hub.update_page("main", |p| p.url = "https://e.org/a".into());
        assert!(hub.may_open_externally("main", "https://e.org/a"));
        assert!(!hub.may_open_externally("main", "https://evil.example/"));
        for i in 0..7 {
            hub.note_popup("main", &format!("https://p{i}.example/"));
        }
        assert!(hub.may_open_externally("main", "https://p6.example/"));
        assert!(
            !hub.may_open_externally("main", "https://p0.example/"),
            "only the newest popups are kept"
        );
        assert!(!hub.may_open_externally("w-2", "https://e.org/a"), "per window");
    }

    #[test]
    fn one_chat_drives_at_a_time() {
        let hub = BrowserHub::default();
        assert!(hub.driver("main").is_none());
        hub.chat("main", "a");
        assert!(hub.begin_driving("main", "a") && hub.begin_driving("main", "a"));
        assert!(!hub.begin_driving("main", "b"));
        assert!(hub.is_driving("main", "a") && hub.driver("main").is_some());
        hub.end_driving("main", "b");
        assert!(hub.is_driving("main", "a"));
        hub.end_driving("main", "a");
        assert!(hub.begin_driving("main", "b"));
        hub.forget_chat("main", "b");
        assert!(!hub.is_driving("main", "b"));
    }

    #[test]
    fn pages_wait_for_the_next_question_and_end_with_the_chat() {
        let hub = BrowserHub::default();
        for i in 0..5 {
            hub.attach("main", "c", page(&format!("https://e.org/{i}")));
        }
        assert_eq!(
            hub.attach("main", "c", page("https://e.org/4")),
            MAX_ATTACHMENTS,
            "the same page replaces itself"
        );
        assert_eq!(hub.attachments("main", "c").len(), MAX_ATTACHMENTS);
        assert_eq!(hub.take_attachments("main", "c").len(), MAX_ATTACHMENTS);
        assert!(hub.take_attachments("main", "c").is_empty());
        hub.attach("main", "c", page("https://e.org/x"));
        hub.discard_attachments("main", "c");
        assert!(hub.attachments("main", "c").is_empty());
        hub.note_suggested("main", "c", "https://e.org/s");
        assert!(hub.was_suggested("main", "c", "https://e.org/s") && !hub.was_suggested("main", "d", "https://e.org/s"));
        let c = hub.chat("main", "c");
        c.set_read_along(true);
        hub.forget_chat("main", "c");
        assert!(!hub.was_suggested("main", "c", "https://e.org/s"));
        assert!(
            !hub.chat("main", "c").read_along(),
            "a new chat with the same id starts with everything off"
        );
        hub.chat("w-2", "x").set_agent(true, AgentLimits::default());
        let held = hub.chat("w-2", "x");
        hub.forget_owner("w-2");
        assert!(!held.may_act());
    }

    #[test]
    fn settings_are_shared() {
        let hub = BrowserHub::default();
        let mut s = hub.settings();
        s.agent.step_limit = 9;
        hub.set_settings(s.clone());
        assert_eq!(hub.settings(), s);
        assert_eq!(hub.chat("main", "n").status().step_limit, 9);
    }
}
