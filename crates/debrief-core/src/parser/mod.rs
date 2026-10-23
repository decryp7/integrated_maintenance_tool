//! Parsers (ADR 0026): one TOML file per log format, kept in a library and matched per file.
//!
//! - [`Parser`]: a parser file, parsed and checked once at the edge ([`Parser::from_toml`]): unknown keys,
//!   schema versions, limits and regexes are refused with the line and column.
//! - [`format`]: its `[format]` (the former profile `[parse]`), [`matching`]: its `[match]` rules and scores.
//! - [`builtin`]: the parsers shipped with Debrief; [`library`]: the user's parser folder and its index.
//! - [`set`]: the library as the engine uses it: every parser compiled with the profile, the per-file choice.
//! - [`draft`]: new parsers and changed copies composed for the format wizard and quick fixes.
//! - [`verify`]: the engine's verdict on a candidate parser (ADR 0032): score, failing lines, timestamps, zone.

pub mod builtin;
pub mod draft;
pub mod format;
pub mod library;
pub mod matching;
pub mod set;
pub mod verify;

use crate::ports::{content_hash, ParserRef};
use crate::profile::ExtractCfg;
use serde::{Deserialize, Serialize};

pub use format::{CsvCfg, DayFirst, FormatCfg, JsonCfg};
pub use matching::MatchCfg;
pub use set::{ParserSet, ParserSetup, SetupParser};

/// The parser schema this Debrief reads (`schema = 1`).
pub const SCHEMA_VERSION: u32 = 1;
/// Largest parser file.
pub const MAX_PARSER_BYTES: usize = 256 * 1024;
/// Most output templates plus regexes of one parser.
pub const MAX_TEXT_FORMATS: usize = 64;
/// Most `[[extract]]` and `[[pipeline]]` entries of one parser (each).
pub const MAX_RULES: usize = 64;

/// A parser id: `[a-z0-9][a-z0-9._-]{0,63}` (also its file name in the library).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ParserId(String);

impl ParserId {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl TryFrom<String> for ParserId {
    type Error = String;

    fn try_from(s: String) -> Result<ParserId, String> {
        let ok = (1..=64).contains(&s.len())
            && s.bytes().next().is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit())
            && s.bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || matches!(b, b'.' | b'_' | b'-'));
        if ok {
            Ok(ParserId(s))
        } else {
            Err(format!(
                "id \"{s}\" must be 1-64 characters: lower-case letters, digits, '.', '_' or '-', starting with a letter or digit"
            ))
        }
    }
}

impl From<ParserId> for String {
    fn from(id: ParserId) -> String {
        id.0
    }
}

impl std::fmt::Display for ParserId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

/// A parser's version (semver, e.g. `1.2.0`).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ParserVersion(semver::Version);

impl ParserVersion {
    pub fn new(major: u64, minor: u64, patch: u64) -> ParserVersion {
        ParserVersion(semver::Version::new(major, minor, patch))
    }

    /// The next patch version (a changed copy of a parser).
    pub fn bump_patch(&self) -> ParserVersion {
        let v = &self.0;
        ParserVersion(semver::Version::new(v.major, v.minor, v.patch.saturating_add(1)))
    }
}

impl TryFrom<String> for ParserVersion {
    type Error = String;

    fn try_from(s: String) -> Result<ParserVersion, String> {
        semver::Version::parse(s.trim())
            .map(ParserVersion)
            .map_err(|e| format!("version \"{s}\" is not a semantic version like 1.0.0 ({e})"))
    }
}

impl From<ParserVersion> for String {
    fn from(v: ParserVersion) -> String {
        v.0.to_string()
    }
}

