//! File importers. An importer turns one file into records. New file types
//! (registry exports, msinfo, ...) are added by implementing [`Importer`] and
//! registering it with [`Importers::register`].

mod csv;
mod json;
pub mod lines;
pub mod text;

use crate::profile::CompiledProfile;
use anyhow::Result;
use chrono::NaiveDate;
use std::path::Path;

pub use self::csv::CsvImporter;
pub(crate) use self::csv::{count_outside_quotes as csv_count_outside_quotes, delimiter as csv_delimiter};
pub use self::json::JsonImporter;
pub use self::text::TextImporter;
// Shared with the streaming importers (`crate::ingest`).
pub(crate) use self::json::{to_record as json_to_record, Depth as JsonDepth};
pub(crate) use self::text::{finish as text_finish, SNIFF_LINES};

pub use imt_plugin_api::text::{decode, MAX_LINE_LEN};
pub(crate) use imt_plugin_api::text::{first_lines, looks_binary, looks_utf16le, utf16};

pub struct ImportCtx<'a> {
    pub profile: &'a CompiledProfile,
    pub path: &'a Path,
    /// Date of the file's last modification, used for logs that only carry a time of day.
    pub file_date: Option<NaiveDate>,
}

pub use imt_plugin_api::importer::Imported;

pub trait Importer: Send + Sync {
    /// Stable id shown in the UI and used by a parser's `[format] kind`.
    fn id(&self) -> &str;

    /// How confident the importer is that it can read this file, 0 (not at
    /// all) to 100. `head` is the decoded start of the file.
    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8;

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported>;

    /// Live tail: parses `tail`, text appended to a file this importer already
    /// read, whose start is `head` (for headers, the detected format and date
    /// order). Line numbers are relative to `tail`. `None` means the importer
    /// cannot append; the workspace then re-imports the whole file. The
    /// default is `None`, so importers that keep cross-record state stay correct.
    fn import_tail(&self, _ctx: &ImportCtx, _head: &str, _tail: &str) -> Option<Result<Imported>> {
        None
    }

    /// Whether one record can span several lines (stack traces, quoted
    /// newlines). Live tail then re-reads a file from the start of its last
    /// record so continuation lines merge into it.
    fn multiline(&self) -> bool {
        true
    }

    /// Fields this importer writes that identify who wrote an event (host, application, process), in
    /// order from the widest to the narrowest, e.g. syslog's `Host`, `App`, `ProcId`. They join the
    /// profile's `[instance] candidates` for the automatic instance key of this importer's sources. The
    /// default is none; must not fail.
    fn identity_fields(&self) -> Vec<String> {
        Vec::new()
    }
}

/// An importer for binary formats (e.g. Windows `.evtx`). Binary importers
/// are asked first, with the raw bytes; text importers only see files that
/// decode as text.
pub trait BinaryImporter: Send + Sync {
    /// Stable id shown in the UI and usable in a parser's `[format] kind`.
    fn id(&self) -> &str;

    /// Confidence 0..=100 from the first bytes (usually a magic number).
    fn sniff(&self, ctx: &ImportCtx, head: &[u8]) -> u8;

    fn import(&self, ctx: &ImportCtx, bytes: &[u8]) -> Result<Imported>;

    /// As [`Importer::identity_fields`]: fields that identify who wrote an event (e.g. EVTX `Computer`,
    /// `ProcessId`). The default is none; must not fail.
    fn identity_fields(&self) -> Vec<String> {
        Vec::new()
    }
}

pub struct Importers {
    list: Vec<Box<dyn Importer>>,
    binary: Vec<Box<dyn BinaryImporter>>,
}

impl Default for Importers {
    fn default() -> Self {
        Self::new()
    }
}

impl Importers {
    /// The built-in importers: JSON, CSV and text.
    pub fn new() -> Self {
        let mut importers = Self::empty();
        importers
            .register(Box::new(JsonImporter))
            .register(Box::new(CsvImporter))
            .register(Box::new(TextImporter));
        importers
    }

    /// No importers at all; add them with [`Importers::register`].
    pub fn empty() -> Self {
        Importers {
            list: Vec::new(),
            binary: Vec::new(),
        }
    }

    /// Adds an importer. When several are equally confident the earliest registered wins.
    pub fn register(&mut self, importer: Box<dyn Importer>) -> &mut Self {
        self.list.push(importer);
        self
    }

    pub fn register_binary(&mut self, importer: Box<dyn BinaryImporter>) -> &mut Self {
        self.binary.push(importer);
        self
    }

    /// Every text importer, in registration order.
    pub fn text_importers(&self) -> impl Iterator<Item = &dyn Importer> {
        self.list.iter().map(|b| b.as_ref())
    }

