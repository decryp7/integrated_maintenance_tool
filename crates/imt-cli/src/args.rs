//! The command line, parsed once into a [`Command`] (pure; nothing here touches the disk or the store).

use crate::time_arg::parse_time;
use crate::BIN_NAME;
use std::path::PathBuf;

/// Rows a query prints when `--limit` is absent.
pub const DEFAULT_LIMIT: usize = 1000;
/// The most rows or items any command prints.
pub const MAX_LIMIT: usize = 100_000;
/// Items per analysis when `--limit` is absent.
pub const DEFAULT_ANALYSIS_LIMIT: usize = 50;

/// What to read and where the store lives.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Source {
    /// Files and folders to import (`--open`, or the positionals of `import`).
    pub open: Vec<PathBuf>,
    /// Keep the store in this folder (reused by later runs: files already read are skipped). Without it a
    /// scratch store is made and removed at the end.
    pub store: Option<PathBuf>,
}

/// How a query's table is written.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TableFormat {
    #[default]
    Table,
    Csv,
    Json,
}

/// What `analyze` runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnalyzeKind {
    Anomalies,
    Patterns,
    Exceptions,
    /// Known issues from `--knowledge` files.
    Issues,
    /// Values over-represented among errors (or `--subset`) against the rest of the window.
    Outliers,
}

impl AnalyzeKind {
    pub fn name(self) -> &'static str {
        match self {
            AnalyzeKind::Anomalies => "anomalies",
            AnalyzeKind::Patterns => "patterns",
            AnalyzeKind::Exceptions => "exceptions",
            AnalyzeKind::Issues => "issues",
            AnalyzeKind::Outliers => "outliers",
        }
    }
}

/// The report file format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ReportFormat {
    #[default]
    Html,
    Pdf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Help,
    Version,
    Import {
        src: Source,
        json: bool,
    },
    Query {
        src: Source,
        ql: String,
        format: TableFormat,
        limit: usize,
    },
    Analyze {
        src: Source,
        kind: AnalyzeKind,
        /// A query-language filter narrowing what is analysed.
        filter: String,
        limit: usize,
        json: bool,
        knowledge: Vec<PathBuf>,
        fail_on_findings: bool,
        /// Outliers only: the selected events as a filter (empty: errors) and the fields examined (empty: all).
        subset: String,
        fields: Vec<String>,
    },
    Report {
        src: Source,
        out: PathBuf,
        format: ReportFormat,
        title: String,
        knowledge: Vec<PathBuf>,
        from: Option<i64>,
        to: Option<i64>,
        top: Option<usize>,
        overwrite: bool,
        json: bool,
    },
}

/// The usage text (`help`, and after a usage error).
pub fn usage() -> String {
    let b = BIN_NAME;
    format!(
        "usage:
  {b} import PATH... [SOURCE OPTIONS] [--json]
  {b} query QL [SOURCE OPTIONS] [--format table|csv|json] [--json] [--limit N]
  {b} analyze anomalies|patterns|exceptions|issues|outliers [SOURCE OPTIONS] [--filter QL] [--limit N] [--json]
                [--knowledge FILE]... [--fail-on-findings] [--subset QL] [--fields A,B]
  {b} report --out FILE [SOURCE OPTIONS] [--format html|pdf] [--title TEXT] [--knowledge FILE]...
                [--from TIME] [--to TIME] [--top N] [--overwrite] [--json]
  {b} help | --version

SOURCE OPTIONS (every command that reads data):
  --open PATH      a file or folder to import (repeatable)
  --store DIR      keep the store in DIR and reuse it next time (default: a scratch store, removed at the end)

Exit codes: 0 done, 1 failed, 2 wrong command line, 3 findings (with --fail-on-findings).
With --json every command prints one JSON object on standard output, errors included.
See docs/cli.md."
    )
}

/// A usage error: what is wrong, for the user to fix.
pub type UsageError = String;

struct Cursor<'a> {
    args: &'a [String],
    pos: usize,
}

impl<'a> Cursor<'a> {
    fn next(&mut self) -> Option<&'a str> {
        let a = self.args.get(self.pos)?;
        self.pos += 1;
        Some(a)
    }

    fn value(&mut self, flag: &str) -> Result<&'a str, UsageError> {
        self.next().ok_or_else(|| format!("{flag} needs a value"))
    }
}

fn number(flag: &str, text: &str) -> Result<usize, UsageError> {
    text.parse::<usize>()
        .map_err(|_| format!("{flag} needs a whole number, not \"{text}\""))
}

