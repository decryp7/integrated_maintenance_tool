//! The parsers shipped with IMT (`crates/imt-core/parsers/*.toml`), each with a small invented sample in its
//! format (empty for binary formats). They are read-only: the library can disable them or replace one with a
//! user file of the same id.

use super::Parser;
use std::sync::OnceLock;

/// One built-in parser file and its sample.
#[derive(Debug, Clone, Copy)]
pub struct BuiltinFile {
    pub toml: &'static str,
    /// File name of the sample (used as the preview's file name).
    pub sample_name: &'static str,
    /// A few lines in the format; empty for a binary format.
    pub sample: &'static str,
}

macro_rules! builtin {
    ($id:literal, $sample:literal) => {
        BuiltinFile {
            toml: include_str!(concat!("../../parsers/", $id, ".toml")),
            sample_name: $sample,
            sample: include_str!(concat!("../../parsers/samples/", $sample)),
        }
    };
}

/// The id of the fallback parser.
pub const GENERIC: &str = "generic";

/// Every built-in parser, in library order.
pub const FILES: &[BuiltinFile] = &[
    builtin!("generic", "app.log"),
    builtin!("serilog-clef", "orders.clef"),
    builtin!("serilog-json", "gateway.json"),
    builtin!("serilog-text", "worker.txt"),
    builtin!("log4net-pattern", "billing.log"),
    builtin!("log4net-csv", "orders-log4net.csv"),
    builtin!("log4net-xml", "billing.xml"),
    builtin!("nlog", "nlog.log"),
    builtin!("mel-console", "console.log"),
    builtin!("iis-w3c", "u_ex261014.log"),
    builtin!("access-combined", "access.log"),
    builtin!("apache-error", "apache-error.log"),
    builtin!("syslog", "syslog.log"),
    builtin!("android-logcat", "logcat.txt"),
    builtin!("hadoop-spark", "spark.log"),
    BuiltinFile {
        toml: include_str!("../../parsers/windows-event.toml"),
        sample_name: "System.evtx",
        sample: "",
    },
];

/// The built-in parsers, parsed once (they are checked by the tests, so parsing cannot fail in a release).
pub fn parsers() -> &'static [Parser] {
    static PARSED: OnceLock<Vec<Parser>> = OnceLock::new();
    PARSED.get_or_init(|| {
        FILES
            .iter()
            .filter_map(|f| match Parser::parse_unchecked(f.toml) {
                Ok(p) => Some(p),
                Err(e) => {
                    log::error!("built-in parser does not parse: {e}");
                    None
                }
            })
            .collect()
    })
}

/// The built-in parser with this id.
pub fn find(id: &str) -> Option<&'static Parser> {
    parsers().iter().find(|p| p.id.as_str() == id)
}

/// The built-in file of the parser with this id.
pub fn file(id: &str) -> Option<&'static BuiltinFile> {
    FILES.iter().zip(parsers()).find(|(_, p)| p.id.as_str() == id).map(|(f, _)| f)
}

/// The fallback parser: the formats IMT understands out of the box.
pub fn generic() -> Parser {
    find(GENERIC).cloned().unwrap_or_else(|| Parser {
        schema: super::SCHEMA_VERSION,
        id: super::ParserId(GENERIC.into()),
        name: "Generic".into(),
        version: super::ParserVersion::new(1, 0, 0),
        description: String::new(),
        category: String::new(),
        matching: Default::default(),
        format: Default::default(),
        extract: Vec::new(),
        pipeline: Vec::new(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_builtin_parses_compiles_and_has_a_unique_id() {
        assert_eq!(parsers().len(), FILES.len());
        let mut ids: Vec<&str> = parsers().iter().map(|p| p.id.as_str()).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), FILES.len());
        for f in FILES {
            let p = Parser::from_toml(f.toml).unwrap_or_else(|e| panic!("{}: {e}", f.sample_name));
            assert!(!p.description.is_empty() && !p.category.is_empty(), "{}", p.id);
            assert!(f.sample.len() <= 4096, "{}", p.id);
        }
        assert_eq!(generic().id.as_str(), GENERIC);
        assert_eq!(generic().matching.priority, -100);
        assert_eq!(file("log4net-csv").map(|f| f.sample_name), Some("orders-log4net.csv"));
        assert!(find("nope").is_none() && file("nope").is_none());
    }
}