impl std::fmt::Display for ParserVersion {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

/// One parser file (see `parsers/generic.toml` for the documented schema). Two parsers are equal when their
/// canonical TOML is.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Parser {
    pub schema: u32,
    pub id: ParserId,
    pub name: String,
    pub version: ParserVersion,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Grouping in Settings ▸ Parsers (".NET", "Web servers", ...).
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub category: String,
    #[serde(default, rename = "match")]
    pub matching: MatchCfg,
    #[serde(default)]
    pub format: FormatCfg,
    /// Format-specific extractors, run before the profile's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub extract: Vec<ExtractCfg>,
    /// Format-specific pipeline steps (e.g. a level from a status code), run before the profile's.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub pipeline: Vec<crate::pipeline::StepCfg>,
}

impl PartialEq for Parser {
    fn eq(&self, other: &Parser) -> bool {
        self.canonical_toml() == other.canonical_toml()
    }
}

/// Why a parser file was refused, with the 1-based line and column when known.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParserError {
    pub message: String,
    pub line: Option<usize>,
    pub column: Option<usize>,
}

impl ParserError {
    pub fn new(message: impl Into<String>) -> ParserError {
        ParserError {
            message: message.into(),
            line: None,
            column: None,
        }
    }

    /// An error at byte `offset` of `text`.
    fn at(message: impl Into<String>, text: &str, offset: usize) -> ParserError {
        let (line, column) = line_col(text, offset);
        ParserError {
            message: message.into(),
            line: Some(line),
            column: Some(column),
        }
    }

    /// An error on the line holding `key = ` (the first occurrence), if there is one.
    fn on_key(message: impl Into<String>, text: &str, key: &str) -> ParserError {
        match find_key(text, key) {
            Some(offset) => ParserError::at(message, text, offset),
            None => ParserError::new(message),
        }
    }
}

impl std::fmt::Display for ParserError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match (self.line, self.column) {
            (Some(l), Some(c)) => write!(f, "line {l}, column {c}: {}", self.message),
            (Some(l), None) => write!(f, "line {l}: {}", self.message),
            _ => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ParserError {}

/// 1-based line and column (in characters) of a byte offset.
fn line_col(text: &str, offset: usize) -> (usize, usize) {
    let mut offset = offset.min(text.len());
    while !text.is_char_boundary(offset) {
        offset -= 1;
    }
    let before = &text[..offset];
    let line = before.matches('\n').count() + 1;
    let column = before.rsplit('\n').next().map_or(0, |l| l.chars().count()) + 1;
    (line, column)
}

/// Byte offset of the first line that starts with `key` followed by `=`.
fn find_key(text: &str, key: &str) -> Option<usize> {
    let mut offset = 0;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if let Some(rest) = t.strip_prefix(key) {
            if rest.trim_start().starts_with('=') {
                return Some(offset + (line.len() - t.len()));
            }
        }
        offset += line.len();
    }
    None
}

impl Parser {
    /// Parses and checks a parser file: syntax, schema version, unknown keys, ids, versions and limits, then
    /// compiles it (with the default profile) so its regexes, conditions and pipeline steps are valid.
    pub fn from_toml(text: &str) -> Result<Parser, ParserError> {
        let parser = Self::parse_unchecked(text)?;
        parser.check(text)?;
        matching::CompiledMatch::new(&parser.matching).map_err(|e| ParserError::on_key(format!("{e:#}"), text, "sniff"))?;
        let profile = crate::Profile::default_profile();
        crate::profile::CompiledProfile::compile_for(profile, &parser, &Default::default())
            .map_err(|e| ParserError::new(format!("{e:#}")))?;
        Ok(parser)
    }

