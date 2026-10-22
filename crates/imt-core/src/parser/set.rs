//! The parser library as the engine uses it: every parser compiled with the profile, and the choice of a
//! parser per file (ADR 0026 §3).
//!
//! A parser's score for a file is the points of its `[match]` rules (each must hold) plus how well its
//! format reads the file's first lines: text formats `70 + 30 × share of lines they match`, other kinds
//! their importer's sniff (0-100). The highest score wins; ties go to the higher priority, then user parsers
//! before built-ins, then library order. A per-source override ("Parse with…") wins over the score.

use super::matching::{CompiledMatch, FileHead};
use super::Parser;
use crate::importers::{first_lines, ImportCtx, Importers};
use crate::ports::ParserRef;
use crate::profile::{CompiledProfile, Profile};
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

/// Characters of a file's start the parsers are scored on (as the importers are chosen from).
pub const SCORE_CHARS: usize = 64 * 1024;
/// Lines a text format's share is counted on.
const SCORE_LINES: usize = 50;

/// What the app hands the engine: the library's parsers and the per-source choices.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ParserSetup {
    pub parsers: Vec<SetupParser>,
    /// Source path → parser id ("Parse with…"); the parser may be disabled.
    pub overrides: BTreeMap<String, String>,
}

/// One parser of a [`ParserSetup`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupParser {
    pub toml: String,
    pub enabled: bool,
    /// A user parser (wins ties against built-ins).
    pub user: bool,
}

impl ParserSetup {
    /// The built-in parsers, all enabled, no overrides.
    pub fn builtins() -> ParserSetup {
        ParserSetup {
            parsers: super::builtin::FILES
                .iter()
                .map(|f| SetupParser {
                    toml: f.toml.to_string(),
                    enabled: true,
                    user: false,
                })
                .collect(),
            overrides: BTreeMap::new(),
        }
    }

    /// Only `parsers` (enabled user parsers), e.g. for tests and previews.
    pub fn of(parsers: &[Parser]) -> ParserSetup {
        ParserSetup {
            parsers: parsers
                .iter()
                .map(|p| SetupParser {
                    toml: p.canonical_toml(),
                    enabled: true,
                    user: true,
                })
                .collect(),
            overrides: BTreeMap::new(),
        }
    }
}

/// How a profile is compiled with a parser (the plugin registry's markers and transforms, or none).
pub type CompileFn<'a> = &'a dyn Fn(Profile, &Parser) -> Result<CompiledProfile>;

struct Entry {
    parser: Parser,
    reference: ParserRef,
    enabled: bool,
    user: bool,
    matcher: CompiledMatch,
    compiled: Arc<CompiledProfile>,
}

/// The parser chosen for a file.
#[derive(Clone, Copy)]
pub struct Choice<'a> {
    pub parser: &'a Parser,
    pub reference: &'a ParserRef,
    /// The profile compiled with this parser: what reads the file.
    pub profile: &'a Arc<CompiledProfile>,
    /// The score it won with (0 for an override).
    pub score: u32,
}

/// A file to choose a parser for.
pub struct Candidate<'a> {
    /// The source path (overrides are keyed by it).
    pub source: &'a str,
    pub path: &'a Path,
    /// The path relative to the opened folder (the display name).
    pub rel: &'a str,
    pub file_date: Option<chrono::NaiveDate>,
    /// The file's first bytes (binary importers sniff them).
    pub bytes: &'a [u8],
    /// Its decoded start, at most [`SCORE_CHARS`]; `None` when it is not text.
    pub text: Option<&'a str>,
}

/// Every parser of the library compiled with the profile, and the per-source overrides.
pub struct ParserSet {
    entries: Vec<Entry>,
    overrides: BTreeMap<String, String>,
    setup: ParserSetup,
    /// The profile with the generic parser: for what does not depend on a file's format.
    base: Arc<CompiledProfile>,
}

