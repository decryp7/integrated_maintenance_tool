//! Investigation helpers (ADR 0039): known-issue matching over the knowledge files, the incident summary report,
//! and investigation playbooks. Commands are thin: the engine (`imt_core::{known_issues, incident, playbook}`) computes,
//! the model (when there is one) only suggests and narrates, and every suggestion is verified by the engine.

pub(crate) mod incident;
pub(crate) mod issues;
pub(crate) mod playbooks;
#[cfg(test)]
mod tests;

use crate::windows::WindowState;
use imt_ai::knowledge::KnowledgeFile;

/// Name and text of the enabled knowledge files the window sees: its project files (and notes), then the global ones.
pub(crate) fn knowledge_texts(app: &tauri::AppHandle<impl tauri::Runtime>, win: &WindowState) -> Vec<(String, String)> {
    let mut files: Vec<KnowledgeFile> = crate::ai::knowledge::for_window(win, &Default::default());
    if let Ok(core) = crate::ai::commands::core(app) {
        files.extend(core.knowledge().snapshot().files().iter().cloned());
    }
    files.into_iter().filter(|f| f.enabled).map(|f| (f.name, f.text)).collect()
}

/// Epoch milliseconds now.
pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}
