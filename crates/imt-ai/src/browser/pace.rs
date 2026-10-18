//! Human pace for agent mode: a cap on actions per minute with a minimum gap between them, and a step budget
//! per task ("stopped after 25 steps; continue?"). Pure, with the clock passed in.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

pub const DEFAULT_ACTIONS_PER_MINUTE: u32 = 10;
pub const MIN_ACTIONS_PER_MINUTE: u32 = 2;
pub const MAX_ACTIONS_PER_MINUTE: u32 = 30;
pub const DEFAULT_STEP_LIMIT: u32 = 25;
pub const MIN_STEP_LIMIT: u32 = 5;
pub const MAX_STEP_LIMIT: u32 = 100;
/// The shortest time between two actions, whatever the per-minute cap.
pub const MIN_GAP: Duration = Duration::from_millis(1500);
const WINDOW: Duration = Duration::from_secs(60);

/// At most `per_minute` actions in any minute, and [`MIN_GAP`] between two.
#[derive(Debug, Clone)]
pub struct RateLimiter {
    per_minute: u32,
    recent: VecDeque<Instant>,
}

impl RateLimiter {
    pub fn new(per_minute: u32) -> RateLimiter {
        RateLimiter {
            per_minute: per_minute.clamp(MIN_ACTIONS_PER_MINUTE, MAX_ACTIONS_PER_MINUTE),
            recent: VecDeque::new(),
        }
    }

    pub fn per_minute(&self) -> u32 {
        self.per_minute
    }

    /// How long to wait before the next action may run (`None`: now).
    pub fn wait(&mut self, now: Instant) -> Option<Duration> {
        while self.recent.front().is_some_and(|t| now.saturating_duration_since(*t) >= WINDOW) {
            self.recent.pop_front();
        }
        let gap = self
            .recent
            .back()
            .map(|t| MIN_GAP.saturating_sub(now.saturating_duration_since(*t)))
            .filter(|d| !d.is_zero());
        let full = (self.recent.len() as u32 >= self.per_minute)
            .then(|| {
                self.recent
                    .front()
                    .map(|t| WINDOW.saturating_sub(now.saturating_duration_since(*t)))
            })
            .flatten();
        match (gap, full) {
            (Some(a), Some(b)) => Some(a.max(b)),
            (a, b) => a.or(b),
        }
    }

    /// Notes an action that ran at `now`.
    pub fn record(&mut self, now: Instant) {
        self.recent.push_back(now);
        while self.recent.len() > self.per_minute as usize {
            self.recent.pop_front();
        }
    }
}

/// The steps one agent task may take before it stops and asks to continue.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StepBudget {
    used: u32,
    limit: u32,
    /// Steps one batch grants (the configured limit).
    batch: u32,
}

impl StepBudget {
    pub fn new(limit: u32) -> StepBudget {
        let batch = limit.clamp(MIN_STEP_LIMIT, MAX_STEP_LIMIT);
        StepBudget {
            used: 0,
            limit: batch,
            batch,
        }
    }

    /// Takes one step: its number (from 1), or `Err(used)` when the budget is spent.
    pub fn take(&mut self) -> Result<u32, u32> {
        if self.used >= self.limit {
            return Err(self.used);
        }
        self.used += 1;
        Ok(self.used)
    }

    pub fn used(&self) -> u32 {
        self.used
    }

    pub fn limit(&self) -> u32 {
        self.limit
    }

    pub fn exhausted(&self) -> bool {
        self.used >= self.limit
    }

    /// "Continue": another batch of `limit` steps for the same task.
    pub fn grant_more(&mut self) {
        self.limit = self.used.saturating_add(self.batch);
    }

    /// A new task (a new question, agent mode turned on): the count starts again.
    pub fn reset(&mut self, limit: u32) {
        *self = StepBudget::new(limit);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_keep_a_gap_and_a_per_minute_cap() {
        let t0 = Instant::now();
        let mut r = RateLimiter::new(3);
        assert_eq!(r.per_minute(), 3);
        assert_eq!(r.wait(t0), None);
        r.record(t0);
        assert_eq!(r.wait(t0 + Duration::from_millis(500)), Some(Duration::from_millis(1000)));
        assert_eq!(r.wait(t0 + MIN_GAP), None);
        r.record(t0 + Duration::from_secs(2));
        r.record(t0 + Duration::from_secs(4));
        // Three in this minute: wait until the first one is a minute old.
        assert_eq!(r.wait(t0 + Duration::from_secs(10)), Some(Duration::from_secs(50)));
        assert_eq!(r.wait(t0 + Duration::from_secs(60)), None);
        assert_eq!(RateLimiter::new(0).per_minute(), MIN_ACTIONS_PER_MINUTE);
        assert_eq!(RateLimiter::new(999).per_minute(), MAX_ACTIONS_PER_MINUTE);
    }

    #[test]
    fn the_step_budget_stops_a_task_until_continue() {
        let mut b = StepBudget::new(5);
        for i in 1..=5 {
            assert_eq!(b.take(), Ok(i));
        }
        assert!(b.exhausted());
        assert_eq!(b.take(), Err(5));
        b.grant_more();
        assert_eq!((b.used(), b.limit()), (5, 10));
        assert_eq!(b.take(), Ok(6));
        b.reset(1);
        assert_eq!((b.used(), b.limit()), (0, MIN_STEP_LIMIT));
        assert_eq!(StepBudget::new(1000).limit(), MAX_STEP_LIMIT);
    }
}
