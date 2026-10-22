//! The project's knowledge as saved in projects and sessions (UI state key `knowledge`, owned by the engine like the
//! chats): a versioned record, read leniently. A damaged or newer record gives no knowledge and a warning; files in
//! it that fail the checks are skipped and counted.

use super::{KnowledgeFile, KnowledgeSet};
use serde::{Deserialize, Serialize};
use serde_json::Value;

pub const KNOWLEDGE_RECORD_VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KnowledgeRecord {
    pub version: u32,
    #[serde(default)]
    pub files: Vec<KnowledgeFile>,
}

/// A file as found in a record, before its checks.
#[derive(Deserialize)]
struct RawFile {
    id: String,
    name: String,
    #[serde(default = "enabled")]
    enabled: bool,
    #[serde(default)]
    text: String,
}

fn enabled() -> bool {
    true
}

#[derive(Deserialize)]
struct RawRecord {
    version: u32,
    #[serde(default)]
    files: Vec<Value>,
}

impl KnowledgeRecord {
    pub fn of(set: &KnowledgeSet) -> KnowledgeRecord {
        KnowledgeRecord {
            version: KNOWLEDGE_RECORD_VERSION,
            files: set.files().to_vec(),
        }
    }

    pub fn to_value(&self) -> Value {
        serde_json::to_value(self).unwrap_or(Value::Null)
    }

    /// Reads the optional project value: `None` (an older project) is no knowledge; a damaged or newer one gives a
    /// warning and no knowledge.
    pub fn read(v: Option<&Value>) -> (KnowledgeSet, Option<String>) {
        let Some(v) = v.filter(|v| !v.is_null()) else {
            return (KnowledgeSet::default(), None);
        };
        let raw = match serde_json::from_value::<RawRecord>(v.clone()) {
            Ok(r) if r.version <= KNOWLEDGE_RECORD_VERSION => r,
            Ok(_) => {
                return (
                    KnowledgeSet::default(),
                    Some("The project's knowledge files were written by a newer Debrief and were not loaded.".into()),
                )
            }
            Err(_) => {
                return (
                    KnowledgeSet::default(),
                    Some("The project's knowledge files are damaged and were not loaded.".into()),
                )
            }
        };
        let total = raw.files.len();
        let files: Vec<KnowledgeFile> = raw
            .files
            .into_iter()
            .filter_map(|f| serde_json::from_value::<RawFile>(f).ok())
            .filter_map(|f| KnowledgeFile::new(&f.id, &f.name, f.enabled, &f.text).ok())
            .collect();
        let set = KnowledgeSet::from_files(files);
        let skipped = total - set.files().len();
        let warning = (skipped > 0)
            .then(|| format!("{skipped} of the project's knowledge files could not be loaded (damaged, repeated or too many)."));
        (set, warning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_record_round_trips() {
        let mut set = KnowledgeSet::default();
        set.add("Glossary", "ACME-GW: payment gateway").unwrap();
        let v = KnowledgeRecord::of(&set).to_value();
        assert_eq!(v["version"], 1);
        assert_eq!(KnowledgeRecord::read(Some(&v)), (set, None));
    }

    #[test]
    fn old_damaged_and_newer_records_give_no_knowledge() {
        assert_eq!(KnowledgeRecord::read(None), (KnowledgeSet::default(), None));
        assert_eq!(KnowledgeRecord::read(Some(&Value::Null)).1, None);
        let (s, w) = KnowledgeRecord::read(Some(&json!({"version": "x"})));
        assert!(s.files().is_empty() && w.unwrap().contains("damaged"));
        let (s, w) = KnowledgeRecord::read(Some(&json!({"version": 9, "files": []})));
        assert!(s.files().is_empty() && w.unwrap().contains("newer"));
    }

    #[test]
    fn bad_files_in_a_record_are_skipped_and_counted() {
        let v = json!({"version": 1, "files": [
            {"id": "k1", "name": "Good", "text": "fact"},
            {"id": "../x", "name": "Bad id", "text": ""},
            {"id": "k2", "name": "", "text": ""},
            {"id": "k3", "name": "Too big", "text": "x".repeat(super::super::MAX_FILE_BYTES + 1)},
            {"id": "k1", "name": "Repeated", "text": ""},
            42
        ]});
        let (s, w) = KnowledgeRecord::read(Some(&v));
        assert_eq!(s.files().len(), 1);
        assert!(s.files()[0].enabled, "enabled by default");
        assert!(w.unwrap().starts_with("5 of the project's knowledge files"));
    }
}
