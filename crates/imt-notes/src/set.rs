//! A window's notes: at most [`MAX_NOTES`], unique ids, every note checked.

use crate::error::{NotesError, Result};
use crate::note::{check_tags, check_text};
use crate::{label, Note, NoteChange, NoteDraft, MAX_AUTHOR_CHARS, MAX_NOTES, MAX_TOTAL_TEXT_BYTES};

#[derive(Debug, Clone, Default, PartialEq)]
pub struct NoteSet {
    notes: Vec<Note>,
}

impl NoteSet {
    /// The first [`MAX_NOTES`] checked notes, without repeated ids (the first wins); how many were left out.
    pub fn from_notes(notes: impl IntoIterator<Item = Note>) -> (NoteSet, usize) {
        let mut set = NoteSet::default();
        let mut skipped = 0;
        let mut bytes = 0;
        for n in notes {
            match n.check() {
                Ok(n) if set.notes.len() < MAX_NOTES && set.get(&n.id).is_none() && bytes + n.text.len() <= MAX_TOTAL_TEXT_BYTES => {
                    bytes += n.text.len();
                    set.notes.push(n)
                }
                _ => skipped += 1,
            }
        }
        (set, skipped)
    }

    pub fn notes(&self) -> &[Note] {
        &self.notes
    }

    pub fn is_empty(&self) -> bool {
        self.notes.is_empty()
    }

    pub fn get(&self, id: &str) -> Option<&Note> {
        self.notes.iter().find(|n| n.id == id)
    }

    /// The text of every note together, in bytes.
    fn text_bytes(&self) -> usize {
        self.notes.iter().map(|n| n.text.len()).sum()
    }

    /// Refuses a change that would make the notes' text larger than [`MAX_TOTAL_TEXT_BYTES`].
    fn check_total(&self, removed: usize, added: usize) -> Result<()> {
        if self.text_bytes() - removed + added > MAX_TOTAL_TEXT_BYTES {
            return Err(NotesError::Limit(format!(
                "this project's notes are at their limit ({} MiB of text): shorten or remove some first",
                MAX_TOTAL_TEXT_BYTES / (1024 * 1024)
            )));
        }
        Ok(())
    }

    fn get_mut(&mut self, id: &str) -> Result<&mut Note> {
        self.notes
            .iter_mut()
            .find(|n| n.id == id)
            .ok_or_else(|| NotesError::NotFound("this note no longer exists".into()))
    }

    /// The next id: one more than the largest.
    fn fresh_id(&self) -> String {
        let max = self
            .notes
            .iter()
            .filter_map(|n| n.id.strip_prefix('n').and_then(|d| d.parse::<u32>().ok()))
            .max()
            .unwrap_or(0);
        format!("n{}", max + 1)
    }

    /// Adds a note from `draft` (its anchor already filled by the caller) by `author` at `now_ms`; returns it.
    pub fn add(&mut self, draft: &NoteDraft, author: &str, now_ms: i64) -> Result<&Note> {
        if self.notes.len() >= MAX_NOTES {
            return Err(NotesError::Limit(format!(
                "this project has {MAX_NOTES} notes, the most it can hold: remove some first"
            )));
        }
        let text = check_text(&draft.text)?;
        self.check_total(0, text.len())?;
        let note = Note {
            id: self.fresh_id(),
            anchor: draft.anchor.check()?,
            text,
            tags: check_tags(draft.tags.as_deref().unwrap_or_default())?,
            severity: draft.severity.unwrap_or_default(),
            author: label(author, MAX_AUTHOR_CHARS),
            created_ms: now_ms,
            updated_ms: now_ms,
        };
        self.notes.push(note);
        Ok(&self.notes[self.notes.len() - 1])
    }

    /// Applies `change` to the note `id` (checked before anything changes); returns it.
    pub fn update(&mut self, id: &str, change: &NoteChange, now_ms: i64) -> Result<&Note> {
        if let Some(t) = &change.text {
            let old = self.get(id).map_or(0, |n| n.text.len());
            self.check_total(old, t.len())?;
        }
        let n = self.get_mut(id)?;
        change.apply(n, now_ms)?;
        Ok(n)
    }

    pub fn remove(&mut self, id: &str) -> Result<()> {
        let before = self.notes.len();
        self.notes.retain(|n| n.id != id);
        if self.notes.len() == before {
            return Err(NotesError::NotFound("this note no longer exists".into()));
        }
        Ok(())
    }