    /// Parses a parser file without compiling it (the syntax, schema and limits are still checked).
    pub(crate) fn parse_unchecked(text: &str) -> Result<Parser, ParserError> {
        if text.len() > MAX_PARSER_BYTES {
            return Err(ParserError::new(format!(
                "the parser file is {} KiB; at most {} KiB are read",
                text.len() / 1024,
                MAX_PARSER_BYTES / 1024
            )));
        }
        let table: toml::Table = toml::from_str(text).map_err(|e| toml_error(&e, text))?;
        match table.get("schema") {
            None => {
                return Err(ParserError::new(format!(
                    "missing `schema = {SCHEMA_VERSION}`: is this a Debrief parser file?"
                )))
            }
            Some(toml::Value::Integer(n)) if *n == SCHEMA_VERSION as i64 => {}
            Some(toml::Value::Integer(n)) => {
                return Err(ParserError::on_key(
                    format!(
                    "this parser was written for parser schema {n}; this Debrief reads schema {SCHEMA_VERSION}: update Debrief to use it"
                ),
                    text,
                    "schema",
                ))
            }
            Some(other) => return Err(ParserError::on_key(format!("schema must be a number, not {other}"), text, "schema")),
        }
        toml::from_str::<Parser>(text).map_err(|e| toml_error(&e, text))
    }

    /// Checks what serde does not: names, limits and rule counts.
    fn check(&self, text: &str) -> Result<(), ParserError> {
        if self.name.trim().is_empty() {
            return Err(ParserError::on_key("name must not be empty", text, "name"));
        }
        let formats = self.format.output_templates.len() + self.format.regexes.len();
        if formats > MAX_TEXT_FORMATS {
            return Err(ParserError::new(format!(
                "{formats} output templates and regexes; a parser has at most {MAX_TEXT_FORMATS}"
            )));
        }
        if self.extract.len() > MAX_RULES || self.pipeline.len() > MAX_RULES {
            return Err(ParserError::new(format!(
                "a parser has at most {MAX_RULES} [[extract]] and {MAX_RULES} [[pipeline]] entries"
            )));
        }
        self.matching.check().map_err(|m| ParserError::on_key(m.message, text, m.key))
    }

    /// The canonical TOML (what the content hash is computed from and what a project embeds).
    pub fn canonical_toml(&self) -> String {
        toml::to_string(self).unwrap_or_default()
    }

    /// The reference sources record: id, version and content hash.
    pub fn reference(&self) -> ParserRef {
        ParserRef {
            id: self.id.to_string(),
            version: self.version.to_string(),
            hash: content_hash(&self.canonical_toml()),
        }
    }

    /// This parser with `fragment` (TOML, e.g. `[format]\ntimestamp_formats = [...]`) merged over it: tables
    /// merge, other values replace. Errors name the line of `fragment`.
    pub fn with_override(&self, fragment: &str) -> Result<Parser, ParserError> {
        let over: toml::Table = toml::from_str(fragment).map_err(|e| toml_error(&e, fragment))?;
        let mut base = toml::Table::try_from(self).map_err(|e| ParserError::new(format!("the parser cannot be written as TOML: {e}")))?;
        crate::profile::merge(&mut base, over);
        let text = toml::to_string(&base).map_err(|e| ParserError::new(format!("the changed parser cannot be written as TOML: {e}")))?;
        Parser::from_toml(&text)
    }
}

