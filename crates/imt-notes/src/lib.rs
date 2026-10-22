//! Notes (ADR 0021): the user's Markdown observations, each attached to an [`Anchor`]: a log event, an instant or a
//! range of time, a source, a marker or span, a chart point, the project, or regions of an image.
//!
//! Pure: the checked types ([`Note`], [`NoteDraft`], [`NoteChange`]), the window's collection ([`NoteSet`]), the
//! versioned project record ([`NoteRecord`], read leniently) and the block of notes the AI reads ([`context`]).
//! The shell keeps a set per window, fills event anchors from the store and stores image copies; nothing here does
//! I/O. Every value from the webview or a saved project goes through a `check` before it is kept.

mod anchor;
pub mod context;
mod error;
mod note;
mod record;
mod set;

pub use anchor::{Anchor, ImageRef, Region};
pub use error::{NotesError, Result};
pub use note::{check_tags, check_text, Note, NoteChange, NoteDraft, NoteSeverity};
pub use record::{NoteRecord, NOTE_RECORD_VERSION};
pub use set::NoteSet;

/// Largest note text (bytes of UTF-8 Markdown).
pub const MAX_TEXT_BYTES: usize = 16 * 1024;
/// Most notes in one project.
pub const MAX_NOTES: usize = 5_000;
/// Most text of all notes of a project together (bytes), so a project's record stays small.
pub const MAX_TOTAL_TEXT_BYTES: usize = 4 * 1024 * 1024;
/// Most tags on one note.
pub const MAX_TAGS: usize = 8;
/// Longest tag (characters).
pub const MAX_TAG_CHARS: usize = 32;
/// Most regions marked on one image note.
pub const MAX_REGIONS: usize = 16;
/// Longest label kept in an anchor (an excerpt, a source, marker, chart or image name).
pub const MAX_LABEL_CHARS: usize = 200;
/// Longest path kept in an anchor.
pub const MAX_PATH_CHARS: usize = 4_096;
/// Longest side of an image a note can mark (the decoder's own limit).
pub const MAX_IMAGE_SIDE: u32 = 16_384;
/// Longest author name kept.
pub const MAX_AUTHOR_CHARS: usize = 64;

/// The content key of an image: the lower-case hex SHA-256 of its file's bytes. A note names its image by it, so
/// the copy kept in the project and the original are the same picture.
pub fn image_key(bytes: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Whether `key` looks like an [`image_key`] (it names a file in the project folder, so nothing else is accepted).
pub fn is_image_key(key: &str) -> bool {
    key.len() == 64 && key.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// `s` without control characters, trimmed and cut to `max` characters.
pub(crate) fn label(s: &str, max: usize) -> String {
    let clean: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    let t = clean.trim();
    match t.char_indices().nth(max) {
        Some((i, _)) => format!("{}…", t[..i].trim_end()),
        None => t.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn image_keys_are_sha256_hex() {
        let k = image_key(b"abc");
        assert_eq!(k, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert!(is_image_key(&k));
        assert!(!is_image_key("../etc/passwd"));
        assert!(!is_image_key(&k.to_uppercase()));
        assert!(!is_image_key(&k[..63]));
    }

    #[test]
    fn labels_lose_control_characters_and_are_cut() {
        assert_eq!(label("  a\nb\tc  ", 10), "a b c");
        assert_eq!(label("abcdef", 3), "abc…");
        assert_eq!(label("", 3), "");
    }
}
