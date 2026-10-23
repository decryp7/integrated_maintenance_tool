//! The importer port: a dump or UMDH format the host can open. The dumps plugin's importers reach the engine
//! through it (`debrief-plugin-host` registers a proxy per importer on [`IMPORTERS_POINT`]); tests use in-process
//! ones. The file is lent as a [`ReadAt`], so a large full dump is read in parts, never whole.

use crate::error::Result;
use crate::limits::Limits;
use crate::model::Document;
use serde::{Deserialize, Serialize};
use std::sync::Arc;

/// The extension point dump importers are contributed to.
pub const IMPORTERS_POINT: &str = "dumps.importers";
/// Bytes of a file's start that sniffing looks at.
pub const SNIFF_BYTES: usize = 64 << 10;
/// Lowest sniff score that counts as recognised.
pub const SNIFF_THRESHOLD: u8 = 50;

/// What an importer produces.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Produces {
    Dump,
    Umdh,
}

/// A file the host lends an importer, read by ranges.
pub trait ReadAt: Send + Sync {
    fn len(&self) -> u64;
    /// Up to `len` bytes at `offset` (fewer at the end of the file).
    fn read_at(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl ReadAt for [u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX).min(<[u8]>::len(self));
        let end = start.saturating_add(len).min(<[u8]>::len(self));
        Ok(self[start..end].to_vec())
    }
}

impl ReadAt for Vec<u8> {
    fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }

    fn read_at(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        self.as_slice().read_at(offset, len)
    }
}

/// What the host tells an importer besides the file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ImportOptions {
    pub limits: Limits,
    /// The file's modification time (Unix ms): the time of a UMDH log, which records none.
    pub file_time_ms: Option<i64>,
    /// Absolute folders with PDBs (symstore layout) the host adds to the reader's own symbol folders: Debrief's symbol
    /// cache, which it fills from the user's symbol servers (ADR 0036).
    #[serde(default)]
    pub symbol_folders: Vec<String>,
}

/// A dump or UMDH format.
pub trait DumpImporter: Send + Sync {
    fn id(&self) -> &str;
    fn produces(&self) -> Produces;
    /// Confidence 0–100 from the file's name and first bytes (at most [`SNIFF_BYTES`]).
    fn sniff(&self, file_name: &str, head: &[u8]) -> u8;
    /// Reads the file into a document. The file is shared so a guarded caller can hand it to a helper thread.
    fn import(&self, file_name: &str, file: Arc<dyn ReadAt>, options: &ImportOptions) -> Result<Document>;
    /// Names the frames of an already read `doc` that still lack a name, from the symbols that are there now
    /// (`options.symbol_folders`), without reading the file again. The result equals reading the file again with
    /// those symbols. A format that cannot do this answers [`ErrorCode::Unsupported`](crate::ErrorCode) and the
    /// caller reads the file again.
    fn resymbolise(&self, _doc: Document, _options: &ImportOptions) -> Result<Document> {
        Err(crate::DumpError::unsupported(
            "this reader cannot name the frames of an open dump again",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slices_read_by_range() {
        let v: Vec<u8> = (0u8..10).collect();
        assert_eq!(ReadAt::len(&v), 10);
        assert_eq!(v.read_at(8, 5).unwrap(), [8, 9]);
        assert!(v.read_at(20, 5).unwrap().is_empty());
        assert!(v.read_at(u64::MAX, usize::MAX).unwrap().is_empty());
        assert!(!ReadAt::is_empty(&v));
        assert!(ReadAt::is_empty(&Vec::new()));
    }
}
