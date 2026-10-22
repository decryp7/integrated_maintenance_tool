//! Knowledge files (ADR 0029): the global ones ([`GlobalKnowledge`], kept by [`super::AiCore`]) and the window's
//! project ones ([`ProjectKnowledge`], saved in the project's UI state under `knowledge`, like the chats). The
//! assistant of every AI feature reads both (`AiCore::assistant`). Knowledge text never goes to IMT's log.

pub(crate) mod commands;
mod global;

pub use global::GlobalKnowledge;

use crate::sync::lock;
use imt_ai::knowledge::{KnowledgeFile, KnowledgeSet};
use std::sync::{Arc, Mutex};
use tauri::{AppHandle, Manager, Runtime};

/// A window's project knowledge files.
#[derive(Default)]
pub struct ProjectKnowledge {
    set: Mutex<Arc<KnowledgeSet>>,
}

impl ProjectKnowledge {
    /// The files as they are now (cheap: shared).
    pub fn snapshot(&self) -> Arc<KnowledgeSet> {
        lock(&self.set).clone()
    }

    /// Applies `f` to a copy and keeps it when `f` succeeds.
    pub fn change<T>(&self, f: impl FnOnce(&mut KnowledgeSet) -> imt_ai::Result<T>) -> imt_ai::Result<T> {
        let mut cur = lock(&self.set);
        let mut next = (**cur).clone();
        let out = f(&mut next)?;
        *cur = Arc::new(next);
        Ok(out)
    }

    pub fn replace(&self, set: KnowledgeSet) {
        *lock(&self.set) = Arc::new(set);
    }
}

/// The project knowledge files the AI reads in window `win`: the user's, then the window's notes (ADR 0021) as one
/// more file, the notes about `focus` first.
pub fn for_window(win: &crate::windows::WindowState, focus: &imt_notes::context::Focus) -> Vec<KnowledgeFile> {
    let mut files = win.ai.knowledge.snapshot().files().to_vec();
    files.extend(crate::notes::ai_file(&win.notes.snapshot(), focus));
    files
}

/// What a chat turn looks at, for ordering the notes: the selected events and the time range.
pub fn chat_focus(ctx: &imt_ai::chat::ChatContext) -> imt_notes::context::Focus {
    imt_notes::context::Focus {
        events: ctx.selected_ids.iter().map(|&id| u64::from(id)).collect(),
        range: ctx.time_range,
        image: None,
    }
}

/// The project knowledge files of the window `label` for the AI (none when it is closed).
pub fn project_of<R: Runtime>(app: &AppHandle<R>, label: &str) -> Vec<KnowledgeFile> {
    app.try_state::<crate::windows::Windows>()
        .and_then(|w| w.get(label))
        .map(|w| for_window(&w, &imt_notes::context::Focus::default()))
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_failed_change_leaves_the_project_knowledge_as_it_was() {
        let p = ProjectKnowledge::default();
        p.change(|s| s.add("Glossary", "x").map(|_| ())).unwrap();
        let before = p.snapshot();
        assert!(p.change(|s| s.add("", "y").map(|_| ())).is_err());
        assert_eq!(p.snapshot(), before);
        p.replace(KnowledgeSet::default());
        assert!(p.snapshot().files().is_empty());
    }
}
