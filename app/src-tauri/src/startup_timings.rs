//! Start-up timings (Paddock rule 16): when the process started, setup finished, the main window was created, its
//! page started loading and painted, reported ready, loaded the workspace and became interactive. Logged once per
//! start as one structured "Startup timings" line, readable with the `startup_timings` command, and checked against
//! budgets by the nightly start-up benchmark (`scripts/ci/startup-bench.mjs`).
//!
//! The marks are milliseconds since the process start (the first line of [`crate::run`]). The page's marks are sent
//! as wall-clock times (`performance.timeOrigin + …`), so its first paint counts from when it happened, not from when
//! its message arrived; a time that cannot be right (before the process, in the future) is replaced by the arrival
//! time. Only the main window's first page counts; a reloaded page's marks are ignored.
//!
//! [`Timeline`] is pure (times are passed in); [`StartupTimings`] is the managed state around it.

use serde::{Deserialize, Serialize};
use std::sync::{Mutex, OnceLock};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// A point of the start, in the order it normally happens.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Mark {
    ProcessStart,
    /// `setup` returned: the event loop runs, the window's assets can be served.
    SetupDone,
    WindowCreated,
    /// The main window's page started loading.
    PageLoad,
    /// The page's first contentful paint (the splash or the shell).
    FirstPaint,
    /// The shell rendered and called `window_ready` (ADR 0058).
    WindowReady,
    /// The first workspace summary was applied.
    WorkspaceLoaded,
    /// The loaded workspace was painted: the user can work.
    FirstInteractive,
}

impl Mark {
    pub const ALL: [Mark; 8] = [
        Mark::ProcessStart,
        Mark::SetupDone,
        Mark::WindowCreated,
        Mark::PageLoad,
        Mark::FirstPaint,
        Mark::WindowReady,
        Mark::WorkspaceLoaded,
        Mark::FirstInteractive,
    ];

    fn name(self) -> &'static str {
        match self {
            Mark::ProcessStart => "processStart",
            Mark::SetupDone => "setupDone",
            Mark::WindowCreated => "windowCreated",
            Mark::PageLoad => "pageLoad",
            Mark::FirstPaint => "firstPaint",
            Mark::WindowReady => "windowReady",
            Mark::WorkspaceLoaded => "workspaceLoaded",
            Mark::FirstInteractive => "firstInteractive",
        }
    }
}

/// The marks the page reports (`startup_mark`); the others are Rust's own.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PageMark {
    FirstPaint,
    WorkspaceLoaded,
    FirstInteractive,
}

impl From<PageMark> for Mark {
    fn from(m: PageMark) -> Mark {
        match m {
            PageMark::FirstPaint => Mark::FirstPaint,
            PageMark::WorkspaceLoaded => Mark::WorkspaceLoaded,
            PageMark::FirstInteractive => Mark::FirstInteractive,
        }
    }
}

/// One mark of the reply: its name and milliseconds since the process start.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MarkAt {
    pub mark: Mark,
    pub ms: u64,
}

/// One step of `setup` and how long it took.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StepTook {
    pub step: &'static str,
    pub ms: u64,
}

/// The `startup_timings` reply.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StartupReport {
    /// The marks reached so far, in [`Mark`] order.
    pub marks: Vec<MarkAt>,
    /// `setup`'s steps in the order they ran.
    pub steps: Vec<StepTook>,
    /// Every mark was reached.
    pub complete: bool,
}

/// The marks and steps of one start (pure: every time is passed in, as ms since the process start).
#[derive(Debug, Default, Clone)]
pub struct Timeline {
    marks: Vec<(Mark, u64)>,
    steps: Vec<(&'static str, u64)>,
    /// The end of the last step (ms).
    step_from: u64,
    logged: bool,
}

impl Timeline {
    /// Records `mark` at `ms` unless it was recorded already (the first start counts). Returns whether it was new.
    pub fn mark(&mut self, mark: Mark, ms: u64) -> bool {
        if self.at(mark).is_some() {
            return false;
        }
        self.marks.push((mark, ms));
        true
    }

    pub fn at(&self, mark: Mark) -> Option<u64> {
        self.marks.iter().find(|(m, _)| *m == mark).map(|(_, ms)| *ms)
    }

    /// Starts timing steps at `ms` (the process start by default).
    #[cfg(test)]
    pub fn steps_from(&mut self, ms: u64) {
        self.step_from = ms;
    }

    /// Ends step `name` at `ms` (it ran since the previous step ended).
    pub fn step(&mut self, name: &'static str, ms: u64) {
        self.steps.push((name, ms.saturating_sub(self.step_from)));
        self.step_from = ms;
    }

    pub fn complete(&self) -> bool {
        Mark::ALL.iter().all(|m| self.at(*m).is_some())
    }

    /// True once, when the timeline became complete: the moment to log it.
    pub fn take_log(&mut self) -> bool {
        if self.logged || !self.complete() {
            return false;
        }
        self.logged = true;
        true
    }

