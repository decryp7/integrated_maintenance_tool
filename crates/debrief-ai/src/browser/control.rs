//! A chat's hold on the browser: read-along and agent mode (both off by default, both ending with the chat),
//! Stop, the sites the chat may visit, the step budget and the pace. Shared between the running turn and the
//! UI's commands, so Stop works while the model is busy.

use super::pace::{RateLimiter, StepBudget, DEFAULT_ACTIONS_PER_MINUTE, DEFAULT_STEP_LIMIT};
use super::policy::Grants;
use super::url::{Site, WebUrl};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Mutex, MutexGuard};
use std::time::Instant;

/// Agent mode's limits (Settings ▸ Web browser).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AgentLimits {
    pub step_limit: u32,
    pub actions_per_minute: u32,
}

impl Default for AgentLimits {
    fn default() -> Self {
        AgentLimits {
            step_limit: DEFAULT_STEP_LIMIT,
            actions_per_minute: DEFAULT_ACTIONS_PER_MINUTE,
        }
    }
}

#[derive(Debug)]
struct State {
    grants: Grants,
    budget: StepBudget,
    rate: RateLimiter,
    /// "Continue" granted more steps since the last turn began: the next turn goes on with the same task.
    continued: bool,
    /// The address the agent's current action was approved to load (followed even on a new site).
    expected_load: Option<String>,
    /// The expected load may be redirected once to another site (a search engine's own redirect to a result).
    redirect_once: bool,
}

#[derive(Debug)]
pub struct ChatBrowser {
    read_along: AtomicBool,
    agent: AtomicBool,
    stopped: AtomicBool,
    state: Mutex<State>,
}

/// What the UI shows about a chat's browser access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChatBrowserStatus {
    pub read_along: bool,
    pub agent: bool,
    pub steps_used: u32,
    pub step_limit: u32,
    /// The step budget is spent: "Continue" grants another batch.
    pub paused: bool,
    pub sites: Vec<Site>,
}

impl ChatBrowser {
    pub fn new(limits: AgentLimits) -> ChatBrowser {
        ChatBrowser {
            read_along: AtomicBool::new(false),
            agent: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
            state: Mutex::new(State {
                grants: Grants::default(),
                budget: StepBudget::new(limits.step_limit),
                rate: RateLimiter::new(limits.actions_per_minute),
                continued: false,
                expected_load: None,
                redirect_once: false,
            }),
        }
    }

    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn read_along(&self) -> bool {
        self.read_along.load(Ordering::Relaxed)
    }

    pub fn set_read_along(&self, on: bool) {
        self.read_along.store(on, Ordering::Relaxed);
    }

    pub fn agent(&self) -> bool {
        self.agent.load(Ordering::Relaxed)
    }

    /// Turns agent mode on (a new task with a fresh step budget) or off.
    pub fn set_agent(&self, on: bool, limits: AgentLimits) {
        if on {
            let mut s = self.lock();
            s.budget.reset(limits.step_limit);
            s.rate = RateLimiter::new(limits.actions_per_minute);
            s.continued = false;
            self.stopped.store(false, Ordering::Relaxed);
        }
        self.agent.store(on, Ordering::Relaxed);
    }

    /// Stop: agent mode ends at once; an action waiting for its turn or its approval does not run.
    pub fn stop(&self) {
        self.agent.store(false, Ordering::Relaxed);
        self.stopped.store(true, Ordering::Relaxed);
        self.clear_expected_load();
    }

    /// Agent actions may run now (agent mode is on and Stop was not pressed).
    pub fn may_act(&self) -> bool {
        self.agent() && !self.stopped.load(Ordering::Relaxed)
    }

    /// "Continue" after the step cap: another batch of steps for the same task.
    pub fn continue_task(&self) {
        let mut s = self.lock();
        s.budget.grant_more();
        s.continued = true;
    }

    /// A turn starts: a new question is a new task unless the user just pressed Continue.
    pub fn begin_turn(&self, limits: AgentLimits) {
        let mut s = self.lock();
        if s.continued {
            s.continued = false;
        } else {
            s.budget.reset(limits.step_limit);
        }
    }

    /// Steps left in this task (for sizing the turn's tool rounds).
    pub fn steps_left(&self) -> u32 {
        let s = self.lock();
        s.budget.limit().saturating_sub(s.budget.used())
    }

    /// Takes a step: its number, or `Err(steps used)` when the budget is spent.
    pub fn take_step(&self) -> Result<u32, u32> {
        self.lock().budget.take()
    }

    /// How long the next action has to wait for human pace.
    pub fn pace_wait(&self, now: Instant) -> Option<std::time::Duration> {
        self.lock().rate.wait(now)
    }

    pub fn record_action(&self, now: Instant) {
        self.lock().rate.record(now);
    }

    pub fn grants(&self) -> Grants {
        self.lock().grants.clone()
    }

    pub fn allow_site(&self, site: Site) {
        self.lock().grants.allow(site);
    }

    /// The agent's current action may load `url` (approved, or on an allowed site).
    pub fn expect_load(&self, url: &WebUrl) {
        self.lock().expected_load = Some(url.as_str().to_string());
    }

    /// Like [`Self::expect_load`], and the load may then go on to one other address the site redirects it to,
    /// whatever its site: research opens a result the engine links only through its own redirect (the result's
    /// address is the engine's to give). One navigation, never a chain.
    pub fn expect_redirect(&self, url: &WebUrl) {
        let mut s = self.lock();
        s.expected_load = Some(url.as_str().to_string());
        s.redirect_once = true;
    }

