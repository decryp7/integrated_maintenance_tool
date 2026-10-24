//! Start-up work that the first window does not need (Paddock rules 10 and 16): starting the store worker and the
//! plugin processes ahead of the first query or import. It is registered during `setup` and runs once the main
//! window reported its first paint (`window_ready`), each task on its own background thread, so it never competes
//! with the window's start; if no paint is reported within [`FALLBACK`] (a start in the tray, a page that is reloaded
//! by the watchdog), it runs then.

use std::sync::Mutex;
use std::time::Duration;

/// The longest the work waits for the first paint.
pub const FALLBACK: Duration = Duration::from_secs(10);

type Task = Box<dyn FnOnce() + Send + 'static>;

/// The tasks waiting for the first paint (managed state).
pub struct AfterFirstPaint {
    /// `None` once they were started.
    tasks: Mutex<Option<Vec<(&'static str, Task)>>>,
    started: std::sync::atomic::AtomicBool,
}

impl AfterFirstPaint {
    /// Runs `task` (named for its thread and the log) after the first paint; at once when that has happened already.
    pub fn later(&self, name: &'static str, task: impl FnOnce() + Send + 'static) {
        let mut tasks = crate::sync::lock(&self.tasks);
        match tasks.as_mut() {
            Some(list) if !self.started() => list.push((name, Box::new(task))),
            _ => {
                drop(tasks);
                spawn(name, Box::new(task));
            }
        }
    }

    /// Whether the tasks were started.
    pub fn started(&self) -> bool {
        self.started.load(std::sync::atomic::Ordering::Acquire)
    }

    /// Starts every waiting task, each on its own background thread. Only the first call does anything.
    pub fn start(&self) {
        let tasks = {
            let mut tasks = crate::sync::lock(&self.tasks);
            if self.started.swap(true, std::sync::atomic::Ordering::AcqRel) {
                return;
            }
            tasks.take().unwrap_or_default()
        };
        for (name, task) in tasks {
            spawn(name, task);
        }
    }
}

impl Default for AfterFirstPaint {
    /// No tasks yet, none started.
    fn default() -> AfterFirstPaint {
        AfterFirstPaint {
            tasks: Mutex::new(Some(Vec::new())),
            started: Default::default(),
        }
    }
}

fn spawn(name: &'static str, task: Task) {
    if let Err(e) = std::thread::Builder::new().name(format!("after-paint-{name}")).spawn(task) {
        log::warn!("Could not start the background task {name}: {e}");
    }
}

/// Starts the waiting tasks (the main window painted).
pub fn first_paint<R: tauri::Runtime, M: tauri::Manager<R>>(app: &M) {
    if let Some(a) = app.try_state::<AfterFirstPaint>() {
        a.start();
    }
}

/// Starts the waiting tasks after [`FALLBACK`] when no paint was reported (a background thread that only waits).
pub fn arm_fallback<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    let app = app.clone();
    let _ = std::thread::Builder::new().name("after-paint-fallback".into()).spawn(move || {
        std::thread::sleep(FALLBACK);
        if let Some(a) = tauri::Manager::try_state::<AfterFirstPaint>(&app) {
            if !a.started() {
                log::info!(
                    "No first paint reported within {} s; starting the background start-up work",
                    FALLBACK.as_secs()
                );
                a.start();
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::mpsc;

    #[test]
    fn tasks_wait_for_the_first_paint_and_run_once_on_background_threads() {
        let a = AfterFirstPaint::default();
        let (tx, rx) = mpsc::channel();
        for name in ["one", "two"] {
            let tx = tx.clone();
            a.later(name, move || tx.send(std::thread::current().name().map(String::from)).unwrap());
        }
        assert!(rx.try_recv().is_err(), "nothing runs before the first paint");
        assert!(!a.started());
        a.start();
        a.start();
        let mut names: Vec<_> = (0..2).map(|_| rx.recv_timeout(Duration::from_secs(10)).unwrap().unwrap()).collect();
        names.sort();
        assert_eq!(names, vec!["after-paint-one", "after-paint-two"]);
        assert!(rx.recv_timeout(Duration::from_millis(50)).is_err(), "each task ran once");
    }

    #[test]
    fn a_task_registered_after_the_first_paint_runs_at_once() {
        let a = AfterFirstPaint::default();
        a.start();
        let (tx, rx) = mpsc::channel();
        a.later("late", move || tx.send(()).unwrap());
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
    }

    #[test]
    fn the_main_windows_ready_starts_the_work() {
        use crate::test_support::Now as _;
        use tauri::Manager;
        let app = crate::test_support::test_app(None);
        app.manage(AfterFirstPaint::default());
        app.manage(crate::startup_watchdog::StartupWatchdog::default());
        let ready = |label: &str| {
            let w = crate::test_support::window(&app, label);
            crate::startup_watchdog::window_ready(w.as_ref().clone(), app.state())
                .now()
                .unwrap();
        };
        let (tx, rx) = mpsc::channel();
        app.state::<AfterFirstPaint>().later("probe", move || tx.send(()).unwrap());
        ready("w2");
        assert!(
            rx.recv_timeout(Duration::from_millis(100)).is_err(),
            "another window's paint does not count"
        );
        ready(crate::windows::MAIN);
        rx.recv_timeout(Duration::from_secs(10)).unwrap();
    }
}