    /// The image keys the notes mark (each once, in order).
    pub fn image_keys(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for n in &self.notes {
            if let crate::Anchor::Image { image, .. } = &n.anchor {
                if !out.contains(&image.key.as_str()) {
                    out.push(&image.key);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{image_key, Anchor, ImageRef, NoteSeverity, Region};

    fn draft(anchor: Anchor, text: &str) -> NoteDraft {
        NoteDraft {
            anchor,
            text: text.into(),
            tags: Some(vec!["Disk".into()]),
            severity: Some(NoteSeverity::Warning),
        }
    }

    #[test]
    fn notes_are_added_changed_and_removed() {
        let mut s = NoteSet::default();
        assert!(s.is_empty());
        let n = s.add(&draft(Anchor::Project, "first"), "ana", 5).unwrap().clone();
        assert_eq!(
            (n.id.as_str(), n.tags[0].as_str(), n.severity, n.created_ms),
            ("n1", "disk", NoteSeverity::Warning, 5)
        );
        assert_eq!(s.add(&draft(Anchor::Instant { at: 1 }, "second"), "ana", 6).unwrap().id, "n2");
        s.remove("n1").unwrap();
        assert_eq!(
            s.add(&draft(Anchor::Project, "third"), "", 7).unwrap().id,
            "n3",
            "ids are not reused"
        );
        let change = NoteChange {
            text: Some("edited".into()),
            ..NoteChange::default()
        };
        assert_eq!(s.update("n2", &change, 9).unwrap().text, "edited");
        assert!(matches!(s.update("n9", &change, 9), Err(NotesError::NotFound(_))));
        assert!(matches!(s.remove("n9"), Err(NotesError::NotFound(_))));
        assert!(s.add(&draft(Anchor::Range { from: 2, to: 1 }, "bad"), "", 1).is_err());
        assert!(s.add(&draft(Anchor::Project, " "), "", 1).is_err());
        assert_eq!(s.notes().len(), 2);
        assert!(s.get("n2").is_some());
    }

    #[test]
    fn a_set_holds_at_most_max_notes() {
        let notes = (1..=MAX_NOTES).map(|i| Note {
            id: format!("n{i}"),
            anchor: Anchor::Project,
            text: "t".into(),
            tags: vec![],
            severity: NoteSeverity::Info,
            author: String::new(),
            created_ms: 0,
            updated_ms: 0,
        });
        let (mut s, skipped) = NoteSet::from_notes(notes);
        assert_eq!((s.notes().len(), skipped), (MAX_NOTES, 0));
        assert!(matches!(s.add(&draft(Anchor::Project, "x"), "", 0), Err(NotesError::Limit(_))));
    }

    #[test]
    fn the_text_of_all_notes_is_bounded() {
        let big = "x".repeat(crate::MAX_TEXT_BYTES);
        let per = crate::MAX_TOTAL_TEXT_BYTES / big.len();
        let mut s = NoteSet::default();
        for _ in 0..per {
            s.add(&draft(Anchor::Project, &big), "", 0).unwrap();
        }
        assert!(matches!(
            s.add(&draft(Anchor::Project, "one more"), "", 0),
            Err(NotesError::Limit(_))
        ));
        let shorter = NoteChange {
            text: Some("short".into()),
            ..NoteChange::default()
        };
        s.update("n1", &shorter, 1).unwrap();
        let longer = NoteChange {
            text: Some(big.clone()),
            ..NoteChange::default()
        };
        s.update("n1", &longer, 1).unwrap();
        s.add(&draft(Anchor::Project, "fits again"), "", 0).unwrap_err();
        // A saved project over the limit keeps what fits.
        let notes: Vec<Note> = s
            .notes()
            .iter()
            .cloned()
            .chain(s.notes().iter().cloned().map(|mut n| {
                n.id = format!("n{}", n.id[1..].parse::<u32>().unwrap() + 1000);
                n
            }))
            .collect();
        let (back, skipped) = NoteSet::from_notes(notes);
        assert_eq!((back.notes().len(), skipped), (per, per));
    }

    #[test]
    fn image_keys_are_listed_once() {
        let img = |b: &[u8]| Anchor::Image {
            image: ImageRef {
                key: image_key(b),
                name: "a.png".into(),
                path: None,
                width: 4,
                height: 4,
            },
            regions: vec![Region { x: 0, y: 0, w: 1, h: 1 }],
        };
        let mut s = NoteSet::default();
        s.add(&draft(img(b"a"), "one"), "", 0).unwrap();
        s.add(&draft(img(b"a"), "two"), "", 0).unwrap();
        s.add(&draft(img(b"b"), "three"), "", 0).unwrap();
        s.add(&draft(Anchor::Project, "four"), "", 0).unwrap();
        assert_eq!(s.image_keys(), vec![image_key(b"a"), image_key(b"b")]);
    }
}
