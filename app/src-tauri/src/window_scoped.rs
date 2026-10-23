//! Features that keep state per window register here (ADR 0016) instead of being listed by hand in the
//! window teardown: each [`Scope`] says what to do when a window is created and when it is destroyed.
//! Teardown is one loop, every hook off the main thread and bounded ([`crate::lifecycle::teardown_step`]),
//! so a stuck feature cannot hold a closing window (or Debrief) open, and a new feature cannot forget to
//! clean up after itself.
//!
//! Window operations themselves (closing a window's pop-outs and browser) stay in
//! [`crate::window_manager::destroyed`]: they run on the main thread before the state hooks.

use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Manager, Runtime};

type Hook<R> = Arc<dyn Fn(&AppHandle<R>, &str) + Send + Sync>;

/// One feature's per-window hooks.
pub struct Scope<R: Runtime> {
    /// For logs (the teardown step's name).
    pub name: &'static str,
    pub on_created: Option<Hook<R>>,
    pub on_destroyed: Hook<R>,
}

impl<R: Runtime> Scope<R> {
    /// A scope with only a teardown.
    pub fn on_destroyed(name: &'static str, f: impl Fn(&AppHandle<R>, &str) + Send + Sync + 'static) -> Scope<R> {
        Scope {
            name,
            on_created: None,
            on_destroyed: Arc::new(f),
        }
    }

    /// The same scope, also told when a window is created. (No shell feature needs it yet: their state is
    /// created on first use; plugins and the coming engine views will.)
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn with_created(mut self, f: impl Fn(&AppHandle<R>, &str) + Send + Sync + 'static) -> Scope<R> {
        self.on_created = Some(Arc::new(f));
        self
    }
}

/// The registered scopes (managed state).
pub struct WindowScopes<R: Runtime>(RwLock<Vec<Arc<Scope<R>>>>);

impl<R: Runtime> Default for WindowScopes<R> {
    fn default() -> Self {
        WindowScopes(RwLock::new(Vec::new()))
    }
}

impl<R: Runtime> WindowScopes<R> {
    fn list(&self) -> Vec<Arc<Scope<R>>> {
        crate::sync::read(&self.0).clone()
    }
}

fn scopes<R: Runtime>(app: &AppHandle<R>) -> tauri::State<'_, WindowScopes<R>> {
    if app.try_state::<WindowScopes<R>>().is_none() {
        app.manage(WindowScopes::<R>::default());
    }
    app.state::<WindowScopes<R>>()
}

/// Registers a feature's per-window hooks.
pub fn register<R: Runtime>(app: &AppHandle<R>, scope: Scope<R>) {
    crate::sync::write(&scopes(app).0).push(Arc::new(scope));
}

/// A window was created: every scope's `on_created`.
pub fn created<R: Runtime>(app: &AppHandle<R>, label: &str) {
    for s in scopes(app).list() {
        if let Some(f) = &s.on_created {
            f(app, label);
        }
    }
}

/// A window is gone: every scope's `on_destroyed`, each off the main thread and bounded.
pub fn destroyed<R: Runtime>(app: &AppHandle<R>, label: &str) {
    use crate::lifecycle::{teardown_step, TEARDOWN_STEP_LIMIT};
    for s in scopes(app).list() {
        let (handle, owner, scope) = (app.clone(), label.to_string(), s.clone());
        teardown_step(s.name, TEARDOWN_STEP_LIMIT, move || (scope.on_destroyed)(&handle, &owner));
    }
}

/// The shell's own per-window features: background tasks, jobs, notification-centre messages, opened
/// diagnostics reports, path grants. Call once when the app is built (and in tests).
pub fn install<R: Runtime>(app: &AppHandle<R>) {
    register(
        app,
        Scope::on_destroyed("background tasks", |app, label| {
            if let Some(tasks) = app.try_state::<crate::background::BackgroundTasks>() {
                tasks.remove_window(label);
            }
        }),
    );
    register(
        app,
        Scope::on_destroyed("watched folders", |app, label| {
            if let Ok(win) = crate::windows::CurrentWindow::of(app, label) {
                win.watched.stop();
            }
        }),
    );
    register(app, Scope::on_destroyed("jobs", crate::jobs::forget_window));
    register(app, Scope::on_destroyed("notifications", crate::notification_centre::forget_window));
    register(
        app,
        Scope::on_destroyed("opened reports", crate::activity::report::open::forget_window),
    );
    register(
        app,
        Scope::on_destroyed("search everything", crate::search_everything::forget_window),
    );
    register(
        app,
        Scope::on_destroyed("path grants", |app, label| crate::paths::grants(app).forget(label)),
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    #[test]
    fn scopes_hear_creation_and_destruction_and_a_stuck_one_is_bounded() {
        let app = crate::test_support::test_app(None);
        let seen = Arc::new(Mutex::new(Vec::new()));
        let (a, b) = (seen.clone(), seen.clone());
        register(
            app.handle(),
            Scope::on_destroyed("test", move |_, l| a.lock().unwrap().push(format!("destroyed {l}")))
                .with_created(move |_, l| b.lock().unwrap().push(format!("created {l}"))),
        );
        created(app.handle(), "w-5");
        destroyed(app.handle(), "w-5");
        assert_eq!(*seen.lock().unwrap(), ["created w-5", "destroyed w-5"]);
        // The shell's own scopes are registered with the app (test_support installs them).
        let names: Vec<&str> = scopes(app.handle()).list().iter().map(|s| s.name).collect();
        for n in [
            "background tasks",
            "jobs",
            "notifications",
            "opened reports",
            "path grants",
            "search everything",
        ] {
            assert!(names.contains(&n), "{n} not registered: {names:?}");
        }
    }
}
