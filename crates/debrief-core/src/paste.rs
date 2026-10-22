//! Pasted text as a source (Edit ▸ Paste as log, dropped text): the pure part. The text is checked against a size cap
//! and named; the shell writes the bytes into the project's `pasted/` folder, where the normal importer detection reads
//! them like any file. A name is made safe for every file system (no path, no reserved characters, bounded length) and
//! unique among the files already there.

use serde::{Deserialize, Serialize};
use std::collections::HashSet;

/// Largest pasted text (bytes). Bigger logs are opened as files.
pub const MAX_PASTE_BYTES: usize = 8 << 20;
/// Longest file name kept, in characters (extension included).
pub const MAX_NAME_CHARS: usize = 80;
/// The name used when none is given.
pub const DEFAULT_NAME: &str = "pasted-log";
/// The folder inside the project (store) folder that holds pasted sources.
pub const PASTE_DIR: &str = "pasted";

/// Why a paste was refused; the message says what to do.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PasteError {
    #[error("there is no text to paste")]
    Empty,
    #[error("the pasted text is {size} bytes; at most {max} bytes can be pasted as a log (save it to a file and open the file instead)")]
    TooLarge { size: usize, max: usize },
}

/// A checked paste: the file name to write and its bytes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PastedLog {
    pub file_name: String,
    pub bytes: Vec<u8>,
}

/// Extensions a pasted name may carry; any other gets `.log`.
const KNOWN_EXTENSIONS: &[&str] = &["log", "txt", "json", "jsonl", "ndjson", "clef", "csv", "xml", "out", "err"];

/// A file name derived from what the user typed: no folders, only letters, digits and `._- ()`, a known extension.
pub fn safe_name(requested: &str) -> String {
    let leaf = requested.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = leaf
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | ' ' | '(' | ')') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches(|c: char| c == '.' || c == ' ' || c == '-').to_string();
    let (stem, ext) = match cleaned.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() && KNOWN_EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()) => {
            (s.to_string(), e.to_ascii_lowercase())
        }
        _ => (cleaned.clone(), "log".to_string()),
    };
    let stem = if stem.trim().is_empty() { DEFAULT_NAME.to_string() } else { stem };
    let room = MAX_NAME_CHARS.saturating_sub(ext.chars().count() + 1);
    let stem: String = stem.chars().take(room).collect();
    format!("{}.{ext}", stem.trim_end())
}

/// `name`, or `name (2)`, `name (3)`... when `taken` (compared ignoring case, as common file systems do) has it.
pub fn unique_name(name: &str, taken: &HashSet<String>) -> String {
    let lower: HashSet<String> = taken.iter().map(|t| t.to_lowercase()).collect();
    if !lower.contains(&name.to_lowercase()) {
        return name.to_string();
    }
    let (stem, ext) = name.rsplit_once('.').unwrap_or((name, "log"));
    (2..)
        .map(|n| format!("{stem} ({n}).{ext}"))
        .find(|candidate| !lower.contains(&candidate.to_lowercase()))
        .unwrap_or_else(|| name.to_string())
}

/// Checks pasted `text` and names it. NUL characters are dropped (they would make the file look binary), line ends
/// are kept as pasted and the text ends with a newline.
pub fn prepare(name: Option<&str>, text: &str, taken: &HashSet<String>) -> Result<PastedLog, PasteError> {
    if text.trim().is_empty() {
        return Err(PasteError::Empty);
    }
    if text.len() > MAX_PASTE_BYTES {
        return Err(PasteError::TooLarge {
            size: text.len(),
            max: MAX_PASTE_BYTES,
        });
    }
    let mut body: String = text.chars().filter(|c| *c != '\0').collect();
    if !body.ends_with('\n') {
        body.push('\n');
    }
    let file_name = unique_name(&safe_name(name.unwrap_or(DEFAULT_NAME)), taken);
    Ok(PastedLog {
        file_name,
        bytes: body.into_bytes(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn none() -> HashSet<String> {
        HashSet::new()
    }

    #[test]
    fn names_are_made_safe_and_get_a_known_extension() {
        assert_eq!(safe_name("web01 errors"), "web01 errors.log");
        assert_eq!(safe_name("app.JSON"), "app.json");
        assert_eq!(safe_name("../../etc/passwd"), "passwd.log");
        assert_eq!(safe_name("C:\\Users\\me\\a:b*c?.txt"), "a-b-c-.txt");
        assert_eq!(safe_name("   "), "pasted-log.log");
        assert_eq!(safe_name(""), "pasted-log.log");
        assert_eq!(safe_name("..."), "pasted-log.log");
        assert_eq!(safe_name("notes.docx"), "notes.docx.log");
        assert_eq!(safe_name(".log"), "log.log");
        let long = safe_name(&"x".repeat(500));
        assert_eq!(long.chars().count(), MAX_NAME_CHARS);
        assert!(long.ends_with(".log"));
    }

    #[test]
    fn names_stay_unique_ignoring_case() {
        let taken: HashSet<String> = ["a.log".to_string(), "A (2).log".to_string()].into();
        assert_eq!(unique_name("b.log", &taken), "b.log");
        assert_eq!(unique_name("A.LOG", &taken), "A (3).LOG");
    }

    #[test]
    fn a_paste_is_checked_and_normalised() {
        assert_eq!(prepare(None, " \n\t", &none()), Err(PasteError::Empty));
        let p = prepare(Some("x"), "one\0\ntwo", &none()).unwrap();
        assert_eq!((p.file_name.as_str(), p.bytes.as_slice()), ("x.log", b"one\ntwo\n".as_slice()));
        let big = "a".repeat(MAX_PASTE_BYTES + 1);
        let err = prepare(None, &big, &none()).unwrap_err();
        assert!(matches!(err, PasteError::TooLarge { .. }));
        assert!(err.to_string().contains("open the file"));
        assert_eq!(
            prepare(None, &"a".repeat(MAX_PASTE_BYTES), &none()).unwrap().bytes.len(),
            MAX_PASTE_BYTES + 1
        );
    }
}
