//! What one chat turn gets of the browser: the window's page (read, and driven only by the chat that drives
//! it), the chat's hold, the approver, the activity log and the pages the user added. Built on the command
//! thread, used on the turn's thread.

use super::approvals::{Approvals, ChatApprover};
use super::hub::BrowserHub;
use super::ActivityLog;
use super::{emit, BrowserEvent, BrowserState};
use debrief_ai::browser::{BrowserEnv, ChatBrowser, PageCapture, PageDriver, PageReader, Researcher, SearchSettings};
use std::sync::Arc;
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

/// Tool rounds of a normal turn (the AI's own default).
const NORMAL_ROUNDS: usize = 6;
/// Rounds beyond the step budget (reading, answering).
const EXTRA_ROUNDS: usize = 4;

static SLEEP: fn(Duration) = std::thread::sleep;

pub struct TurnBrowser {
    reader: Box<dyn PageReader>,
    driver: Box<dyn PageDriver>,
    pub control: Arc<ChatBrowser>,
    approver: ChatApprover,
    log: ActivityLog,
    search: SearchSettings,
    /// Pages the user added for this question (taken from the hub: they go with this turn).
    pub pages: Vec<PageCapture>,
    hub: Arc<BrowserHub>,
    owner: String,
    chat: String,
    driving: bool,
    /// Announces the end of the turn's driving ("AI is browsing" clears).
    announce: Box<dyn Fn() + Send + Sync>,
    /// Research mode for the turn's `web_research` (a research job the turn waits for).
    researcher: Box<dyn Researcher>,
}

impl TurnBrowser {
    /// The browser parts for a turn of chat `chat` in window `owner`; `None` without the browser.
    pub fn begin<R: Runtime>(app: &AppHandle<R>, owner: &str, chat: &str) -> Option<TurnBrowser> {
        let hub = app.try_state::<BrowserState>()?.0.clone();
        let approvals = app.try_state::<Arc<Approvals>>()?.inner().clone();
        let settings = hub.settings();
        let control = hub.chat(owner, chat);
        control.begin_turn(settings.agent);
        // Only one chat drives a window's browser; another one's agent turn reads but cannot act.
        let driving = control.may_act() && hub.begin_driving(owner, chat);
        if driving {
            super::driving::refresh(app, owner);
        }
        let (a, o) = (app.clone(), owner.to_string());
        let announce: Box<dyn Fn() + Send + Sync> = Box::new(move || super::driving::refresh(&a, &o));
        let pages = hub.take_attachments(owner, chat);
        if !pages.is_empty() {
            emit(
                app,
                owner,
                BrowserEvent::Sources {
                    session_id: chat.to_string(),
                    sources: vec![],
                },
            );
        }
        // Read along and agent mode show the real browser window next to Debrief.
        let on_screen = control.read_along() || control.may_act();
        let reader = super::page_for(app, owner, on_screen, None, None)?;
        let (h2, o2, c2) = (hub.clone(), owner.to_string(), chat.to_string());
        let may_drive: Option<Box<dyn Fn() -> bool + Send + Sync>> = driving.then(|| Box::new(move || h2.is_driving(&o2, &c2)) as _);
        let driver = super::page_for(app, owner, on_screen, may_drive, Some(control.clone()))?;
        Some(TurnBrowser {
            reader: Box::new(reader),
            driver: Box::new(driver),
            control,
            approver: ChatApprover {
                approvals,
                window: owner.to_string(),
                chat: chat.to_string(),
            },
            log: ActivityLog { window: owner.to_string() },
            search: settings.search,
            pages,
            hub,
            owner: owner.to_string(),
            chat: chat.to_string(),
            driving,
            announce,
            researcher: Box::new(super::research::TurnResearcher {
                app: app.clone(),
                owner: owner.to_string(),
                chat: chat.to_string(),
            }),
        })
    }

    /// The parts as the chat sees them.
    pub fn env(&self) -> BrowserEnv<'_> {
        BrowserEnv {
            reader: self.reader.as_ref(),
            driver: self.driver.as_ref(),
            control: &self.control,
            approver: &self.approver,
            log: &self.log,
            search: self.search.clone(),
            chat: self.chat.clone(),
            sleep: &SLEEP,
            research: Some(self.researcher.as_ref()),
        }
    }

    /// Tool rounds for the turn: enough for the steps left in an agent task.
    pub fn rounds(&self) -> usize {
        if self.control.may_act() {
            (self.control.steps_left() as usize + EXTRA_ROUNDS).max(NORMAL_ROUNDS)
        } else {
            NORMAL_ROUNDS
        }
    }

    /// A link the chat suggested may be opened from its card.
    pub fn note_suggested(&self, url: &str) {
        self.hub.note_suggested(&self.owner, &self.chat, url);
    }
}

impl Drop for TurnBrowser {
    fn drop(&mut self) {
        if self.driving {
            self.hub.end_driving(&self.owner, &self.chat);
            (self.announce)();
        }
    }
}