impl ParserSet {
    /// Compiles `setup` with `profile`. A parser that does not parse or compile is left out (logged): the
    /// library listed it as broken already. Fails only when the profile itself does not compile.
    pub fn compile(profile: &Profile, setup: &ParserSetup, compile: CompileFn) -> Result<ParserSet> {
        let base = Arc::new(compile(profile.clone(), &super::builtin::generic())?);
        let mut entries: Vec<Entry> = Vec::new();
        for p in &setup.parsers {
            let parser = match Parser::parse_unchecked(&p.toml) {
                Ok(parser) => parser,
                Err(e) => {
                    log::warn!("a parser of the library does not parse and is left out: {e}");
                    continue;
                }
            };
            if entries.iter().any(|e| e.parser.id == parser.id) {
                log::warn!(parser = parser.id.as_str(); "a second parser with this id is left out");
                continue;
            }
            let compiled = super::matching::CompiledMatch::new(&parser.matching).and_then(|m| Ok((m, compile(profile.clone(), &parser)?)));
            match compiled {
                Ok((matcher, compiled)) => entries.push(Entry {
                    reference: parser.reference(),
                    parser,
                    enabled: p.enabled,
                    user: p.user,
                    matcher,
                    compiled: Arc::new(compiled),
                }),
                Err(e) => log::warn!(parser = parser.id.as_str(); "a parser of the library does not compile and is left out: {e:#}"),
            }
        }
        Ok(ParserSet {
            entries,
            overrides: setup.overrides.clone(),
            setup: setup.clone(),
            base,
        })
    }

    /// The built-in parsers with `profile`, compiled without plugins.
    pub fn builtins(profile: &Profile) -> Result<ParserSet> {
        Self::compile(profile, &ParserSetup::builtins(), &|p, parser| {
            CompiledProfile::compile_for(p, parser, &Default::default())
        })
    }

    /// One parser, already compiled with the profile (the generic parser's rules, the compiled format): what
    /// a preview or a test reads with.
    pub fn of_compiled(compiled: CompiledProfile) -> ParserSet {
        let mut parser = super::builtin::generic();
        parser.format = compiled.format.clone();
        let compiled = Arc::new(compiled);
        let matcher = CompiledMatch::new(&Default::default()).expect("no rules always compile");
        ParserSet {
            entries: vec![Entry {
                reference: compiled.parser.clone(),
                parser,
                enabled: true,
                user: false,
                matcher,
                compiled: compiled.clone(),
            }],
            overrides: BTreeMap::new(),
            setup: ParserSetup::default(),
            base: compiled,
        }
    }

    /// The same library compiled with another profile.
    pub fn with_profile(&self, profile: &Profile, compile: CompileFn) -> Result<ParserSet> {
        Self::compile(profile, &self.setup, compile)
    }

    /// What this set was compiled from.
    pub fn setup(&self) -> &ParserSetup {
        &self.setup
    }

    /// The profile compiled with the generic parser (format-independent analysis: markers, charts, ...).
    pub fn base(&self) -> &Arc<CompiledProfile> {
        &self.base
    }

    /// Every parser (enabled or not), in library order.
    pub fn parsers(&self) -> impl Iterator<Item = (&Parser, &ParserRef, bool)> {
        self.entries.iter().map(|e| (&e.parser, &e.reference, e.enabled))
    }

    /// The compiled profile of the parser with this id.
    pub fn profile_of(&self, id: &str) -> Option<&Arc<CompiledProfile>> {
        self.entry(id).map(|e| &e.compiled)
    }

    /// The parser with this id.
    pub fn parser(&self, id: &str) -> Option<(&Parser, &ParserRef)> {
        self.entry(id).map(|e| (&e.parser, &e.reference))
    }

    fn entry(&self, id: &str) -> Option<&Entry> {
        self.entries.iter().find(|e| e.parser.id.as_str() == id)
    }

    /// The parser overriding the choice for this source, if one is set (and in the set).
    pub fn override_of(&self, source: &str) -> Option<&str> {
        self.overrides.get(source).map(String::as_str).filter(|id| self.entry(id).is_some())
    }

    /// Identity fields of the parser with this id (they join the instance candidates).
    pub fn identity_of(&self, id: &str) -> &[String] {
        self.entry(id).map_or(&[], |e| &e.parser.format.identity)
    }

