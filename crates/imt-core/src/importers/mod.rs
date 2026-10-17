//! File importers. An importer turns one file into records. New file types
//! (registry exports, msinfo, ...) are added by implementing [`Importer`] and
//! registering it with [`Importers::register`].

mod csv;
mod json;
mod text;

use crate::model::Record;
use crate::profile::CompiledProfile;
use anyhow::Result;
use chrono::NaiveDate;
use std::path::Path;

pub use self::csv::CsvImporter;
pub use self::json::JsonImporter;
pub use self::text::TextImporter;

/// Lines longer than this (4 MiB) are skipped and counted, so one corrupt
/// line cannot blow up memory in later processing.
pub const MAX_LINE_LEN: usize = 4 << 20;

pub struct ImportCtx<'a> {
    pub profile: &'a CompiledProfile,
    pub path: &'a Path,
    /// Date of the file's last modification, used for logs that only carry a time of day.
    pub file_date: Option<NaiveDate>,
}

#[derive(Default)]
pub struct Imported {
    pub records: Vec<Record>,
    pub skipped: usize,
}

pub trait Importer: Send + Sync {
    /// Stable id shown in the UI and used by `parse.format`.
    fn id(&self) -> &'static str;

    /// How confident the importer is that it can read this file, 0 (not at
    /// all) to 100. `head` is the decoded start of the file.
    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8;

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported>;
}

pub struct Importers {
    list: Vec<Box<dyn Importer>>,
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
        Importers { list: Vec::new() }
    }

    /// Adds an importer. When several are equally confident the earliest registered wins.
    pub fn register(&mut self, importer: Box<dyn Importer>) -> &mut Self {
        self.list.push(importer);
        self
    }

    /// Picks the importer for a file: the one forced by `parse.format`, else
    /// the most confident one.
    pub fn select(&self, ctx: &ImportCtx, head: &str) -> Option<&dyn Importer> {
        let forced = ctx.profile.profile.parse.format.as_str();
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

/// Decodes file bytes: UTF-8 (with or without BOM) or UTF-16 LE/BE, which is
/// common for files written by Windows tools. Returns None for binary data.
pub fn decode(bytes: &[u8]) -> Option<String> {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, u16::from_le_bytes)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, u16::from_be_bytes)
    } else if looks_utf16le(bytes) {
        utf16(bytes, u16::from_le_bytes)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    let sample: String = text.chars().take(8192).collect();
    let control = sample
        .chars()
        .filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\x0c'))
        .count();
    if control > sample.len() / 100 + 2 {
        return None;
    }
    Some(text)
}

fn utf16(bytes: &[u8], f: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| f([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

fn looks_utf16le(bytes: &[u8]) -> bool {
    let n = bytes.len().min(512) & !1;
    n >= 8 && bytes[..n].iter().skip(1).step_by(2).filter(|b| **b == 0).count() * 10 > n / 2 * 9
}

pub(crate) fn first_lines(text: &str, n: usize) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).take(n)
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

    use crate::profile::{CompiledProfile, Profile};

    fn ctx_profile(toml: &str) -> CompiledProfile {
        CompiledProfile::compile(Profile::from_toml(toml).unwrap()).unwrap()
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
        let forced = ctx_profile("[parse]\nformat = 'text'");
        let ctx = ImportCtx {
            profile: &forced,
            path: Path::new("x"),
            file_date: None,
        };
        assert_eq!(importers.select(&ctx, "{}").unwrap().id(), "text");
        let unknown = ctx_profile("[parse]\nformat = 'xml'");
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
        assert_eq!(importers.select(&ctx, "x").unwrap().id(), "best");
    }

    #[test]
    fn decodes_variants() {
        assert_eq!(decode(b"\xEF\xBB\xBFhello").unwrap(), "hello");
        let mut be = vec![0xFE, 0xFF];
        for u in "hi".encode_utf16() {
            be.extend(u.to_be_bytes());
        }
        assert_eq!(decode(&be).unwrap(), "hi");
        let le: Vec<u8> = "no bom here".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode(&le).unwrap(), "no bom here");
        assert_eq!(first_lines("a\n\n  \nb\nc", 2).collect::<Vec<_>>(), vec!["a", "b"]);
    }
}
