//! Knowledge files (ADR 0029): Markdown notes the user writes about their systems (names, log sources, known errors
//! and fixes, environments) that every AI feature reads as background. Two scopes: the project's (saved with the
//! project) and the global ones (Debrief's configuration folder, stored by the shell).
//!
//! Pure: the validated file type and its collection ([`KnowledgeSet`]), the project record, assembling the prompt
//! block within limits ([`assemble`]) and the provider decorator that adds it to every request
//! ([`KnowledgeProvider`]). Knowledge text is the user's own, trusted text, but it is data in the prompt: it comes
//! after Debrief's rules and never replaces them.

mod assemble;
mod file;
mod provider;
mod record;

pub use assemble::{assemble, budget_chars, KnowledgeBlock, BLOCK_CLOSE, BLOCK_OPEN, FILE_CLOSE, FILE_OPEN};
pub use file::{check_id, check_name, check_text, KnowledgeChange, KnowledgeFile, KnowledgeInfo, KnowledgeScope, KnowledgeSet};
pub use provider::KnowledgeProvider;
pub use record::{KnowledgeRecord, KNOWLEDGE_RECORD_VERSION};

/// Largest knowledge file (bytes of UTF-8).
pub const MAX_FILE_BYTES: usize = 64 * 1024;
/// Most files in one scope.
pub const MAX_FILES: usize = 32;
/// Longest file name (characters).
pub const MAX_NAME_CHARS: usize = 80;
/// Most characters of knowledge in one prompt, whatever the model's context window.
pub const MAX_TOTAL_CHARS: usize = 96 * 1024;