    /// The parser for a file: the override for its source, else the best scoring enabled parser. The error
    /// is the reason shown for an unrecognised file.
    pub fn choose(&self, file: &Candidate, importers: &Importers) -> Result<Choice<'_>, String> {
        if let Some(e) = self.override_of(file.source).and_then(|id| self.entry(id)) {
            return Ok(e.choice(0));
        }
        let name = file.path.file_name().map(|n| n.to_string_lossy()).unwrap_or_default();
        let head = FileHead {
            name: &name,
            rel: file.rel,
            text: file.text,
        };
        let mut best: Option<(u32, &Entry, usize)> = None;
        for (i, e) in self.entries.iter().enumerate().filter(|(_, e)| e.enabled) {
            let Some(rules) = e.matcher.score(&head, &e.parser.format.csv.delimiter) else {
                continue;
            };
            let mut format = format_score(&e.compiled, importers, file);
            // A parser naming an importer reads the files its rules select even when that importer's sniff
            // does not recognise them (the user said what they are).
            if format == 0 && e.matcher.has_rules() && names_importer(&e.compiled.format.kind, importers) {
                format = 1;
            }
            if format == 0 {
                continue;
            }
            let score = rules + format;
            let better = match best {
                None => true,
                Some((s, b, j)) => {
                    (score, e.matcher.priority, e.user, std::cmp::Reverse(i)) > (s, b.matcher.priority, b.user, std::cmp::Reverse(j))
                }
            };
            if better {
                best = Some((score, e, i));
            }
        }
        best.map(|(score, e, _)| e.choice(score))
            .ok_or_else(|| "no enabled parser reads this file (see Settings ▸ Parsers)".to_string())
    }

    /// The sources whose recorded parser is not the one this set reads them with: overridden to another
    /// parser, or recorded with a parser that is now missing, disabled or changed. `recorded` is
    /// `(source path, parser)` per source.
    pub fn stale<'r, T: Copy>(&self, recorded: impl IntoIterator<Item = (T, &'r str, &'r ParserRef)>) -> Vec<T> {
        recorded
            .into_iter()
            .filter(|(_, path, used)| match self.override_of(path) {
                Some(id) => self.entry(id).is_none_or(|e| e.reference != **used),
                None => self.entry(&used.id).is_none_or(|e| !e.enabled || e.reference != **used),
            })
            .map(|(t, _, _)| t)
            .collect()
    }
}

impl Entry {
    fn choice(&self, score: u32) -> Choice<'_> {
        Choice {
            parser: &self.parser,
            reference: &self.reference,
            profile: &self.compiled,
            score,
        }
    }
}

/// Whether `kind` names an installed importer (not `auto` or `text`).
fn names_importer(kind: &str, importers: &Importers) -> bool {
    !matches!(kind, "auto" | "text")
        && (importers.text_importers().any(|i| i.id() == kind) || importers.binary_importers().any(|i| i.id() == kind))
}

/// How well a parser's format reads a file's start (0: not at all).
fn format_score(profile: &CompiledProfile, importers: &Importers, file: &Candidate) -> u32 {
    let ctx = ImportCtx {
        profile,
        path: file.path,
        file_date: file.file_date,
    };
    let text_sniff = |id: Option<&str>| -> u32 {
        let Some(text) = file.text else { return 0 };
        importers
            .text_importers()
            .filter(|i| id.is_none_or(|id| i.id() == id))
            .map(|i| u32::from(i.sniff(&ctx, text)))
            .max()
            .unwrap_or(0)
    };
    let binary_sniff = |id: Option<&str>| -> u32 {
        importers
            .binary_importers()
            .filter(|i| id.is_none_or(|id| i.id() == id))
            .map(|i| u32::from(i.sniff(&ctx, file.bytes)))
            .max()
            .unwrap_or(0)
    };
    match profile.format.kind.as_str() {
        // As the importers would be chosen: binary ones first.
        "auto" => match binary_sniff(None) {
            0 => text_sniff(None),
            n => n,
        },
        "text" => file.text.map_or(0, |t| text_score(profile, t)),
        id => match binary_sniff(Some(id)) {
            0 => text_sniff(Some(id)),
            n => n,
        },
    }
}

