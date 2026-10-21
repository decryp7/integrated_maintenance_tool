//! Importers: a plugin turns one file (text IMT decoded, or raw bytes) into records. The host picks the
//! importer by sniffing, applies the profile's pipeline, marker rules and ids to what it returns, and writes
//! event batches; an importer only parses.

use crate::model::Record;
use crate::time::TimestampParser;
use anyhow::Result;
use chrono::NaiveDate;
use std::path::Path;

pub use crate::text::{decode, MAX_LINE_LEN};

/// Components of a file's path a plugin sees without [`crate::manifest::Permission::OpenedFilePaths`]: enough
/// for layouts that encode meaning in folders (`pods/<ns>_<pod>_<uid>/<container>/0.log`), never the whole path.
pub const PATH_TAIL_COMPONENTS: usize = 4;

/// What an importer knows about the file it reads.
#[derive(Clone, Copy)]
pub struct ImportCtx<'a> {
    /// The file's name (no folders).
    pub file_name: &'a str,
    /// The trailing [`PATH_TAIL_COMPONENTS`] components of its path, or the full path for plugins granted
    /// [`crate::manifest::Permission::OpenedFilePaths`].
    pub path: &'a Path,
    /// Date of the file's last modification, used for logs that only carry a time of day.
    pub file_date: Option<NaiveDate>,
    /// The active profile's timestamp settings.
    pub timestamps: &'a TimestampParser,
}

impl<'a> ImportCtx<'a> {
    /// A context for a file named by `path` (tests, tools).
    pub fn new(path: &'a Path, timestamps: &'a TimestampParser) -> ImportCtx<'a> {
        ImportCtx {
            file_name: path.file_name().and_then(|n| n.to_str()).unwrap_or(""),
            path,
            file_date: None,
            timestamps,
        }
    }
}

/// What an import produced.
#[derive(Debug, Default)]
pub struct Imported {
    /// The parsed events, in file order.
    pub records: Vec<Record>,
    /// Lines or entries skipped (not records).
    pub skipped: usize,
    /// Problems met while reading, aggregated per kind (e.g. lines that look like events but match no
    /// format). Importers that find none leave it empty (`..Default::default()`).
    pub problems: crate::problems::Problems,
}

/// A text format. The host decodes the file (UTF-8, UTF-16, BOMs) and refuses binary files before it asks.
pub trait TextImporter: Send + Sync {
    /// Stable id shown in the UI and used by `parse.format`.
    fn id(&self) -> &str;

    /// How confident the importer is that it can read this file, 0 (not at all) to 100. `head` is the decoded
    /// start of the file.
    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8;

    /// Imports `content` (the decoded file) into records.
    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported>;

    /// Live tail: parses `tail`, text appended to a file this importer already read, whose start is `head`
    /// (for headers, the detected format and date order). Line numbers are relative to `tail`. `None` means
    /// the importer cannot append; the host then re-imports the whole file.
    fn import_tail(&self, _ctx: &ImportCtx, _head: &str, _tail: &str) -> Option<Result<Imported>> {
        None
    }

    /// Whether one record can span several lines (stack traces, quoted newlines).
    fn multiline(&self) -> bool {
        true
    }

    /// Fields this importer writes that identify who wrote an event (host, application, process), from the
    /// widest to the narrowest, e.g. syslog's `Host`, `App`, `ProcId` (ADR 0020). They join the profile's
    /// `[instance] candidates` for the automatic instance key of this importer's sources. The default is none.
    fn identity_fields(&self) -> Vec<String> {
        Vec::new()
    }
}

/// A binary format (e.g. Windows `.evtx`). Binary importers are asked first, with the raw bytes.
pub trait BinaryImporter: Send + Sync {
    /// Stable id shown in the UI and usable in `parse.format`.
    fn id(&self) -> &str;

    /// Confidence 0..=100 from the first bytes (usually a magic number).
    fn sniff(&self, ctx: &ImportCtx, head: &[u8]) -> u8;

    /// Imports the raw `bytes` of the file into records.
    fn import(&self, ctx: &ImportCtx, bytes: &[u8]) -> Result<Imported>;

    /// As [`TextImporter::identity_fields`] (e.g. EVTX `Computer`, `ProcessId`). The default is none.
    fn identity_fields(&self) -> Vec<String> {
        Vec::new()
    }
}

/// The trailing [`PATH_TAIL_COMPONENTS`] components of `path`.
pub fn path_tail(path: &Path) -> std::path::PathBuf {
    let parts: Vec<_> = path.components().filter(|c| matches!(c, std::path::Component::Normal(_))).collect();
    let start = parts.len().saturating_sub(PATH_TAIL_COMPONENTS);
    parts[start..].iter().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugins_see_the_tail_of_a_path() {
        let p = Path::new("/home/me/logs/pods/ns_pod_uid/app/0.log");
        assert_eq!(path_tail(p), Path::new("pods/ns_pod_uid/app/0.log"));
        assert_eq!(path_tail(Path::new("x.log")), Path::new("x.log"));
        let ts = TimestampParser::default();
        let ctx = ImportCtx::new(Path::new("a/b.log"), &ts);
        assert_eq!(ctx.file_name, "b.log");
    }
}
