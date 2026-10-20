//! Background work the user may want to see or stop while IMT runs (also from
//! the tray): what keeps running until stopped (live tail per window, the
//! process monitor, PerfMon collection), plus the running jobs, which
//! [`crate::jobs`] mirrors here. Producers register a [`TaskSpec`] with an
//! optional stop action; listeners (tray, UI event) are told about changes.
//!
//! The registry is plain Rust (no Tauri) so it is unit-testable; `lib.rs`
//! manages one instance and wires the listeners. Commands:
//! `background_tasks`, `stop_background_task`. Event: `imt://background-tasks`.

use serde::Serialize;
use std::sync::{Arc, Mutex, MutexGuard};

/// Event carrying the current task list (`TaskInfo[]`) to every window.
pub const TASKS_EVENT: &str = "imt://background-tasks";
/// More tasks than this are refused (a runaway producer must not grow the tray menu forever).
const MAX_TASKS: usize = 64;
const MAX_LABEL: usize = 120;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum TaskKind {
    LiveTail,
    ProcessMonitor,
    Perfmon,
    /// A running job (see [`crate::jobs`]); its stop action cancels it.
    Job,
    Other,
}

/// What a producer registers.
#[derive(Debug, Clone)]
pub struct TaskSpec {
    /// Stable id, unique per task (`live:<window>`, `monitor`, `perfmon:<name>`, ...).
    pub id: String,
    pub label: String,
    pub kind: TaskKind,
    /// The window the task belongs to, if any.
    pub window: Option<String>,
}

/// A task as the UI and the tray see it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskInfo {
    pub id: String,
    pub label: String,
    pub kind: TaskKind,
    pub window: Option<String>,
    /// 0..=1 when known.
    pub progress: Option<f32>,
    pub stoppable: bool,
    /// Epoch milliseconds.
    pub started_at: i64,
}

/// Stops a task. Runs off the UI thread; may block briefly (joins a worker, runs `logman`).
pub type StopFn = Arc<dyn Fn() -> anyhow::Result<()> + Send + Sync>;
type Listener = Arc<dyn Fn(&[TaskInfo]) + Send + Sync>;

struct Entry {
    info: TaskInfo,
    stop: Option<StopFn>,
}

/// The registry. Managed as Tauri state; cheap to share behind `Arc`.
#[derive(Default)]
pub struct BackgroundTasks {
    entries: Mutex<Vec<Entry>>,
    listeners: Mutex<Vec<Listener>>,
}

impl BackgroundTasks {
    fn entries(&self) -> MutexGuard<'_, Vec<Entry>> {
        self.entries.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// Adds a task, or replaces the one with the same id (keeping its start time).
    pub fn register(&self, spec: TaskSpec, stop: Option<StopFn>) -> anyhow::Result<()> {
        if spec.id.is_empty() || spec.id.len() > MAX_LABEL {
            anyhow::bail!("invalid background task id");
        }
        {
            let mut entries = self.entries();
            let label: String = spec.label.chars().filter(|c| !c.is_control()).take(MAX_LABEL).collect();
            let existing = entries.iter().position(|e| e.info.id == spec.id);
            let started_at = existing.map_or_else(|| chrono::Utc::now().timestamp_millis(), |i| entries[i].info.started_at);
            let entry = Entry {
                info: TaskInfo {
                    id: spec.id,
                    label,
                    kind: spec.kind,
                    window: spec.window,
                    progress: None,
                    stoppable: stop.is_some(),
                    started_at,
                },
                stop,
            };
            match existing {
                Some(i) => entries[i] = entry,
                None if entries.len() >= MAX_TASKS => anyhow::bail!("too many background tasks"),
                None => entries.push(entry),
            }
        }
        self.changed();
        Ok(())
    }

    /// Reports progress (clamped to 0..=1; `None`: unknown). Unknown ids are ignored.
    #[allow(dead_code)] // used by downloads (the AI module) once installed
    pub fn set_progress(&self, id: &str, progress: Option<f32>) {
        let found = {
            let mut entries = self.entries();
            let entry = entries.iter_mut().find(|e| e.info.id == id);
            entry.map(|e| e.info.progress = progress.filter(|p| p.is_finite()).map(|p| p.clamp(0.0, 1.0)))
        };
        if found.is_some() {
            self.changed();
        }
    }

