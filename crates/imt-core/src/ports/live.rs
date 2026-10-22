//! DTOs of live tail on a store ([`super::Tail`], [`super::Ingest::append_tail`]): what one refresh changed and
//! how each followed file was last read. Plain structs and unit enums (they cross the worker protocol as JSON).

use serde::{Deserialize, Serialize};

/// How the store reads a followed file.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum TailRead {
    /// Only the appended bytes, from where the last read stopped.
    #[default]
    Append,
    /// Read again in full when it changes (binary formats, archives, formats whose events depend on earlier
    /// ones, files without a tail state); [`TailFile::note`] says why.
    Whole,
}

/// A followed file as the store last read it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailFile {
    pub source: u32,
    /// The file's path (an archive member's `<archive>!/<member>`).
    pub path: String,
    pub name: String,
    /// Bytes turned into events: the next read starts here.
    pub offset: u64,
    /// The file's size when it was last read.
    pub size: u64,
    /// Lines read (the next read starts at line `lines + 1`).
    pub lines: u64,
    pub read: TailRead,
    /// Why the file is read whole, or that it was opened at its end.
    pub note: Option<String>,
    /// Wall-clock time (ms) the store last read something new from it.
    pub changed_ms: Option<i64>,
}

/// What one live tail refresh changed ([`super::Ingest::append_tail`]). Event ids are stable: appended events get
/// new ids, existing ones keep theirs (a file read again in full gets a new source and new ids).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TailAppend {
    /// Events added (by appended lines, new files and files read again).
    pub appended: usize,
    /// Sources whose events changed in any way.
    pub sources_changed: Vec<u32>,
    /// Sources of files that appeared under an opened folder.
    pub added: Vec<u32>,
    /// Sources of files that were truncated or replaced (read again from their start as this new source).
    pub truncated: Vec<u32>,
    /// Sources of files read again in full because they cannot be read from the middle.
    pub reimported: Vec<u32>,
    /// Sources whose file disappeared (their events stay).
    pub deleted: Vec<u32>,
}

impl TailAppend {
    /// Whether nothing changed.
    pub fn is_empty(&self) -> bool {
        self.appended == 0
            && self.sources_changed.is_empty()
            && self.added.is_empty()
            && self.truncated.is_empty()
            && self.reimported.is_empty()
            && self.deleted.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tail_dtos_round_trip_as_json() {
        let f = TailFile {
            source: 3,
            path: "/logs/a.log".into(),
            name: "a.log".into(),
            offset: 10,
            size: 12,
            lines: 2,
            read: TailRead::Whole,
            note: Some("binary".into()),
            changed_ms: Some(5),
        };
        let json = serde_json::to_value(&f).unwrap();
        assert_eq!(json["read"], "whole");
        assert_eq!(json["changedMs"], 5);
        assert_eq!(serde_json::from_value::<TailFile>(json).unwrap(), f);
        let mut a = TailAppend::default();
        assert!(a.is_empty());
        a.deleted.push(1);
        assert!(!a.is_empty());
        let back: TailAppend = serde_json::from_str(&serde_json::to_string(&a).unwrap()).unwrap();
        assert_eq!(back, a);
    }
}