    pub fn report(&self) -> StartupReport {
        let marks = Mark::ALL
            .iter()
            .filter_map(|m| self.at(*m).map(|ms| MarkAt { mark: *m, ms }))
            .collect();
        let steps = self.steps.iter().map(|(step, ms)| StepTook { step, ms: *ms }).collect();
        StartupReport {
            marks,
            steps,
            complete: self.complete(),
        }
    }

    /// The steps as `name=ms` pairs, slowest first (the log line's `Steps` field).
    pub fn steps_text(&self) -> String {
        let mut steps = self.steps.clone();
        steps.sort_by_key(|s| std::cmp::Reverse(s.1));
        steps.iter().map(|(n, ms)| format!("{n}={ms}")).collect::<Vec<_>>().join(" ")
    }
}

/// A page's wall-clock time (epoch ms) as ms since the process start; the arrival time when it cannot be right.
pub fn page_time(at_epoch_ms: f64, process_epoch_ms: f64, now_ms: u64) -> u64 {
    let ms = at_epoch_ms - process_epoch_ms;
    // One second of slack for clock granularity between the page's and the process's clocks.
    if !ms.is_finite() || ms < -1000.0 || ms > now_ms as f64 + 1000.0 {
        return now_ms;
    }
    (ms.max(0.0) as u64).min(now_ms)
}

/// When the process started: set by the first line of `run`, else when first asked (tests).
static PROCESS_START: OnceLock<(Instant, SystemTime)> = OnceLock::new();

/// Remembers the process start. Call first thing in `run`.
pub fn process_started() {
    let _ = PROCESS_START.get_or_init(|| (Instant::now(), SystemTime::now()));
}

fn process_start() -> (Instant, SystemTime) {
    *PROCESS_START.get_or_init(|| (Instant::now(), SystemTime::now()))
}

/// The start's timeline (managed state).
pub struct StartupTimings {
    t0: Instant,
    epoch0_ms: f64,
    timeline: Mutex<Timeline>,
}

impl Default for StartupTimings {
    fn default() -> Self {
        let (t0, wall) = process_start();
        let epoch0_ms = wall.duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0);
        let mut timeline = Timeline::default();
        timeline.mark(Mark::ProcessStart, 0);
        StartupTimings {
            t0,
            epoch0_ms,
            timeline: Mutex::new(timeline),
        }
    }
}

impl StartupTimings {
    /// Milliseconds since the process start.
    pub fn now_ms(&self) -> u64 {
        u64::try_from(self.t0.elapsed().as_millis()).unwrap_or(u64::MAX)
    }

    fn with<T>(&self, f: impl FnOnce(&mut Timeline) -> T) -> T {
        f(&mut crate::sync::lock(&self.timeline))
    }

    /// Records one of Rust's marks now.
    pub fn mark(&self, mark: Mark) {
        let ms = self.now_ms();
        self.record(mark, ms);
    }

    /// Records a page's mark sent as a wall-clock time.
    pub fn page_mark(&self, mark: PageMark, at_epoch_ms: f64) {
        let ms = page_time(at_epoch_ms, self.epoch0_ms, self.now_ms());
        self.record(mark.into(), ms);
    }

    fn record(&self, mark: Mark, ms: u64) {
        let log = self.with(|t| t.mark(mark, ms) && t.take_log());
        if log {
            self.log();
        }
    }

    /// Ends `setup` step `name` now.
    pub fn step(&self, name: &'static str) {
        let ms = self.now_ms();
        self.with(|t| t.step(name, ms));
    }

    pub fn report(&self) -> StartupReport {
        self.with(|t| t.report())
    }

    /// The one "Startup timings" line.
    fn log(&self) {
        let (t, steps) = self.with(|t| (t.clone(), t.steps_text()));
        let at = |m: Mark| t.at(m).unwrap_or_default();
        log::info!(
            SetupDoneMs = at(Mark::SetupDone),
            WindowCreatedMs = at(Mark::WindowCreated),
            PageLoadMs = at(Mark::PageLoad),
            FirstPaintMs = at(Mark::FirstPaint),
            WindowReadyMs = at(Mark::WindowReady),
            WorkspaceLoadedMs = at(Mark::WorkspaceLoaded),
            FirstInteractiveMs = at(Mark::FirstInteractive),
            Steps = steps.as_str();
            "Startup timings: {}",
            Mark::ALL.iter().skip(1).map(|m| format!("{} {} ms", m.name(), at(*m))).collect::<Vec<_>>().join(", ")
        );
    }
}

/// Records Rust mark `mark` now, when the app manages the timings.
pub fn mark<R: tauri::Runtime, M: tauri::Manager<R>>(app: &M, mark: Mark) {
    if let Some(t) = app.try_state::<StartupTimings>() {
        t.mark(mark);
    }
}