/// A TOML error with its position.
fn toml_error(e: &toml::de::Error, text: &str) -> ParserError {
    match e.span() {
        Some(span) => ParserError::at(e.message(), text, span.start),
        None => ParserError::new(e.message()),
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// The generic parser with `fragment` (`[format]` keys) merged over it, the way tests used to write a
    /// profile `[parse]` section.
    pub fn generic_with(fragment: &str) -> Parser {
        builtin::generic().with_override(fragment).expect("valid test parser")
    }

    /// A test's TOML compiled: its `[format]` table merged over the generic parser, the rest as the profile.
    pub fn compiled(text: &str) -> crate::profile::CompiledProfile {
        let mut table: toml::Table = toml::from_str(text).expect("valid test TOML");
        let parser = match table.remove("format") {
            Some(format) => {
                let mut t = toml::Table::new();
                t.insert("format".into(), format);
                generic_with(&toml::to_string(&t).expect("TOML"))
            }
            None => builtin::generic(),
        };
        let profile = crate::Profile::from_toml(&toml::to_string(&table).expect("TOML")).expect("valid test profile");
        crate::profile::CompiledProfile::compile_for(profile, &parser, &Default::default()).expect("valid test profile")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINIMAL: &str = "schema = 1\nid = \"demo\"\nname = \"Demo\"\nversion = \"1.0.0\"\n";

    #[test]
    fn a_minimal_parser_reads_with_defaults() {
        let p = Parser::from_toml(MINIMAL).unwrap();
        assert_eq!(
            (p.id.as_str(), p.name.as_str(), p.version.to_string().as_str()),
            ("demo", "Demo", "1.0.0")
        );
        assert_eq!(p.format, FormatCfg::default());
        assert_eq!(p.matching, MatchCfg::default());
        let r = p.reference();
        assert_eq!((r.id.as_str(), r.version.as_str(), r.hash.len()), ("demo", "1.0.0", 32));
        // The canonical form reads back to the same parser and hash.
        let back = Parser::from_toml(&p.canonical_toml()).unwrap();
        assert_eq!(back, p);
        assert_eq!(back.reference(), r);
    }

    #[test]
    fn errors_carry_their_line_and_column() {
        let syntax = Parser::from_toml("schema = 1\nid = \"x\"\nname = \n").unwrap_err();
        assert_eq!(syntax.line, Some(3));
        assert!(syntax.column.is_some());
        let unknown = Parser::from_toml(&format!("{MINIMAL}[format]\nkind = \"text\"\nregexs = []\n")).unwrap_err();
        assert!(unknown.message.contains("regexs"), "{}", unknown.message);
        assert_eq!(unknown.line, Some(7));
        assert_eq!(unknown.column, Some(1));
        let bad_id = Parser::from_toml("schema = 1\nid = \"Bad Id\"\nname = \"x\"\nversion = \"1.0.0\"\n").unwrap_err();
        assert!(bad_id.message.contains("lower-case"), "{}", bad_id.message);
        assert_eq!(bad_id.line, Some(2));
        let bad_version = Parser::from_toml("schema = 1\nid = \"x\"\nname = \"x\"\nversion = \"one\"\n").unwrap_err();
        assert!(bad_version.message.contains("semantic version"));
        assert_eq!(bad_version.line, Some(4));
        let empty_name = Parser::from_toml("schema = 1\nid = \"x\"\nname = \" \"\nversion = \"1.0.0\"\n").unwrap_err();
        assert_eq!((empty_name.message.as_str(), empty_name.line), ("name must not be empty", Some(3)));
        assert_eq!(empty_name.to_string(), "line 3, column 1: name must not be empty");
    }

    #[test]
    fn schema_versions_are_checked_first() {
        let missing = Parser::from_toml("id = \"x\"").unwrap_err();
        assert!(missing.message.contains("missing `schema = 1`"));
        assert_eq!(missing.to_string(), missing.message);
        let newer = Parser::from_toml("# shared\nschema = 2\nid = \"x\"\nfuture = true\n").unwrap_err();
        assert!(
            newer.message.contains("parser schema 2") && newer.message.contains("update Debrief"),
            "{}",
            newer.message
        );
        assert_eq!(newer.line, Some(2));
        let text = Parser::from_toml("schema = \"1\"").unwrap_err();
        assert!(text.message.contains("must be a number"));
    }

    #[test]
    fn limits_and_compile_errors_are_refused() {
        let huge = format!("{MINIMAL}description = \"{}\"\n", "x".repeat(MAX_PARSER_BYTES));
        assert!(Parser::from_toml(&huge).unwrap_err().message.contains("KiB"));
        let many: Vec<String> = (0..=MAX_TEXT_FORMATS).map(|i| format!("'^(?P<message>{i}.*)$'")).collect();
        let too_many = Parser::from_toml(&format!("{MINIMAL}[format]\nregexes = [{}]\n", many.join(", "))).unwrap_err();
        assert!(too_many.message.contains("at most 64"));
        let regex = Parser::from_toml(&format!("{MINIMAL}[format]\nregexes = ['(']\n")).unwrap_err();
        assert!(regex.message.contains("format.regexes"), "{}", regex.message);
        let long = format!(
            "{MINIMAL}[format]\nregexes = ['{}']\n",
            "a".repeat(crate::user_regex::MAX_PATTERN_LEN + 1)
        );
        assert!(Parser::from_toml(&long).is_err());
        let sniff = Parser::from_toml(&format!("{MINIMAL}[match]\nsniff = '('\n")).unwrap_err();
        assert!(sniff.message.contains("sniff"));
        let rules: Vec<String> = (0..=MAX_RULES)
            .map(|i| format!("[[pipeline]]\nstep = \"drop\"\nwhen = [{{ field = \"F{i}\", op = \"exists\" }}]\n"))
            .collect();
        assert!(Parser::from_toml(&format!("{MINIMAL}{}", rules.concat()))
            .unwrap_err()
            .message
            .contains("[[pipeline]]"));
        let step = Parser::from_toml(&format!("{MINIMAL}[[pipeline]]\nstep = \"explode\"\n")).unwrap_err();
        assert!(!step.message.is_empty());
    }

    #[test]
    fn overrides_merge_over_a_parser() {
        let p = Parser::from_toml(&format!(
            "{MINIMAL}[format]\nkind = \"text\"\nregexes = ['^(?P<timestamp>\\S+) (?P<message>.*)$']\n"
        ))
        .unwrap();
        let changed = p.with_override("[format]\ntimestamp_formats = [\"%d.%m.%Y\"]\n").unwrap();
        assert_eq!(changed.format.timestamp_formats, ["%d.%m.%Y"]);
        assert_eq!(changed.format.regexes, p.format.regexes);
        assert_ne!(changed.reference().hash, p.reference().hash);
        let bad = p.with_override("[format\n").unwrap_err();
        assert_eq!(bad.line, Some(1));
        assert!(p.with_override("[format]\nnope = 1\n").is_err());
    }

    #[test]
    fn ids_and_versions() {
        assert!(ParserId::try_from("log4net-csv".to_string()).is_ok());
        assert!(ParserId::try_from("a.b_c-1".to_string()).is_ok());
        for bad in ["", "-x", "X", "a b", &"a".repeat(65)] {
            assert!(ParserId::try_from(bad.to_string()).is_err(), "{bad}");
        }
        let v = ParserVersion::try_from("1.2.3".to_string()).unwrap();
        assert_eq!(v.bump_patch().to_string(), "1.2.4");
        assert!(ParserVersion::new(1, 10, 0) > ParserVersion::new(1, 9, 9));
        assert_eq!(line_col("ab\ncé\nx", 6), (2, 3));
        assert_eq!(line_col("ab\ncé\nx", 5), (2, 2), "inside a character: its start");
        assert_eq!(line_col("ab", 99), (1, 3));
        assert_eq!(find_key("a = 1\n  b= 2\n", "b"), Some(8));
        assert_eq!(find_key("bb = 1\n", "b"), None);
    }

    #[test]
    fn errors_print_the_position_they_know() {
        let msg = |line, column| {
            ParserError {
                message: "bad".into(),
                line,
                column,
            }
            .to_string()
        };
        assert_eq!(msg(Some(3), Some(5)), "line 3, column 5: bad");
        assert_eq!(msg(Some(3), None), "line 3: bad");
        assert_eq!(msg(None, None), "bad");
    }

    #[test]
    fn a_key_that_is_not_in_the_text_gives_an_error_without_a_position() {
        let missing = ParserError::on_key("nope", "a = 1\n", "b");
        assert_eq!((missing.line, missing.column), (None, None));
        let found = ParserError::on_key("here", "a = 1\nb = 2\n", "b");
        assert_eq!(found.line, Some(2));
    }
}
