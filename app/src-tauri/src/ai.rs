//! The local AI assistant (`imt-ai`): set-up and models, the use cases, chat, and the Settings → AI section.
//! Inference runs in one `imt-ai-worker` process shared by every window (a downloaded llama.cpp
//! `llama-server` build, ADR 0009; never a cloud service); each window has its own chats.
//!
//! Everything is optional: in safe mode, without the worker binary or before a model is set up, every
//! command fails with a typed [`AiCmdError`] (`code` = `unavailable` / `noModel`) and the UI hides or greys
//! its AI features.
//!
//! Events: `imt://ai-setup` (set-up and activation stages, all windows), `imt://ai-status` (engine state
//! changed, all windows), `imt://ai-chat` (chat stream, to the window of the chat), `imt://ai-runtime`
//! (inference runtime downloads, all windows).

mod chats;
pub(crate) mod commands;
mod core;
pub(crate) mod error;
pub(crate) mod folder_setup;
pub(crate) mod manual;
pub(crate) mod requests;
pub(crate) mod runtime_commands;
mod runtimes;
mod settings_section;
#[cfg(test)]
mod tests;

pub use self::core::{AiCore, AiDeps};
pub use chats::WindowChats;
pub use settings_section::AiSettingsSection;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, Runtime};

pub const SETUP_EVENT: &str = "imt://ai-setup";
pub const STATUS_EVENT: &str = "imt://ai-status";
pub const CHAT_EVENT: &str = "imt://ai-chat";
/// Inference runtime installs (`runtimes::RuntimeEvent`, all windows).
pub const RUNTIME_EVENT: &str = "imt://ai-runtime";

/// The app-wide AI runtime, managed as Tauri state.
pub struct AiState(pub Arc<AiCore>);

/// A window's AI state: its chats and the use cases it is running (to cancel them).
#[derive(Default)]
pub struct WindowAi {
    pub chats: WindowChats,
    ops: Mutex<Vec<Arc<AtomicBool>>>,
}

impl WindowAi {
    /// Registers the cancel flag of a running AI job, so `ai_cancel` reaches it too.
    pub fn track_op(&self, flag: Arc<AtomicBool>) {
        self.ops.lock().unwrap_or_else(|p| p.into_inner()).push(flag);
    }

    pub fn end_op(&self, flag: &Arc<AtomicBool>) {
        self.ops.lock().unwrap_or_else(|p| p.into_inner()).retain(|f| !Arc::ptr_eq(f, flag));
    }

    /// Cancels every running use case of the window; returns how many there were.
    pub fn cancel_ops(&self) -> usize {
        let ops = self.ops.lock().unwrap_or_else(|p| p.into_inner());
        for f in ops.iter() {
            f.store(true, Ordering::Relaxed);
        }
        ops.len()
    }
}

impl Drop for WindowAi {
    fn drop(&mut self) {
        self.cancel_ops();
        self.chats.cancel_all();
    }
}

/// Creates the AI runtime and registers Settings → AI. Call from `setup` after `settings::init`.
/// Never fails: problems leave the AI unavailable with a reason the UI shows.
pub fn init<R: Runtime>(app: &AppHandle<R>, safe_mode: bool) {
    let dir = crate::app_dirs::config(app).ok().map(|d| d.join("ai"));
    let data = crate::app_dirs::data(app).unwrap_or_else(|_| std::env::temp_dir().join("imt-data"));
    let models = data.join("models");
    let deps = AiDeps::system(imt_ai::worker::default_worker_path(), &data);
    let core = Arc::new(AiCore::new(dir, models, safe_mode, deps));
    manage(app, core.clone());
    core.autoload();
    match core.status() {
        s if s.available => log::info!(State = format!("{:?}", s.state).as_str(); "AI assistant ready to use"),
        s => log::info!("AI assistant unavailable: {}", s.reason.unwrap_or_default()),
    }
}

/// Manages `core` and registers its settings section (separate from [`init`] so tests can pass fakes).
pub fn manage<R: Runtime>(app: &AppHandle<R>, core: Arc<AiCore>) {
    app.manage(AiState(core.clone()));
    if let Some(jobs) = app.try_state::<crate::jobs::Jobs>() {
        // A quarantined worker pauses the AI queue (the waiting requests stay) until "Reset AI".
        let c = core.clone();
        jobs.set_gate(crate::jobs::JobKind::Ai, move || c.quarantine_reason());
        let handle = app.clone();
        jobs.set_retry(crate::jobs::JobKind::Ai, move |job| requests::retry(&handle, job));
        let handle = app.clone();
        jobs.set_retry(crate::jobs::JobKind::Download, move |job| commands::retry_download(&handle, job));
    }
    if let Err(e) = crate::settings::register(app, Arc::new(AiSettingsSection { core }), crate::settings::SectionGroup::Ai) {
        log::warn!("AI settings section not registered: {e:#}");
    }
}
