//! One note and the requests that make or change one.

use crate::anchor::{check_regions, Anchor, Region};
use crate::error::{invalid, NotesError, Result};
use crate::{label, MAX_AUTHOR_CHARS, MAX_TAGS, MAX_TAG_CHARS, MAX_TEXT_BYTES};
use serde::{Deserialize, Serialize};
#[cfg(feature = "ts")]
use ts_rs::TS;

/// How a note reads (its colour).
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoteSeverity {
    #[default]
    Info,
    Warning,
    Problem,
}

impl NoteSeverity {
    pub fn label(self) -> &'static str {
        match self {
            NoteSeverity::Info => "info",
            NoteSeverity::Warning => "warning",
            NoteSeverity::Problem => "problem",
        }
    }
}

/// A note as kept: id (`n1`, `n2`, ...), anchor, Markdown text, tags, severity, author and times (epoch ms).
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Note {
    pub id: String,
    pub anchor: Anchor,
    pub text: String,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default)]
    pub severity: NoteSeverity,
    #[serde(default)]
    pub author: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub created_ms: i64,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub updated_ms: i64,
}

impl Note {
    /// The note as kept: every field checked (a note read from a saved project goes through this too).
    pub fn check(&self) -> Result<Note> {
        Ok(Note {
            id: check_id(&self.id)?.to_string(),
            anchor: self.anchor.check()?,
            text: check_text(&self.text)?,
            tags: check_tags(&self.tags)?,
            severity: self.severity,
            author: label(&self.author, MAX_AUTHOR_CHARS),
            created_ms: self.created_ms,
            updated_ms: self.updated_ms.max(self.created_ms),
        })
    }
}

/// A new note from the UI: the engine sets its id, author and times (and fills event and image anchors).
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteDraft {
    pub anchor: Anchor,
    pub text: String,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub severity: Option<NoteSeverity>,
}

/// A change to a note: only the fields given change. `regions` applies to image notes only.
#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct NoteChange {
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub text: Option<String>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub tags: Option<Vec<String>>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub severity: Option<NoteSeverity>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(optional))]
    pub regions: Option<Vec<Region>>,
}

impl NoteChange {
    /// Applies the change to `note` (checked before anything changes).
    pub(crate) fn apply(&self, note: &mut Note, now_ms: i64) -> Result<()> {
        let text = self.text.as_deref().map(check_text).transpose()?;
        let tags = self.tags.as_deref().map(check_tags).transpose()?;
        let regions = match (&self.regions, &note.anchor) {
            (None, _) => None,
            (Some(r), Anchor::Image { image, .. }) => Some(check_regions(r, image.width, image.height)?),
            (Some(_), _) => return Err(invalid("only notes on images have areas")),
        };
        if let Some(t) = text {
            note.text = t;
        }
        if let Some(t) = tags {
            note.tags = t;
        }
        if let Some(s) = self.severity {
            note.severity = s;
        }
        if let (Some(r), Anchor::Image { regions, .. }) = (regions, &mut note.anchor) {
            *regions = r;
        }
        note.updated_ms = now_ms.max(note.created_ms);
        Ok(())
    }
}

/// An id: `n` and 1 to 7 digits.
pub(crate) fn check_id(id: &str) -> Result<&str> {
    let digits = id.strip_prefix('n').unwrap_or("");
    if digits.is_empty() || digits.len() > 7 || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return Err(NotesError::NotFound("this note no longer exists".into()));
    }
    Ok(id)
}

/// Text: not blank, at most [`MAX_TEXT_BYTES`], no NUL; line endings become `\n`.
pub fn check_text(text: &str) -> Result<String> {
    if text.len() > MAX_TEXT_BYTES {
        return Err(NotesError::Limit(format!(
            "the note is too long ({} KiB; at most {} KiB)",
            text.len().div_ceil(1024),
            MAX_TEXT_BYTES / 1024
        )));
    }
    if text.contains('\0') {
        return Err(invalid("the note must not contain NUL characters"));
    }
    let t = text.replace("\r\n", "\n").replace('\r', "\n");
    if t.trim().is_empty() {
        return Err(invalid("write something in the note"));
    }
    Ok(t.trim_end().to_string())
}

