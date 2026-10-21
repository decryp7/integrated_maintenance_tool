//! Notes (ADR 0021): each window's notes ([`WindowNotes`], an `imt_notes::NoteSet` saved in the project's UI state
//! under `notes`, like the chats and the knowledge files), the images opened to mark them ([`images`]) and the
//! commands ([`commands`]). The notes reach the AI as one more project knowledge file ([`ai_file`]). Note text never
//! goes to IMT's log.

pub(crate) mod commands;
pub(crate) mod images;
#[cfg(test)]
mod tests;

use crate::error::{CmdError, ErrorCode};
use crate::sync::lock;
use imt_notes::context::{render, Focus, MAX_CONTEXT_BYTES};
use imt_notes::{NoteSet, NotesError};
use std::sync::{Arc, Mutex};

impl From<NotesError> for CmdError {
    fn from(e: NotesError) -> CmdError {
        let code = match &e {
            NotesError::Invalid(_) => ErrorCode::Invalid,
            NotesError::NotFound(_) => ErrorCode::NotFound,
            NotesError::Limit(_) => ErrorCode::Limit,
        };
        CmdError::new(code, e.to_string())
    }
}

/// A window's notes and the images it opened for them.
#[derive(Default)]
pub struct WindowNotes {
    set: Mutex<Arc<NoteSet>>,
    pub(crate) images: images::NoteImages,
}

impl WindowNotes {
    /// The notes as they are now (cheap: shared).
    pub fn snapshot(&self) -> Arc<NoteSet> {
        lock(&self.set).clone()
    }

    /// Applies `f` to a copy and keeps it when `f` succeeds.
    pub fn change<T>(&self, f: impl FnOnce(&mut NoteSet) -> imt_notes::Result<T>) -> imt_notes::Result<T> {
        let mut cur = lock(&self.set);
        let mut next = (**cur).clone();
        let out = f(&mut next)?;
        *cur = Arc::new(next);
        Ok(out)
    }

    pub fn replace(&self, set: NoteSet) {
        *lock(&self.set) = Arc::new(set);
    }
}

/// The name notes are signed with: the OS user (none when the system does not say).
pub(crate) fn author() -> String {
    std::env::var("USERNAME").or_else(|_| std::env::var("USER")).unwrap_or_default()
}

pub(crate) fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// The id of the knowledge file the notes are handed to the AI as (`k0` is never given to a user's file).
pub const NOTES_KNOWLEDGE_ID: &str = "k0";

/// The window's notes as one more project knowledge file for the AI (ADR 0021 §6): the ones about `focus` first,
/// at most [`MAX_CONTEXT_BYTES`]. None without notes.
pub fn ai_file(notes: &NoteSet, focus: &Focus) -> Option<imt_ai::knowledge::KnowledgeFile> {
    let text = render(notes.notes(), focus, MAX_CONTEXT_BYTES)?;
    imt_ai::knowledge::KnowledgeFile::new(NOTES_KNOWLEDGE_ID, "Notes", true, &text).ok()
}