/// How well the text formats of a parser read the first lines: the share of the lines that start an event
/// (those a format reads or that look like an event start) it reads. All of them: 100 (above
/// what the importers' own sniff gives the generic parser); fewer: `40 × share`, so a parser that misses
/// event lines only reads a file nothing else reads. 0 when it reads none.
fn text_score(profile: &CompiledProfile, text: &str) -> u32 {
    let reads = |l: &str| match &profile.text_format_set {
        Some(set) => set.is_match(l),
        None => profile.text_formats.iter().any(|f| f.regex.is_match(l)),
    };
    let (mut hits, mut starts) = (0usize, 0usize);
    for l in first_lines(text, SCORE_LINES) {
        if reads(l) {
            hits += 1;
            starts += 1;
        } else if crate::importers::lines::looks_like_event_start(l) || crate::importers::lines::json_event(profile, l).is_some() {
            starts += 1;
        }
    }
    if hits == 0 {
        return 0;
    }
    let share = hits as f64 / starts as f64;
    if hits == starts {
        100
    } else {
        (40.0 * share).round().max(1.0) as u32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::builtin;

    fn set(setup: &ParserSetup) -> ParserSet {
        ParserSet::compile(&Profile::default_profile(), setup, &|p, parser| {
            CompiledProfile::compile_for(p, parser, &Default::default())
        })
        .unwrap()
    }

    fn choose<'s>(set: &'s ParserSet, name: &str, text: &str) -> Result<Choice<'s>, String> {
        let path = Path::new(name);
        set.choose(
            &Candidate {
                source: name,
                path,
                rel: name,
                file_date: None,
                bytes: text.as_bytes(),
                text: Some(text),
            },
            &Importers::default(),
        )
    }

    fn parser(toml: &str) -> Parser {
        Parser::from_toml(&format!("schema = 1\nversion = \"1.0.0\"\n{toml}")).unwrap()
    }

    #[test]
    fn every_builtin_text_sample_is_read_by_its_parser() {
        let s = set(&ParserSetup::builtins());
        // Formats only a plugin reads go to the generic parser without the plugin.
        let plugin_formats = ["log4net-xml", "iis-w3c", "access-combined", "syslog", "windows-event"];
        for (f, p) in builtin::FILES.iter().zip(builtin::parsers()) {
            if f.sample.is_empty() || plugin_formats.contains(&p.id.as_str()) {
                continue;
            }
            let chosen = choose(&s, f.sample_name, f.sample).unwrap();
            assert_eq!(chosen.parser.id, p.id, "{}", f.sample_name);
            assert!(chosen.score > 0);
        }
        // A CSV file with a header and a JSON file of another logger are the generic parser's.
        let csv = choose(&s, "events.csv", include_str!("../../parsers/samples/events.csv")).unwrap();
        assert_eq!(csv.parser.id.as_str(), "generic");
        let ecs = choose(&s, "app.json", "{\"@timestamp\":\"2026-10-14T10:00:00Z\",\"message\":\"x\"}\n").unwrap();
        assert_eq!(ecs.parser.id.as_str(), "generic");
        assert_eq!(
            choose(&s, "notes.txt", "just words\n").err().unwrap(),
            "no enabled parser reads this file (see Settings ▸ Parsers)"
        );
    }

    #[test]
    fn a_folder_of_mixed_files_gets_a_parser_per_file() {
        let s = set(&ParserSetup::builtins());
        let files = [
            (
                "a/orders.clef",
                "{\"@t\":\"2026-10-14T10:00:00Z\",\"@mt\":\"Started {App}\",\"App\":\"x\"}\n",
                "serilog-clef",
            ),
            (
                "b/orders.csv",
                "2025/11/08,00:00:00.013+01:00,INFO,1,Svc,1,Started\n2025/11/08,00:00:01.013+01:00,WARN,2,Svc,1,Slow\n",
                "log4net-csv",
            ),
            ("c/app.log", "2026-10-14 10:00:00.000 +00:00 [INF] Started\n", "serilog-text"),
            ("d/web.log", "2026-10-14 10:00:00|INFO|Web.Startup|Listening\n", "nlog"),
        ];
        for (name, text, want) in files {
            assert_eq!(choose(&s, name, text).unwrap().parser.id.as_str(), want, "{name}");
        }
    }

    #[test]
    fn disabled_parsers_are_skipped_and_overrides_win() {
        let mut setup = ParserSetup::builtins();
        for p in &mut setup.parsers {
            if p.toml.contains("id = \"serilog-clef\"") {
                p.enabled = false;
            }
        }
        let clef = "{\"@t\":\"2026-10-14T10:00:00Z\",\"@mt\":\"x\"}\n";
        assert_eq!(choose(&set(&setup), "a.clef", clef).unwrap().parser.id.as_str(), "generic");
        // Parse with…: a disabled parser may be chosen for one source.
        setup.overrides.insert("a.clef".into(), "serilog-clef".into());
        let s = set(&setup);
        let c = choose(&s, "a.clef", clef).unwrap();
        assert_eq!((c.parser.id.as_str(), c.score), ("serilog-clef", 0));
        assert_eq!(s.override_of("a.clef"), Some("serilog-clef"));
        // An override naming a parser not in the library is ignored.
        setup.overrides.insert("a.clef".into(), "gone".into());
        let s = set(&setup);
        assert_eq!(s.override_of("a.clef"), None);
        assert_eq!(choose(&s, "a.clef", clef).unwrap().parser.id.as_str(), "generic");
        // Nothing enabled: nothing is read.
        let none = ParserSetup {
            parsers: setup.parsers.iter().cloned().map(|p| SetupParser { enabled: false, ..p }).collect(),
            ..Default::default()
        };
        assert!(choose(&set(&none), "a.log", "2026-10-14 10:00:00 INFO a\n").is_err());
    }

    #[test]
    fn ties_go_to_priority_then_user_parsers() {
        let a = parser("id = \"a\"\nname = \"A\"\n[format]\nkind = \"text\"\nregexes = ['^(?P<timestamp>\\S+ \\S+) (?P<message>.*)$']\n");
        let b = parser("id = \"b\"\nname = \"B\"\n[format]\nkind = \"text\"\nregexes = ['^(?P<timestamp>\\S+ \\S+) (?P<message>.*)$']\n");
        let line = "2026-10-14 10:00:00 hello\n";
        // Same score, same priority: library order.
        assert_eq!(
            choose(&set(&ParserSetup::of(&[a.clone(), b.clone()])), "x", line)
                .unwrap()
                .parser
                .id
                .as_str(),
            "a"
        );
        // A user parser wins over a built-in.
        let mut setup = ParserSetup::of(&[a.clone(), b.clone()]);
        setup.parsers[0].user = false;
        assert_eq!(choose(&set(&setup), "x", line).unwrap().parser.id.as_str(), "b");
        // Priority before that.
        let mut high = a.clone();
        high.matching.priority = 5;
        setup.parsers[0].toml = high.canonical_toml();
        assert_eq!(choose(&set(&setup), "x", line).unwrap().parser.id.as_str(), "a");
        // Rules add points: a file glob decides.
        let mut named = b.clone();
        named.matching.files = vec!["*.app".into()];
        let s = set(&ParserSetup::of(&[a.clone(), named]));
        assert_eq!(choose(&s, "x.app", line).unwrap().parser.id.as_str(), "b");
        assert_eq!(choose(&s, "x.log", line).unwrap().parser.id.as_str(), "a");
    }

    #[test]
    fn broken_and_duplicate_parsers_are_left_out() {
        let ok = parser("id = \"ok\"\nname = \"Ok\"\n[format]\nkind = \"text\"\nregexes = ['^(?P<message>.+)$']\n");
        let mut setup = ParserSetup::of(&[ok.clone(), ok.clone()]);
        setup.parsers.push(SetupParser {
            toml: "schema = 1\nid = \"broken\"\n".into(),
            enabled: true,
            user: true,
        });
        setup.parsers.push(SetupParser {
            toml: "schema = 1\nid = \"badre\"\nname = \"x\"\nversion = \"1.0.0\"\n[format]\nregexes = ['(']\n".into(),
            enabled: true,
            user: true,
        });
        let s = set(&setup);
        assert_eq!(s.parsers().count(), 1);
        assert_eq!(s.setup(), &setup);
        assert!(s.parser("ok").is_some() && s.parser("broken").is_none());
        assert!(s.profile_of("ok").is_some());
        assert!(s.identity_of("nope").is_empty());
        assert_eq!(s.base().parser.id, "generic");
        let again = s
            .with_profile(&Profile::default_profile(), &|p, parser| {
                CompiledProfile::compile_for(p, parser, &Default::default())
            })
            .unwrap();
        assert_eq!(again.parsers().count(), 1);
    }

    #[test]
    fn stale_sources_are_those_read_with_another_parser() {
        let s = set(&ParserSetup::builtins());
        let clef = s.parser("serilog-clef").unwrap().1.clone();
        let changed = ParserRef {
            hash: "other".into(),
            ..clef.clone()
        };
        let gone = ParserRef {
            id: "gone".into(),
            ..clef.clone()
        };
        let recorded = [(1, "a", &clef), (2, "b", &changed), (3, "c", &gone)];
        assert_eq!(s.stale(recorded), vec![2, 3]);
        let mut setup = ParserSetup::builtins();
        setup.overrides.insert("a".into(), "generic".into());
        for p in &mut setup.parsers {
            p.enabled = !p.toml.contains("id = \"serilog-clef\"");
        }
        let s = set(&setup);
        assert_eq!(s.stale([(1, "a", &clef), (4, "d", &clef)]), vec![1, 4]);
        let generic = s.parser("generic").unwrap().1.clone();
        assert!(s.stale([(1, "a", &generic)]).is_empty());
    }

    #[test]
    fn binary_files_score_by_their_bytes_and_plugin_kinds_need_their_importer() {
        let s = set(&ParserSetup::builtins());
        let path = Path::new("System.evtx");
        let c = Candidate {
            source: "System.evtx",
            path,
            rel: "System.evtx",
            file_date: None,
            bytes: b"ElfFile\0",
            text: None,
        };
        // Without the Windows plugin nothing reads EVTX.
        assert!(s.choose(&c, &Importers::default()).is_err());
        let p = parser("id = \"z\"\nname = \"Z\"\n[format]\nkind = \"text\"\nregexes = ['^(?P<message>.+)$']\n");
        let s = set(&ParserSetup::of(&[p]));
        assert!(s.choose(&c, &Importers::default()).is_err());
    }
}