    /// Removes a task (it finished or was stopped elsewhere). Returns whether it existed.
    pub fn remove(&self, id: &str) -> bool {
        let removed = {
            let mut entries = self.entries();
            let before = entries.len();
            entries.retain(|e| e.info.id != id);
            entries.len() != before
        };
        if removed {
            self.changed();
        }
        removed
    }

    /// Removes every task of a window (it was closed).
    pub fn remove_window(&self, window: &str) {
        let removed = {
            let mut entries = self.entries();
            let before = entries.len();
            entries.retain(|e| e.info.window.as_deref() != Some(window));
            entries.len() != before
        };
        if removed {
            self.changed();
        }
    }

    pub fn list(&self) -> Vec<TaskInfo> {
        self.entries().iter().map(|e| e.info.clone()).collect()
    }

    #[cfg(test)]
    pub fn running(&self) -> usize {
        self.entries().len()
    }

    /// Runs a task's stop action and removes it when that succeeded. A panic in
    /// the action is contained and reported as an error.
    pub fn stop(&self, id: &str) -> anyhow::Result<()> {
        let stop = {
            let entries = self.entries();
            let entry = entries
                .iter()
                .find(|e| e.info.id == id)
                .ok_or_else(|| anyhow::anyhow!("this background task is no longer running"))?;
            entry
                .stop
                .clone()
                .ok_or_else(|| anyhow::anyhow!("\"{}\" cannot be stopped", entry.info.label))?
        };
        // Outside the lock: the action may call back into the registry (e.g. `remove`).
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| stop())) {
            Ok(Ok(())) => {
                self.remove(id);
                Ok(())
            }
            Ok(Err(e)) => {
                log::error!(Task = id; "Background task could not be stopped: {e:#}");
                Err(e)
            }
            Err(p) => {
                let message = imt_core::guard::panic_message(&*p);
                log::error!(Task = id; "Stopping a background task crashed: {message}");
                Err(anyhow::anyhow!("stopping failed: {message}"))
            }
        }
    }

    /// Calls `f` with the full list after every change.
    pub fn subscribe(&self, f: impl Fn(&[TaskInfo]) + Send + Sync + 'static) {
        self.listeners.lock().unwrap_or_else(|p| p.into_inner()).push(Arc::new(f));
    }

    fn changed(&self) {
        let list = self.list();
        let listeners = self.listeners.lock().unwrap_or_else(|p| p.into_inner()).clone();
        for l in listeners {
            // A failing listener (e.g. the tray is gone) must not break the producer.
            if let Err(p) = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| l(&list))) {
                log::error!("A background task listener crashed: {}", imt_core::guard::panic_message(&*p));
            }
        }
    }
}

/// The running background work, for the status bar.
#[tauri::command]
pub async fn background_tasks(tasks: tauri::State<'_, BackgroundTasks>) -> crate::CmdResult<Vec<TaskInfo>> {
    Ok(tasks.list())
}

