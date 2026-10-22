//! The project's notes as saved in projects and sessions (UI state key `notes`, owned by the engine like the chats
//! and the knowledge files): a versioned record, read leniently. A damaged or newer record gives no notes and a
//! warning; notes in it that fail their checks are skipped and counted.

use crate::{Note, NoteSet};
use serde::{Deserialize, Serialize};
use serde_json::Value;
#[cfg(feature = "ts")]
use ts_rs::TS;

pub const NOTE_RECORD_VERSION: u32 = 1;

#[cfg_attr(feature = "ts", derive(TS))]
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoteRecord {
    pub version: u32,
    #[serde(default)]
    pub notes: Vec<Note>,
}

#[derive(Deserialize)]
struct RawRecord {
    version: u32,
    #[serde(default)]
    notes: Vec<Value>,
}

impl NoteRecord {
    pub fn of(set: &NoteSet) -> NoteRecord {
        NoteRecord {
            version: NOTE_RECORD_VERSION,
            notes: set.notes().to_vec(),
        }
    }

    /// Reads the optional project value: none (a project without notes) is no notes; a damaged or newer one gives a
    /// warning and no notes.
    pub fn read(v: Option<&Value>) -> (NoteSet, Option<String>) {
        let Some(v) = v.filter(|v| !v.is_null()) else {
            return (NoteSet::default(), None);
        };
        let raw = match serde_json::from_value::<RawRecord>(v.clone()) {
            Ok(r) if r.version <= NOTE_RECORD_VERSION => r,
            Ok(_) => {
                return (
                    NoteSet::default(),
                    Some("The project's notes were written by a newer Debrief and were not loaded.".into()),
                )
            }
            Err(_) => {
                return (
                    NoteSet::default(),
                    Some("The project's notes are damaged and were not loaded.".into()),
                )
            }
        };
        let total = raw.notes.len();
        let parsed: Vec<Note> = raw.notes.into_iter().filter_map(|n| serde_json::from_value(n).ok()).collect();
        let unreadable = total - parsed.len();
        let (set, skipped) = NoteSet::from_notes(parsed);
        let left_out = unreadable + skipped;
        let warning =
            (left_out > 0).then(|| format!("{left_out} of the project's notes could not be loaded (damaged, repeated or too many)."));
        (set, warning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Anchor, NoteDraft};
    use serde_json::json;

    #[test]
    fn a_record_round_trips() {
        let mut set = NoteSet::default();
        let d = NoteDraft {
            anchor: Anchor::Event {
                event_id: 42,
                ts: Some(1_000),
                source: "app.log".into(),
                excerpt: "disk full".into(),
            },
            text: "**Root cause** here".into(),
            tags: None,
            severity: None,
        };
        set.add(&d, "ana", 5).unwrap();
        let v = serde_json::to_value(NoteRecord::of(&set)).unwrap();
        assert_eq!(v["version"], 1);
        assert_eq!(v["notes"][0]["anchor"]["kind"], "event");
        assert_eq!(NoteRecord::read(Some(&v)), (set, None));
    }

    #[test]
    fn missing_damaged_and_newer_records_give_no_notes() {
        assert_eq!(NoteRecord::read(None), (NoteSet::default(), None));
        assert_eq!(NoteRecord::read(Some(&Value::Null)).1, None);
        let (s, w) = NoteRecord::read(Some(&json!({"version": "x"})));
        assert!(s.is_empty() && w.unwrap().contains("damaged"));
        let (s, w) = NoteRecord::read(Some(&json!({"version": 2, "notes": []})));
        assert!(s.is_empty() && w.unwrap().contains("newer"));
    }

    #[test]
    fn damaged_notes_are_skipped_and_counted() {
        let ok = |id: &str| json!({"id": id, "anchor": {"kind": "project"}, "text": "t", "createdMs": 1, "updatedMs": 1});
        let v = json!({"version": 1, "notes": [
            ok("n1"),
            ok("n1"),
            {"id": "n2", "anchor": {"kind": "range", "from": 5, "to": 1}, "text": "t", "createdMs": 1, "updatedMs": 1},
            {"id": "n3", "anchor": {"kind": "project"}, "text": " ", "createdMs": 1, "updatedMs": 1},
            {"id": "n4", "anchor": {"kind": "spaceship"}, "text": "t", "createdMs": 1, "updatedMs": 1},
            42,
            ok("n5")
        ]});
        let (s, w) = NoteRecord::read(Some(&v));
        assert_eq!(s.notes().iter().map(|n| n.id.as_str()).collect::<Vec<_>>(), vec!["n1", "n5"]);
        assert!(w.unwrap().starts_with("5 of the project's notes"));
    }
}