fn limit(flag: &str, text: &str) -> Result<usize, UsageError> {
    let n = number(flag, text)?;
    if n == 0 || n > MAX_LIMIT {
        return Err(format!("{flag} must be between 1 and {MAX_LIMIT}"));
    }
    Ok(n)
}

/// Every option any command takes; a command refuses the ones it does not use.
#[derive(Default)]
struct Options {
    src: Source,
    json: bool,
    format: Option<String>,
    limit: Option<usize>,
    filter: Option<String>,
    knowledge: Vec<PathBuf>,
    fail_on_findings: bool,
    subset: Option<String>,
    fields: Vec<String>,
    out: Option<PathBuf>,
    title: Option<String>,
    from: Option<i64>,
    to: Option<i64>,
    top: Option<usize>,
    overwrite: bool,
    positional: Vec<String>,
    used: Vec<&'static str>,
}

fn collect(cur: &mut Cursor<'_>) -> Result<Options, UsageError> {
    let mut o = Options::default();
    while let Some(a) = cur.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) if f.starts_with("--") => (f, Some(v)),
            _ => (a, None),
        };
        let val = |cur: &mut Cursor<'_>| -> Result<String, UsageError> {
            match inline {
                Some(v) => Ok(v.to_string()),
                None => cur.value(flag).map(str::to_string),
            }
        };
        match flag {
            "--open" => {
                o.src.open.push(PathBuf::from(val(cur)?));
                o.used.push("--open");
            }
            "--store" => {
                o.src.store = Some(PathBuf::from(val(cur)?));
                o.used.push("--store");
            }
            "--json" => {
                o.json = true;
                o.used.push("--json");
            }
            "--format" => {
                o.format = Some(val(cur)?);
                o.used.push("--format");
            }
            "--limit" => {
                o.limit = Some(limit("--limit", &val(cur)?)?);
                o.used.push("--limit");
            }
            "--filter" => {
                o.filter = Some(val(cur)?);
                o.used.push("--filter");
            }
            "--knowledge" => {
                o.knowledge.push(PathBuf::from(val(cur)?));
                o.used.push("--knowledge");
            }
            "--fail-on-findings" => {
                o.fail_on_findings = true;
                o.used.push("--fail-on-findings");
            }
            "--subset" => {
                o.subset = Some(val(cur)?);
                o.used.push("--subset");
            }
            "--fields" => {
                o.fields = val(cur)?
                    .split(',')
                    .map(|f| f.trim().to_string())
                    .filter(|f| !f.is_empty())
                    .collect();
                o.used.push("--fields");
            }
            "--out" => {
                o.out = Some(PathBuf::from(val(cur)?));
                o.used.push("--out");
            }
            "--title" => {
                o.title = Some(val(cur)?);
                o.used.push("--title");
            }
            "--from" => {
                o.from = Some(parse_time(&val(cur)?).map_err(|e| format!("--from: {e}"))?);
                o.used.push("--from");
            }
            "--to" => {
                o.to = Some(parse_time(&val(cur)?).map_err(|e| format!("--to: {e}"))?);
                o.used.push("--to");
            }
            "--top" => {
                o.top = Some(number("--top", &val(cur)?)?);
                o.used.push("--top");
            }
            "--overwrite" => {
                o.overwrite = true;
                o.used.push("--overwrite");
            }
            f if f.starts_with("--") => return Err(format!("unknown option {f}")),
            _ => o.positional.push(a.to_string()),
        }
    }
    Ok(o)
}

fn only(o: &Options, command: &str, allowed: &[&str]) -> Result<(), UsageError> {
    match o.used.iter().find(|u| !allowed.contains(u)) {
        Some(bad) => Err(format!("{command} does not take {bad}")),
        None => Ok(()),
    }
}

const SOURCE_FLAGS: [&str; 2] = ["--open", "--store"];