    /// Every binary importer, in registration order.
    pub fn binary_importers(&self) -> impl Iterator<Item = &dyn BinaryImporter> {
        self.binary.iter().map(|b| b.as_ref())
    }

    /// The text importer with this id.
    pub fn find(&self, id: &str) -> Option<&dyn Importer> {
        self.list.iter().find(|i| i.id() == id).map(|b| b.as_ref())
    }

    /// The binary importer for these bytes: the one forced by the parser's `[format] kind`,
    /// else the most confident one (earliest registered on ties).
    pub fn select_binary(&self, ctx: &ImportCtx, head: &[u8]) -> Option<&dyn BinaryImporter> {
        let forced = ctx.profile.format.kind.as_str();
        if forced != "auto" {
            return self.binary.iter().find(|i| i.id() == forced).map(|b| b.as_ref());
        }
        self.binary
            .iter()
            .map(|i| (i.sniff(ctx, head), i))
            .filter(|(score, _)| *score > 0)
            .reduce(|best, c| if c.0 > best.0 { c } else { best })
            .map(|(_, i)| i.as_ref())
    }

    /// Picks the importer for a file: the one forced by the parser's `[format] kind`, else
    /// the most confident one.
    pub fn select(&self, ctx: &ImportCtx, head: &str) -> Option<&dyn Importer> {
        let forced = ctx.profile.format.kind.as_str();
        if forced != "auto" {
            return self.list.iter().find(|i| i.id() == forced).map(|b| b.as_ref());
        }
        self.list
            .iter()
            .map(|i| (i.sniff(ctx, head), i))
            .filter(|(score, _)| *score > 0)
            .reduce(|best, c| if c.0 > best.0 { c } else { best })
            .map(|(_, i)| i.as_ref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16() {
        let s = "Windows Registry Editor Version 5.00\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        for u in s.encode_utf16() {
            bytes.extend(u.to_le_bytes());
        }
        assert_eq!(decode(&bytes).unwrap(), s);
        assert!(decode(&[0u8, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 1, 2, 3]).is_none());
    }

    use crate::profile::CompiledProfile;

    fn ctx_profile(toml: &str) -> CompiledProfile {
        crate::parser::testing::compiled(toml)
    }

    #[test]
    fn selects_importer() {
        let auto = ctx_profile("");
        let ctx = ImportCtx {
            profile: &auto,
            path: Path::new("x"),
            file_date: None,
        };
        let importers = Importers::default();
        assert_eq!(importers.select(&ctx, "{\"@t\":\"2026-10-14T10:00:00Z\"}").unwrap().id(), "json");
        assert_eq!(importers.select(&ctx, "a,b\n1,2\n").unwrap().id(), "csv");
        assert_eq!(importers.select(&ctx, "2026-10-14 10:00:00 INFO x").unwrap().id(), "text");
        assert!(importers.select(&ctx, "just words").is_none());
        let forced = ctx_profile("[format]\nkind = 'text'");
        let ctx = ImportCtx {
            profile: &forced,
            path: Path::new("x"),
            file_date: None,
        };
        assert_eq!(importers.select(&ctx, "{}").unwrap().id(), "text");
        let unknown = ctx_profile("[format]\nkind = 'xml'");
        let ctx = ImportCtx {
            profile: &unknown,
            path: Path::new("x"),
            file_date: None,
        };
        assert!(importers.select(&ctx, "{}").is_none());
    }

    struct Fixed(&'static str, u8);

    impl Importer for Fixed {
        fn id(&self) -> &'static str {
            self.0
        }
        fn sniff(&self, _ctx: &ImportCtx, _head: &str) -> u8 {
            self.1
        }
        fn import(&self, _ctx: &ImportCtx, _content: &str) -> Result<Imported> {
            Ok(Imported::default())
        }
    }

    #[test]
    fn registration_order_breaks_ties() {
        let profile = ctx_profile("");
        let ctx = ImportCtx {
            profile: &profile,
            path: Path::new("x"),
            file_date: None,
        };
        assert!(Importers::empty().select(&ctx, "x").is_none());
        let mut importers = Importers::empty();
        importers
            .register(Box::new(Fixed("first", 50)))
            .register(Box::new(Fixed("second", 50)));
        assert_eq!(importers.select(&ctx, "x").unwrap().id(), "first");
        importers.register(Box::new(Fixed("best", 51)));
        let best = importers.select(&ctx, "x").unwrap();
        assert_eq!(best.id(), "best");
        assert!(best.import(&ctx, "x").unwrap().records.is_empty());
    }
}
