//! The browsers of the open Debrief windows (one per window, opened on demand) and what the shell keeps about
//! them: the page shown, where the Browser panel last placed it, popups waiting for the user's decision, pages
//! handed to a chat, links a chat suggested, and each chat's hold on the browser (`debrief_ai::browser::ChatBrowser`).
//! Plain Rust, no Tauri.

use debrief_ai::browser::{AgentLimits, ChatBrowser, PageCapture, ResearchSettings, SearchSettings};
use serde::Serialize;
use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex, MutexGuard, RwLock};

/// Pages a chat may hold for its next question.
pub const MAX_ATTACHMENTS: usize = 3;
/// Links remembered per chat for "Open".
const MAX_SUGGESTED: usize = 50;
/// The hold of a research job is the chat key `research:<job id>`.
pub const RESEARCH_KEY_PREFIX: &str = "research:";

/// The hold of research job `job` on a window's browser.
pub fn research_key(job: u64) -> String {
    format!("{RESEARCH_KEY_PREFIX}{job}")
}

/// Who drives a window's browser now, as the Browser panel shows it ("AI is browsing — Stop"). The Stop of
/// an agent calls `browser_agent_stop` with its chat; the Stop of research cancels its job (`jobs_cancel`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum AiDriver {
    /// A chat turn in agent mode.
    Agent { session_id: String },
    /// A research job reading result pages.
    Research,
}

/// The browser settings (Settings ▸ Web browser).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BrowserSettings {
    pub search: SearchSettings,
    pub agent: AgentLimits,
    /// Research mode: automatic or offered, and how many results it reads.
    pub research: ResearchSettings,
    /// A browser the user chose (empty: the installed or discovered one).
    pub browser_path: Option<std::path::PathBuf>,
}

#[derive(Default)]
struct Owner {
    chats: HashMap<String, Arc<ChatBrowser>>,
    /// The chat whose agent turn drives the browser now.
    driving: Option<String>,
    /// The driver last announced to the windows ([`BrowserHub::driving_changed`]).
    announced: Option<AiDriver>,
    attachments: HashMap<String, Vec<PageCapture>>,
    suggested: HashMap<String, VecDeque<String>>,
}

impl Owner {
    /// The AI drives the browser: a chat (or research) holds it and was not stopped.
    fn driver(&self) -> Option<AiDriver> {
        let id = self.driving.as_ref()?;
        if !self.chats.get(id)?.may_act() {
            return None;
        }
        Some(if id.starts_with(RESEARCH_KEY_PREFIX) {
            AiDriver::Research
        } else {
            AiDriver::Agent { session_id: id.clone() }
        })
    }
}

#[derive(Default)]
pub struct BrowserHub {
    owners: Mutex<HashMap<String, Owner>>,
    settings: RwLock<BrowserSettings>,
}