/// The main window's page reached `mark` at `atEpochMs` (wall clock). Marks of other windows and repeats are ignored.
#[tauri::command]
pub async fn startup_mark<R: tauri::Runtime>(
    webview: tauri::Webview<R>,
    timings: tauri::State<'_, StartupTimings>,
    mark: PageMark,
    at_epoch_ms: f64,
) -> crate::CmdResult<()> {
    if webview.label() == crate::windows::MAIN {
        timings.page_mark(mark, at_epoch_ms);
    }
    Ok(())
}

/// This start's timings so far.
#[tauri::command]
pub async fn startup_timings(timings: tauri::State<'_, StartupTimings>) -> crate::CmdResult<StartupReport> {
    Ok(timings.report())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Now as _;
    use tauri::Manager;

    fn full() -> Timeline {
        let mut t = Timeline::default();
        for (i, m) in Mark::ALL.iter().enumerate() {
            t.mark(*m, i as u64 * 100);
        }
        t
    }

    #[test]
    fn the_first_time_of_a_mark_counts() {
        let mut t = Timeline::default();
        assert!(t.mark(Mark::FirstPaint, 300));
        assert!(!t.mark(Mark::FirstPaint, 900));
        assert_eq!(t.at(Mark::FirstPaint), Some(300));
    }

    #[test]
    fn the_timeline_is_logged_once_when_complete() {
        let mut t = Timeline::default();
        for m in &Mark::ALL[..7] {
            t.mark(*m, 1);
            assert!(!t.take_log(), "logged before {m:?} was the last mark");
        }
        t.mark(Mark::FirstInteractive, 2);
        assert!(t.take_log());
        assert!(!t.take_log());
    }

    #[test]
    fn the_report_lists_marks_in_order_and_steps_as_they_ran() {
        let mut t = Timeline::default();
        t.mark(Mark::WindowReady, 500);
        t.mark(Mark::ProcessStart, 0);
        t.steps_from(10);
        t.step("logging", 25);
        t.step("ai", 80);
        let r = t.report();
        assert_eq!(
            r.marks.iter().map(|m| m.mark).collect::<Vec<_>>(),
            vec![Mark::ProcessStart, Mark::WindowReady]
        );
        assert_eq!(r.steps, vec![StepTook { step: "logging", ms: 15 }, StepTook { step: "ai", ms: 55 }]);
        assert!(!r.complete);
        assert_eq!(t.steps_text(), "ai=55 logging=15");
        assert!(full().report().complete);
    }

    #[test]
    fn page_times_count_from_the_process_start_and_impossible_ones_use_the_arrival() {
        let p = 1_700_000_000_000.0;
        assert_eq!(page_time(p + 420.0, p, 600), 420);
        assert_eq!(page_time(p - 200.0, p, 600), 0, "clock granularity before the start");
        assert_eq!(page_time(p - 5000.0, p, 600), 600, "before the process: the arrival");
        assert_eq!(page_time(p + 9000.0, p, 600), 600, "in the future: the arrival");
        assert_eq!(page_time(f64::NAN, p, 600), 600);
        assert_eq!(page_time(p + 900.0, p, 600), 600, "never after the arrival");
    }

    #[test]
    fn the_report_serializes_as_the_ui_expects() {
        let mut t = Timeline::default();
        t.mark(Mark::FirstInteractive, 7);
        t.steps_from(0);
        t.step("menu", 3);
        let v = serde_json::to_value(t.report()).unwrap();
        assert_eq!(
            v,
            serde_json::json!({ "marks": [{ "mark": "firstInteractive", "ms": 7 }], "steps": [{ "step": "menu", "ms": 3 }], "complete": false })
        );
        let m: PageMark = serde_json::from_value(serde_json::json!("workspaceLoaded")).unwrap();
        assert_eq!(Mark::from(m), Mark::WorkspaceLoaded);
        assert!(
            serde_json::from_value::<PageMark>(serde_json::json!("setupDone")).is_err(),
            "Rust's own marks are not the page's"
        );
    }

    #[test]
    fn only_the_main_windows_page_marks_count() {
        let app = crate::test_support::test_app(None);
        app.manage(StartupTimings::default());
        let other = crate::test_support::window(&app, "w2");
        let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs_f64() * 1000.0;
        startup_mark(other.as_ref().clone(), app.state(), PageMark::FirstPaint, now)
            .now()
            .unwrap();
        assert_eq!(
            startup_timings(app.state())
                .now()
                .unwrap()
                .marks
                .iter()
                .filter(|m| m.mark == Mark::FirstPaint)
                .count(),
            0
        );
        let main = crate::test_support::window(&app, crate::windows::MAIN);
        startup_mark(main.as_ref().clone(), app.state(), PageMark::FirstPaint, now)
            .now()
            .unwrap();
        let r = startup_timings(app.state()).now().unwrap();
        assert!(r.marks.iter().any(|m| m.mark == Mark::FirstPaint));
        assert!(r.marks.iter().any(|m| m.mark == Mark::ProcessStart && m.ms == 0));
    }
}