/// Tags: trimmed, lower case, 1 to [`MAX_TAG_CHARS`] letters, digits or `-_.:/`, at most [`MAX_TAGS`], repeats
/// dropped (blank ones too).
pub fn check_tags(tags: &[String]) -> Result<Vec<String>> {
    let mut out: Vec<String> = Vec::new();
    for t in tags {
        let t = t.trim().trim_start_matches('#').to_lowercase();
        if t.is_empty() {
            continue;
        }
        if t.chars().count() > MAX_TAG_CHARS || !t.chars().all(|c| c.is_alphanumeric() || "-_.:/".contains(c)) {
            return Err(invalid(format!(
                "a tag is 1 to {MAX_TAG_CHARS} letters, digits or - _ . : / (no spaces)"
            )));
        }
        if !out.contains(&t) {
            out.push(t);
        }
    }
    if out.len() > MAX_TAGS {
        return Err(invalid(format!("a note has at most {MAX_TAGS} tags")));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ImageRef, MAX_REGIONS};

    pub(crate) fn note(id: &str, anchor: Anchor) -> Note {
        Note {
            id: id.into(),
            anchor,
            text: "seen".into(),
            tags: vec![],
            severity: NoteSeverity::Info,
            author: "ana".into(),
            created_ms: 10,
            updated_ms: 10,
        }
    }

    #[test]
    fn texts_are_checked_and_normalised() {
        assert_eq!(check_text("a\r\nb\rc  \n").unwrap(), "a\nb\nc");
        assert!(check_text("  \n").unwrap_err().to_string().contains("write something"));
        assert!(check_text("a\0").is_err());
        assert!(matches!(check_text(&"x".repeat(MAX_TEXT_BYTES + 1)), Err(NotesError::Limit(_))));
    }

    #[test]
    fn tags_are_normalised_deduplicated_and_bounded() {
        let t = |v: &[&str]| check_tags(&v.iter().map(|s| s.to_string()).collect::<Vec<_>>());
        assert_eq!(t(&[" #Disk ", "disk", "", "net/io"]).unwrap(), vec!["disk", "net/io"]);
        assert!(t(&["two words"]).is_err());
        assert!(t(&[&"x".repeat(MAX_TAG_CHARS + 1)]).is_err());
        let many: Vec<String> = (0..=MAX_TAGS).map(|i| format!("t{i}")).collect();
        assert!(check_tags(&many).is_err());
    }

    #[test]
    fn ids_are_n_and_digits() {
        assert!(check_id("n12").is_ok());
        for bad in ["", "n", "k1", "n1a", "n12345678", "../n1"] {
            assert!(matches!(check_id(bad), Err(NotesError::NotFound(_))), "{bad}");
        }
    }

    #[test]
    fn a_checked_note_cleans_its_fields() {
        let mut n = note("n1", Anchor::Project);
        n.author = "a\nb".into();
        n.updated_ms = 3;
        n.tags = vec!["X".into()];
        let c = n.check().unwrap();
        assert_eq!((c.author.as_str(), c.updated_ms, c.tags[0].as_str()), ("a b", 10, "x"));
        n.id = "bad".into();
        assert!(n.check().is_err());
    }

    #[test]
    fn changes_apply_only_the_given_fields() {
        let mut n = note("n1", Anchor::Instant { at: 1 });
        NoteChange {
            text: Some("new".into()),
            severity: Some(NoteSeverity::Problem),
            ..NoteChange::default()
        }
        .apply(&mut n, 50)
        .unwrap();
        assert_eq!((n.text.as_str(), n.severity, n.updated_ms), ("new", NoteSeverity::Problem, 50));
        assert_eq!(NoteSeverity::Problem.label(), "problem");
        let before = n.clone();
        let bad = NoteChange {
            text: Some("ok".into()),
            tags: Some(vec!["a b".into()]),
            ..NoteChange::default()
        };
        assert!(bad.apply(&mut n, 60).is_err());
        assert_eq!(n, before, "a refused change changes nothing");
        let regions = NoteChange {
            regions: Some(vec![Region { x: 0, y: 0, w: 1, h: 1 }]),
            ..NoteChange::default()
        };
        assert!(regions.apply(&mut n, 60).unwrap_err().to_string().contains("only notes on images"));
    }

    #[test]
    fn image_notes_change_their_regions_within_the_image() {
        let mut n = note(
            "n2",
            Anchor::Image {
                image: ImageRef {
                    key: crate::image_key(b"i"),
                    name: "i.png".into(),
                    path: None,
                    width: 10,
                    height: 10,
                },
                regions: vec![Region { x: 0, y: 0, w: 2, h: 2 }],
            },
        );
        let set = |r: Vec<Region>| NoteChange {
            regions: Some(r),
            tags: Some(vec!["ui".into()]),
            ..NoteChange::default()
        };
        set(vec![Region { x: 5, y: 5, w: 5, h: 5 }]).apply(&mut n, 20).unwrap();
        assert!(matches!(&n.anchor, Anchor::Image { regions, .. } if regions[0].x == 5));
        assert_eq!(n.tags, vec!["ui"]);
        assert!(set(vec![Region { x: 6, y: 5, w: 5, h: 5 }]).apply(&mut n, 20).is_err());
        assert!(set(vec![Region { x: 0, y: 0, w: 1, h: 1 }; MAX_REGIONS + 1])
            .apply(&mut n, 20)
            .is_err());
    }

    #[test]
    fn drafts_and_changes_refuse_unknown_fields() {
        assert!(serde_json::from_value::<NoteChange>(serde_json::json!({"author": "x"})).is_err());
        let d: NoteDraft = serde_json::from_value(serde_json::json!({"anchor": {"kind": "project"}, "text": "t"})).unwrap();
        assert_eq!((d.tags, d.severity), (None, None));
        assert!(serde_json::from_value::<NoteDraft>(serde_json::json!({"anchor": {"kind": "project"}, "text": "t", "id": "n1"})).is_err());
    }
}