fn allowed(extra: &[&'static str]) -> Vec<&'static str> {
    SOURCE_FLAGS.iter().chain(extra.iter()).copied().collect()
}

/// Parses the arguments after the program name.
pub fn parse(args: &[String]) -> Result<Command, UsageError> {
    let mut cur = Cursor { args, pos: 0 };
    let Some(first) = cur.next() else { return Ok(Command::Help) };
    match first {
        "help" | "--help" | "-h" => Ok(Command::Help),
        "--version" | "-V" | "version" => Ok(Command::Version),
        "import" => {
            let mut o = collect(&mut cur)?;
            only(&o, "import", &allowed(&["--json"]))?;
            o.src.open.extend(o.positional.drain(..).map(PathBuf::from));
            if o.src.open.is_empty() {
                return Err("import needs at least one file or folder".into());
            }
            Ok(Command::Import { src: o.src, json: o.json })
        }
        "query" => {
            let mut o = collect(&mut cur)?;
            only(&o, "query", &allowed(&["--json", "--format", "--limit"]))?;
            if o.positional.len() != 1 {
                return Err("query needs the query text as one argument (quote it)".into());
            }
            let format = match (o.format.as_deref(), o.json) {
                (Some("table"), false) | (None, false) => TableFormat::Table,
                (Some("csv"), false) => TableFormat::Csv,
                (Some("json"), _) | (None, true) => TableFormat::Json,
                (Some(f), _) => return Err(format!("--format is table, csv or json, not \"{f}\"")),
            };
            Ok(Command::Query {
                src: o.src,
                ql: o.positional.remove(0),
                format,
                limit: o.limit.unwrap_or(DEFAULT_LIMIT),
            })
        }
        "analyze" | "analyse" => {
            let o = collect(&mut cur)?;
            only(
                &o,
                "analyze",
                &allowed(&[
                    "--json",
                    "--limit",
                    "--filter",
                    "--knowledge",
                    "--fail-on-findings",
                    "--subset",
                    "--fields",
                ]),
            )?;
            if o.positional.len() != 1 {
                return Err("analyze needs one of anomalies, patterns, exceptions, issues, outliers".into());
            }
            let kind = match o.positional[0].as_str() {
                "anomalies" => AnalyzeKind::Anomalies,
                "patterns" => AnalyzeKind::Patterns,
                "exceptions" => AnalyzeKind::Exceptions,
                "issues" => AnalyzeKind::Issues,
                "outliers" => AnalyzeKind::Outliers,
                other => {
                    return Err(format!(
                        "unknown analysis \"{other}\": use anomalies, patterns, exceptions, issues or outliers"
                    ))
                }
            };
            if kind == AnalyzeKind::Patterns && o.fail_on_findings {
                return Err("--fail-on-findings is for anomalies, exceptions and issues (every log has patterns)".into());
            }
            if kind != AnalyzeKind::Outliers && (o.subset.is_some() || !o.fields.is_empty()) {
                return Err("--subset and --fields are for analyze outliers".into());
            }
            if kind == AnalyzeKind::Issues && o.knowledge.is_empty() {
                return Err("analyze issues needs --knowledge FILE (a Markdown file declaring known issues)".into());
            }
            if kind != AnalyzeKind::Issues && !o.knowledge.is_empty() {
                return Err("--knowledge is for analyze issues".into());
            }
            Ok(Command::Analyze {
                src: o.src,
                kind,
                filter: o.filter.unwrap_or_default(),
                limit: o.limit.unwrap_or(DEFAULT_ANALYSIS_LIMIT),
                json: o.json,
                knowledge: o.knowledge,
                fail_on_findings: o.fail_on_findings,
                subset: o.subset.unwrap_or_default(),
                fields: o.fields,
            })
        }
        "report" => {
            let o = collect(&mut cur)?;
            only(
                &o,
                "report",
                &allowed(&[
                    "--json",
                    "--out",
                    "--format",
                    "--title",
                    "--knowledge",
                    "--from",
                    "--to",
                    "--top",
                    "--overwrite",
                ]),
            )?;
            if let Some(p) = o.positional.first() {
                return Err(format!("report takes no argument (\"{p}\"); use --open PATH to read files"));
            }
            let out = o.out.ok_or("report needs --out FILE")?;
            let format = match o.format.as_deref() {
                Some("html") => ReportFormat::Html,
                Some("pdf") => ReportFormat::Pdf,
                None => match out.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).as_deref() {
                    Some("pdf") => ReportFormat::Pdf,
                    _ => ReportFormat::Html,
                },
                Some(f) => return Err(format!("--format is html or pdf, not \"{f}\"")),
            };
            if let (Some(f), Some(t)) = (o.from, o.to) {
                if t < f {
                    return Err("--to is before --from".into());
                }
            }
            if matches!(o.top, Some(0) | Some(11..)) {
                return Err("--top is between 1 and 10".into());
            }
            Ok(Command::Report {
                src: o.src,
                out,
                format,
                title: o.title.unwrap_or_default(),
                knowledge: o.knowledge,
                from: o.from,
                to: o.to,
                top: o.top,
                overwrite: o.overwrite,
                json: o.json,
            })
        }
        other => Err(format!("unknown command \"{other}\"")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> Result<Command, UsageError> {
        parse(&s.split_whitespace().map(str::to_string).collect::<Vec<_>>())
    }

    #[test]
    fn no_arguments_is_help() {
        assert_eq!(p("").unwrap(), Command::Help);
        assert_eq!(p("--version").unwrap(), Command::Version);
    }

    #[test]
    fn import_takes_paths_and_a_store() {
        let Command::Import { src, json } = p("import a.log dir --store s --json").unwrap() else {
            panic!()
        };
        assert_eq!(src.open, [PathBuf::from("a.log"), PathBuf::from("dir")]);
        assert_eq!(src.store, Some(PathBuf::from("s")));
        assert!(json);
        assert!(p("import").unwrap_err().contains("at least one"));
    }

    #[test]
    fn query_has_a_format_and_a_bounded_limit() {
        let Command::Query { ql, format, limit, .. } = p("query level=error --open x --format csv --limit 5").unwrap() else {
            panic!()
        };
        assert_eq!((ql.as_str(), format, limit), ("level=error", TableFormat::Csv, 5));
        let Command::Query { format, limit, .. } = p("query q --json").unwrap() else {
            panic!()
        };
        assert_eq!((format, limit), (TableFormat::Json, DEFAULT_LIMIT));
        assert!(p("query").unwrap_err().contains("query text"));
        assert!(p("query a b").is_err());
        assert!(p("query a --limit 0").unwrap_err().contains("between"));
        assert!(p("query a --limit 100001").is_err());
        assert!(p("query a --format xml").unwrap_err().contains("table, csv or json"));
        assert!(p("query a --out f").unwrap_err().contains("does not take --out"));
        assert!(p("query a --bogus").unwrap_err().contains("unknown option"));
        assert!(p("query a --limit").unwrap_err().contains("needs a value"));
    }

    #[test]
    fn analyze_checks_its_kind_and_knowledge() {
        let Command::Analyze {
            kind,
            limit,
            fail_on_findings,
            ..
        } = p("analyze anomalies --fail-on-findings").unwrap()
        else {
            panic!()
        };
        assert_eq!(
            (kind, limit, fail_on_findings),
            (AnalyzeKind::Anomalies, DEFAULT_ANALYSIS_LIMIT, true)
        );
        assert!(p("analyze issues").unwrap_err().contains("--knowledge"));
        assert!(p("analyze patterns --knowledge k.md").unwrap_err().contains("analyze issues"));
        assert!(p("analyze nonsense").unwrap_err().contains("unknown analysis"));
        assert!(p("analyze").is_err());
        assert!(p("analyze patterns --fail-on-findings").unwrap_err().contains("every log"));
        assert!(p("analyze issues --knowledge k.md").is_ok());
        let Command::Analyze { kind, subset, fields, .. } = p("analyze outliers --subset Host=B --fields Build,Region").unwrap() else {
            panic!()
        };
        assert_eq!((kind, subset.as_str(), fields.len()), (AnalyzeKind::Outliers, "Host=B", 2));
        assert!(p("analyze patterns --subset x").unwrap_err().contains("analyze outliers"));
    }

    #[test]
    fn report_needs_an_output_and_infers_the_format() {
        let Command::Report { format, out, .. } = p("report --out r.pdf").unwrap() else {
            panic!()
        };
        assert_eq!((format, out), (ReportFormat::Pdf, PathBuf::from("r.pdf")));
        let Command::Report { format, .. } = p("report --out r.html --format pdf").unwrap() else {
            panic!()
        };
        assert_eq!(format, ReportFormat::Pdf);
        let Command::Report { format, .. } = p("report --out r").unwrap() else {
            panic!()
        };
        assert_eq!(format, ReportFormat::Html);
        assert!(p("report").unwrap_err().contains("--out"));
        assert!(p("report --out r --top 11").is_err());
        assert!(p("report --out r --from 2026-10-02 --to 2026-10-01").is_err());
        assert!(p("report x --out r").is_err());
        assert!(p("report --out r --format doc").is_err());
    }

    #[test]
    fn unknown_commands_are_refused() {
        assert!(p("frobnicate").unwrap_err().contains("unknown command"));
        assert!(usage().contains(BIN_NAME));
    }
}