impl BrowserHub {
    fn lock(&self) -> MutexGuard<'_, HashMap<String, Owner>> {
        crate::sync::lock(&self.owners)
    }

    fn with<T>(&self, owner: &str, f: impl FnOnce(&mut Owner) -> T) -> T {
        f(self.lock().entry(owner.to_string()).or_default())
    }

    pub fn settings(&self) -> BrowserSettings {
        crate::sync::read(&self.settings).clone()
    }

    pub fn set_settings(&self, s: BrowserSettings) {
        *crate::sync::write(&self.settings) = s;
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
    #[cfg(test)]
    pub fn driver(&self, owner: &str) -> Option<Arc<ChatBrowser>> {
        self.lock()
            .get(owner)
            .and_then(|o| o.driving.as_ref().and_then(|id| o.chats.get(id)).cloned())
    }

    pub fn is_driving(&self, owner: &str, chat: &str) -> bool {
        self.lock().get(owner).is_some_and(|o| o.driving.as_deref() == Some(chat))
    }

    // ---- "AI is browsing" ----

    /// Who drives the window's browser now (a hold that was stopped does not count: the AI no longer acts).
    pub fn ai_driver(&self, owner: &str) -> Option<AiDriver> {
        self.lock().get(owner).and_then(Owner::driver)
    }

    /// The driver changed since it was last announced: `Some(new driver)` to announce (and remember as
    /// announced), `None` when nothing changed or the window is gone. Call after anything that may change it.
    pub fn driving_changed(&self, owner: &str) -> Option<Option<AiDriver>> {
        let mut owners = self.lock();
        let o = owners.get_mut(owner)?;
        let now = o.driver();
        (o.announced != now).then(|| {
            o.announced = now.clone();
            now
        })
    }

    /// Stop: every hold of the window stops (agent turns, read-along, research).
    pub fn stop_all(&self, owner: &str) {
        self.with(owner, |o| {
            for c in o.chats.values() {
                c.stop();
            }
        });
    }

    /// Stop of research: every research hold of the window stops at once (its job ends soon after).
    pub fn stop_research(&self, owner: &str) {
        self.with(owner, |o| {
            for (id, c) in &o.chats {
                if id.starts_with(RESEARCH_KEY_PREFIX) {
                    c.stop();
                }
            }
        });
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

    fn agent(hub: &BrowserHub, chat: &str) -> Arc<ChatBrowser> {
        let c = hub.chat("main", chat);
        c.set_agent(true, AgentLimits::default());
        c
    }

    #[test]
    fn an_agent_turn_drives_until_it_ends_or_stop_or_agent_mode_off() {
        let hub = BrowserHub::default();
        assert_eq!(hub.driving_changed("main"), None, "no window state yet: nothing to announce");
        let c = agent(&hub, "c-1");
        assert_eq!(hub.driving_changed("main"), None, "agent mode alone does not drive");
        assert!(hub.begin_driving("main", "c-1"));
        let driver = AiDriver::Agent { session_id: "c-1".into() };
        assert_eq!(hub.ai_driver("main"), Some(driver.clone()));
        assert_eq!(hub.driving_changed("main"), Some(Some(driver.clone())));
        assert_eq!(hub.driving_changed("main"), None, "announced once");
        // The turn ends.
        hub.end_driving("main", "c-1");
        assert_eq!(hub.driving_changed("main"), Some(None));
        // Stop pressed while the turn still holds the browser.
        assert!(hub.begin_driving("main", "c-1"));
        assert_eq!(hub.driving_changed("main"), Some(Some(driver.clone())));
        c.stop();
        assert_eq!(hub.ai_driver("main"), None);
        assert_eq!(hub.driving_changed("main"), Some(None));
        hub.end_driving("main", "c-1");
        assert_eq!(hub.driving_changed("main"), None, "already announced as stopped");
        // Agent mode turned off.
        c.set_agent(true, AgentLimits::default());
        assert!(hub.begin_driving("main", "c-1"));
        assert_eq!(hub.driving_changed("main"), Some(Some(driver)));
        c.set_agent(false, AgentLimits::default());
        assert_eq!(hub.driving_changed("main"), Some(None));
    }

    #[test]
    fn driving_ends_when_the_browser_or_the_chat_or_the_window_goes() {
        let hub = BrowserHub::default();
        agent(&hub, "c-1");
        assert!(hub.begin_driving("main", "c-1"));
        assert!(hub.driving_changed("main").is_some_and(|d| d.is_some()));
        hub.stop_all("main");
        assert_eq!(hub.driving_changed("main"), Some(None), "stopped: the AI no longer acts");
        hub.end_driving("main", "c-1");
        agent(&hub, "c-1");
        assert!(hub.begin_driving("main", "c-1"));
        assert!(hub.driving_changed("main").is_some_and(|d| d.is_some()));
        hub.forget_chat("main", "c-1");
        assert_eq!(hub.driving_changed("main"), Some(None), "the chat was deleted");
        agent(&hub, "c-2");
        assert!(hub.begin_driving("main", "c-2"));
        hub.forget_owner("main");
        assert_eq!(hub.ai_driver("main"), None);
        assert_eq!(hub.driving_changed("main"), None, "the window is gone: nobody to tell");
    }

    #[test]
    fn research_drives_until_it_finishes_or_is_stopped() {
        let hub = BrowserHub::default();
        let key = research_key(7);
        assert_eq!(key, "research:7");
        let c = agent(&hub, &key);
        assert!(hub.begin_driving("main", &key));
        assert_eq!(hub.ai_driver("main"), Some(AiDriver::Research));
        assert_eq!(hub.driving_changed("main"), Some(Some(AiDriver::Research)));
        // Stop of research stops research holds only.
        let other = agent(&hub, "c-1");
        hub.stop_research("main");
        assert!(!c.may_act() && other.may_act());
        assert_eq!(hub.driving_changed("main"), Some(None), "Stop of research");
        hub.end_driving("main", &key);
        hub.forget_chat("main", &key);
        let again = research_key(8);
        agent(&hub, &again);
        assert!(hub.begin_driving("main", &again));
        assert_eq!(hub.driving_changed("main"), Some(Some(AiDriver::Research)));
        // Finished, failed or cancelled: the job lets go of the browser.
        hub.end_driving("main", &again);
        hub.forget_chat("main", &again);
        assert_eq!(hub.driving_changed("main"), Some(None));
    }

    #[test]
    fn the_driver_goes_to_the_ui_tagged_by_kind() {
        let agent = serde_json::to_value(AiDriver::Agent { session_id: "c-1".into() }).unwrap();
        assert_eq!(agent, serde_json::json!({"kind": "agent", "sessionId": "c-1"}));
        assert_eq!(
            serde_json::to_value(AiDriver::Research).unwrap(),
            serde_json::json!({"kind": "research"})
        );
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