/// Stops a background task (off the UI thread: stopping may join a worker or run `logman`).
#[tauri::command]
pub async fn stop_background_task<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: String) -> crate::CmdResult<()> {
    crate::error::lift(
        tauri::async_runtime::spawn_blocking(move || {
            let tasks = tauri::Manager::try_state::<BackgroundTasks>(&app).ok_or("IMT is not ready yet")?;
            tasks.stop(&id).map_err(crate::err)
        })
        .await
        .map_err(crate::err)?,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn spec(id: &str, window: Option<&str>) -> TaskSpec {
        TaskSpec {
            id: id.into(),
            label: format!("Task {id}\n"),
            kind: TaskKind::LiveTail,
            window: window.map(String::from),
        }
    }

    #[test]
    fn register_list_progress_remove() {
        let tasks = BackgroundTasks::default();
        let seen = Arc::new(AtomicUsize::new(0));
        let s = seen.clone();
        tasks.subscribe(move |list| s.store(list.len() + 100, Ordering::SeqCst));
        tasks.register(spec("a", Some("main")), None).unwrap();
        assert_eq!(seen.load(Ordering::SeqCst), 101);
        let started = tasks.list()[0].started_at;
        assert_eq!(tasks.list()[0].label, "Task a", "control characters are dropped");
        assert!(!tasks.list()[0].stoppable);
        // Replacing keeps the start time and the position.
        tasks.register(spec("b", Some("w-2")), None).unwrap();
        tasks.register(spec("a", Some("main")), Some(Arc::new(|| Ok(())))).unwrap();
        assert_eq!(tasks.list()[0].started_at, started);
        assert!(tasks.list()[0].stoppable);
        assert_eq!(tasks.running(), 2);
        tasks.set_progress("a", Some(1.7));
        assert_eq!(tasks.list()[0].progress, Some(1.0));
        tasks.set_progress("a", Some(f32::NAN));
        assert_eq!(tasks.list()[0].progress, None);
        tasks.set_progress("nope", Some(0.5));
        assert!(tasks.remove("a"));
        assert!(!tasks.remove("a"));
        tasks.remove_window("w-2");
        tasks.remove_window("w-2");
        assert_eq!(tasks.running(), 0);
        assert_eq!(seen.load(Ordering::SeqCst), 100);
        assert!(tasks.register(spec("", None), None).is_err());
    }

    #[test]
    fn stop_runs_the_action_and_contains_failures() {
        let _g = crate::activity::test_capture();
        let tasks = Arc::new(BackgroundTasks::default());
        let calls = Arc::new(AtomicUsize::new(0));
        let c = calls.clone();
        let t = tasks.clone();
        // The action may call back into the registry without deadlocking.
        tasks
            .register(
                spec("ok", None),
                Some(Arc::new(move || {
                    c.fetch_add(1, Ordering::SeqCst);
                    t.list();
                    Ok(())
                })),
            )
            .unwrap();
        tasks.stop("ok").unwrap();
        assert_eq!((calls.load(Ordering::SeqCst), tasks.running()), (1, 0));
        assert!(tasks.stop("ok").unwrap_err().to_string().contains("no longer running"));

        tasks.register(spec("plain", None), None).unwrap();
        assert!(tasks.stop("plain").unwrap_err().to_string().contains("cannot be stopped"));
        tasks
            .register(spec("err", None), Some(Arc::new(|| Err(anyhow::anyhow!("locked")))))
            .unwrap();
        assert_eq!(tasks.stop("err").unwrap_err().to_string(), "locked");
        tasks.register(spec("boom", None), Some(Arc::new(|| panic!("kaboom")))).unwrap();
        assert!(tasks.stop("boom").unwrap_err().to_string().contains("kaboom"));
        // Failed stops keep the task listed, and are logged.
        assert_eq!(tasks.running(), 3);
        let logged = crate::activity::captured();
        let failed = logged
            .iter()
            .find(|c| c.message.contains("could not be stopped: locked"))
            .expect("stop error logged");
        assert_eq!((failed.level, failed.props["Task"].as_str()), (log::Level::Error, "err"));
        let crashed = logged
            .iter()
            .find(|c| c.message.contains("crashed: kaboom"))
            .expect("stop crash logged");
        assert_eq!(crashed.props["Task"], "boom");
    }

    #[test]
    fn listeners_that_panic_do_not_break_producers_and_the_size_is_capped() {
        let tasks = BackgroundTasks::default();
        let _g = crate::activity::test_capture();
        tasks.subscribe(|_| panic!("listener bug"));
        for i in 0..MAX_TASKS {
            tasks.register(spec(&format!("t{i}"), None), None).unwrap();
        }
        // Logged once, then deduplicated (the activity logger counts the repeats).
        let logged = crate::activity::captured();
        assert_eq!(
            logged
                .iter()
                .filter(|c| c.message.contains("listener crashed: listener bug"))
                .count(),
            1
        );
        assert!(tasks.register(spec("one-too-many", None), None).is_err());
        // Replacing an existing one is still fine at the cap.
        tasks.register(spec("t0", None), None).unwrap();
    }
}