    pub fn clear_expected_load(&self) {
        let mut s = self.lock();
        s.expected_load = None;
        s.redirect_once = false;
    }

    /// The browser window asks before every navigation: while agent mode drives, only allowed sites and the
    /// address an approved action loads. The user's own browsing is never restricted here.
    pub fn allows_navigation(&self, url: &WebUrl) -> bool {
        if !self.may_act() {
            return true;
        }
        let mut s = self.lock();
        if s.grants.allows(&url.site()) || s.expected_load.as_deref() == Some(url.as_str()) {
            return true;
        }
        std::mem::take(&mut s.redirect_once)
    }

    pub fn status(&self) -> ChatBrowserStatus {
        let s = self.lock();
        ChatBrowserStatus {
            read_along: self.read_along(),
            agent: self.agent(),
            steps_used: s.budget.used(),
            step_limit: s.budget.limit(),
            paused: s.budget.exhausted(),
            sites: s.grants.sites().cloned().collect(),
        }
    }
}

/// Agent mode works best with a model of 7 billion parameters or more; smaller ones lose track on long
/// tasks. The UI shows a hint (never a block) when this is true.
pub fn small_for_agent(params_b: Option<f64>) -> bool {
    params_b.is_some_and(|p| p > 0.0 && p < 6.5)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn limits(step_limit: u32) -> AgentLimits {
        AgentLimits {
            step_limit,
            ..AgentLimits::default()
        }
    }

    #[test]
    fn everything_is_off_by_default_and_stop_ends_agent_mode() {
        let c = ChatBrowser::new(AgentLimits::default());
        assert!(!c.read_along() && !c.agent() && !c.may_act());
        c.set_read_along(true);
        c.set_agent(true, limits(5));
        assert!(c.read_along() && c.may_act());
        c.stop();
        assert!(!c.agent() && !c.may_act());
        c.set_agent(true, limits(5));
        assert!(c.may_act(), "turning it on again starts a new task");
        c.set_agent(false, limits(5));
        assert!(!c.may_act());
    }

    #[test]
    fn the_step_budget_belongs_to_a_task_and_continue_extends_it() {
        let c = ChatBrowser::new(limits(5));
        c.set_agent(true, limits(5));
        for _ in 0..5 {
            c.take_step().unwrap();
        }
        assert_eq!(c.take_step(), Err(5));
        assert!(c.status().paused);
        c.continue_task();
        c.begin_turn(limits(5));
        assert_eq!(c.steps_left(), 5);
        assert_eq!(c.take_step(), Ok(6));
        // A new question is a new task.
        c.begin_turn(limits(5));
        assert_eq!((c.status().steps_used, c.status().step_limit), (0, 5));
    }

    #[test]
    fn navigation_while_the_agent_drives_is_limited_to_allowed_and_approved_addresses() {
        let c = ChatBrowser::new(AgentLimits::default());
        let docs = WebUrl::parse("https://learn.microsoft.com/a").unwrap();
        let evil = WebUrl::parse("https://evil.example/collect").unwrap();
        assert!(c.allows_navigation(&evil), "the user browses freely");
        c.set_agent(true, AgentLimits::default());
        assert!(!c.allows_navigation(&docs));
        c.expect_load(&docs);
        assert!(c.allows_navigation(&docs) && !c.allows_navigation(&evil));
        c.clear_expected_load();
        assert!(!c.allows_navigation(&docs));
        c.allow_site(docs.site());
        assert!(c.allows_navigation(&WebUrl::parse("https://learn.microsoft.com/b").unwrap()));
        assert_eq!(c.status().sites, vec![docs.site()]);
        assert!(c.grants().allows(&docs.site()));
        c.stop();
        assert!(c.allows_navigation(&evil), "after Stop the user is in control again");
    }

    #[test]
    fn an_expected_redirect_may_go_to_one_other_site_once() {
        let c = ChatBrowser::new(AgentLimits::default());
        c.set_agent(true, AgentLimits::default());
        let goto = WebUrl::parse("https://www.search.example/goto?url=CAESAbc1").unwrap();
        let result = WebUrl::parse("https://learn.example.org/a").unwrap();
        let next = WebUrl::parse("https://tracker.example/b").unwrap();
        c.expect_redirect(&goto);
        assert!(c.allows_navigation(&goto), "the redirect itself");
        assert!(c.allows_navigation(&goto), "...without using up the hop");
        assert!(c.allows_navigation(&result), "the site it redirects to");
        assert!(!c.allows_navigation(&next), "one hop, never a chain");
        c.expect_redirect(&goto);
        c.clear_expected_load();
        assert!(!c.allows_navigation(&result));
        c.expect_redirect(&goto);
        c.stop();
        c.set_agent(true, AgentLimits::default());
        assert!(!c.allows_navigation(&result), "Stop ends it");
        c.expect_load(&goto);
        assert!(!c.allows_navigation(&result), "a plain expected load is never redirected elsewhere");
    }

    #[test]
    fn pace_is_tracked_per_chat() {
        let c = ChatBrowser::new(AgentLimits::default());
        let t = Instant::now();
        assert_eq!(c.pace_wait(t), None);
        c.record_action(t);
        assert!(c.pace_wait(t).is_some());
    }

    #[test]
    fn small_models_get_a_hint() {
        assert!(small_for_agent(Some(4.0)) && small_for_agent(Some(1.5)));
        assert!(!small_for_agent(Some(7.0)) && !small_for_agent(Some(8.2)) && !small_for_agent(Some(70.0)));
        assert!(!small_for_agent(None) && !small_for_agent(Some(0.0)));
    }
}
