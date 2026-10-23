//! The first-start "set up the assistant" flow as a background job. The set-up dialog only asks which model; this
//! starts what follows (the inference runtime when none is installed, then the model: download, check, load, test)
//! and keeps the user informed without a window: the Jobs pane and the status bar show the phases, and exactly one
//! message says how it ended ("AI assistant ready" with Open the assistant, or a plain failure with Retry).
//! Cancelling is the job's Cancel in the Jobs pane and says nothing.

use super::commands::{core, spawn_setup, SetupJob};
use super::error::{AiCmdError, AiErrorCode, AiResult};
use super::runtime_commands::start_install_chained;
use crate::notification_centre::centre::{NewNotice, NoticeAction, NoticeScope, NoticeSeverity, NoticeSource};
use crate::notify::NotificationKind;
use tauri::{AppHandle, Runtime};

/// The model a background set-up is for, as its job and messages name it.
#[derive(Debug, Clone)]
pub(crate) struct Assistant {
    pub model_id: String,
    pub name: String,
}

impl Assistant {
    /// Tells the user how the set-up ended: ready, or failed (a cancel is the user's own act and stays silent).
    pub(crate) fn finished<R: Runtime>(&self, app: &AppHandle<R>, result: &AiResult<()>) {
        match result {
            Ok(()) => ready(app, &self.name),
            Err(e) if e.code == AiErrorCode::Cancelled => {}
            Err(e) => failed(app, &self.model_id, &e.message),
        }
    }
}

fn ready<R: Runtime>(app: &AppHandle<R>, name: &str) {
    let notice = NewNotice::new(NoticeSource::Ai, NoticeSeverity::Success, "AI assistant ready", NoticeScope::App)
        .body(format!("{name} is set up and runs on this computer."))
        .primary(NoticeAction::OpenAssistant)
        .dedupe("ai-setup");
    crate::notification_centre::post(app, None, notice, Some(NotificationKind::JobFinished));
}

/// One plain message for a failed set-up, whichever step failed.
pub(crate) fn failed<R: Runtime>(app: &AppHandle<R>, model_id: &str, reason: &str) {
    let notice = NewNotice::new(
        NoticeSource::Ai,
        NoticeSeverity::Error,
        "The AI assistant could not be set up",
        NoticeScope::App,
    )
    .body(reason)
    .primary(NoticeAction::SetUpAssistant {
        model_id: model_id.to_string(),
    })
    .dedupe("ai-setup");
    crate::notification_centre::post(app, None, notice, Some(NotificationKind::JobFailed));
}

/// Starts the background set-up of `model_id` and returns its first job at once. The first job is the runtime install
/// when none is installed (the model job follows it), else the model job itself.
pub(crate) fn start<R: Runtime>(app: AppHandle<R>, model_id: String) -> AiResult<u64> {
    let core = core(&app)?;
    let name = core
        .catalog()
        .find(&model_id)
        .map(|m| m.name.clone())
        .ok_or_else(|| AiCmdError::not_found(format!("'{model_id}' is not in the model list")))?;
    let runtime = core.runtime_overview()?;
    let assistant = Assistant {
        model_id: model_id.clone(),
        name: name.clone(),
    };
    if runtime.needs_download {
        if let Some(build) = runtime.suggested.clone() {
            return start_install_chained(app, build, assistant);
        }
    }
    start_model(app, assistant)
}

/// The model step: download (when not installed), check, load and test.
pub(crate) fn start_model<R: Runtime>(app: AppHandle<R>, assistant: Assistant) -> AiResult<u64> {
    let core = core(&app)?;
    let file_name = core
        .catalog()
        .find(&assistant.model_id)
        .map(|m| m.file.clone())
        .ok_or_else(|| AiCmdError::not_found(format!("'{}' is not in the model list", assistant.model_id)))?;
    let id = assistant.model_id.clone();
    let job = SetupJob {
        title: format!("Setting up the AI assistant: {}", assistant.name),
        model_id: Some(id.clone()),
        file_name,
        assistant: Some(assistant),
    };
    spawn_setup(app, core, job, Box::new(move |c, p, on| c.run_setup(&id, p, on)))
}
