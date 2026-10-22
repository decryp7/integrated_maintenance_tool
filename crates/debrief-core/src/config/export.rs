//! Writing a diff as CSV or as a self-contained HTML page. Only rows passing
//! the view's kind and search filters are written (no folding, no paging).

use super::diff::Diff;
use super::dto::{ChangeKind, DiffView, ValueDto};
use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};
use std::io::Write;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum DiffExportFormat {
    Csv,
    Html,
}

impl DiffExportFormat {
    pub fn extension(self) -> &'static str {
        match self {
            DiffExportFormat::Csv => "csv",
            DiffExportFormat::Html => "html",
        }
    }
}

/// Labels of the two files, for the HTML header.
pub struct Sides<'a> {
    pub left: &'a str,
    pub right: &'a str,
}

/// Writes the filtered diff; returns the number of rows written.
pub fn write_diff(diff: &Diff, view: &DiffView, format: DiffExportFormat, sides: &Sides, out: &mut dyn Write) -> Result<usize> {
    let search = view.search.trim().to_lowercase();
    let rows = diff.entries.iter().filter(|e| diff.matches(e, &view.kinds, &search));
    match format {
        DiffExportFormat::Csv => {
            let mut w = csv::Writer::from_writer(out);
            w.write_record(["Change", "Path", "Old type", "Old value", "New type", "New value"])?;
            let mut n = 0;
            for e in rows {
                let (ok, ov) = cells(e.old.as_ref().map(ValueDto::from));
                let (nk, nv) = cells(e.new.as_ref().map(ValueDto::from));
                w.write_record([e.kind.as_str(), &e.display, &ok, &ov, &nk, &nv])?;
                n += 1;
            }
            w.flush()?;
            Ok(n)
        }
        DiffExportFormat::Html => {
            let c = &diff.counts;
            write!(
                out,
                "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\"><title>Configuration diff</title>\n<style>{STYLE}</style></head><body>\n\
                 <h1>Configuration diff</h1>\n<p><span class=\"removed\">− {}</span><br><span class=\"added\">+ {}</span></p>\n\
                 <p>{} added · {} removed · {} changed · {} unchanged · {} ignored</p>\n\
                 <table><thead><tr><th scope=\"col\">Change</th><th scope=\"col\">Path</th><th scope=\"col\">Old</th><th scope=\"col\">New</th></tr></thead><tbody>\n",
                escape(sides.left),
                escape(sides.right),
                c.added,
                c.removed,
                c.changed,
                c.unchanged,
                c.ignored
            )?;
            let mut n = 0;
            for e in rows {
                let cell = |v: &Option<super::ConfigValue>| v.as_ref().map(|v| escape(&ValueDto::from(v).text)).unwrap_or_default();
                writeln!(
                    out,
                    "<tr class=\"{k}\"><td>{k}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                    escape(&e.display),
                    cell(&e.old),
                    cell(&e.new),
                    k = e.kind.as_str()
                )?;
                n += 1;
            }
            writeln!(out, "</tbody></table></body></html>")?;
            Ok(n)
        }
    }
}

fn cells(v: Option<ValueDto>) -> (String, String) {
    match v {
        Some(v) => (format!("{:?}", v.kind).to_lowercase(), v.text),
        None => (String::new(), String::new()),
    }
}

const STYLE: &str = "body{font-family:system-ui,sans-serif;margin:24px;color:#1b1b1b;background:#fff}\
table{border-collapse:collapse;width:100%;font-family:ui-monospace,monospace;font-size:12px}\
th,td{border:1px solid #ccc;padding:3px 6px;text-align:left;vertical-align:top;white-space:pre-wrap;word-break:break-all}\
.added{background:#e6f4ea}.removed{background:#fde8e8}.changed{background:#fff4d6}";

/// Escapes text for HTML element content and attribute values.
pub fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

/// Parses `csv`/`html` (used at the IPC edge).
pub fn parse_format(s: &str) -> Result<DiffExportFormat> {
    match s.to_ascii_lowercase().as_str() {
        "csv" => Ok(DiffExportFormat::Csv),
        "html" => Ok(DiffExportFormat::Html),
        other => Err(anyhow!("unknown diff export format \"{other}\" (use csv or html)")),
    }
}

/// The kinds that are real changes (added, removed, changed).
pub fn changes_only() -> Vec<ChangeKind> {
    vec![ChangeKind::Added, ChangeKind::Removed, ChangeKind::Changed]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{diff, CompareOptions, ConfigParser, JsonParser, Limits};

    fn sample() -> Diff {
        let l = JsonParser.parse(r#"{"a": "<x>", "b": 1, "c": [1]}"#, &Limits::default()).unwrap();
        let r = JsonParser
            .parse(r#"{"a": "<y>", "b": 1, "d": "q,\"r\""}"#, &Limits::default())
            .unwrap();
        diff(&l, &r, &CompareOptions::default()).unwrap()
    }

    #[test]
    fn writes_csv() {
        let mut out = Vec::new();
        let view = DiffView {
            kinds: changes_only(),
            ..DiffView::default()
        };
        let n = write_diff(&sample(), &view, DiffExportFormat::Csv, &Sides { left: "l", right: "r" }, &mut out).unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(n, 4);
        assert_eq!(
            text,
            "Change,Path,Old type,Old value,New type,New value\nchanged,a,string,<x>,string,<y>\nremoved,c,section,,,\nremoved,c[0],integer,1,,\nadded,d,,,string,\"q,\"\"r\"\"\"\n"
        );
    }

    #[test]
    fn writes_escaped_html() {
        let mut out = Vec::new();
        let view = DiffView {
            search: "A".into(),
            ..DiffView::default()
        };
        let n = write_diff(
            &sample(),
            &view,
            DiffExportFormat::Html,
            &Sides { left: "<l>", right: "r&" },
            &mut out,
        )
        .unwrap();
        let text = String::from_utf8(out).unwrap();
        assert_eq!(n, 1);
        assert!(text.contains("<td>&lt;x&gt;</td><td>&lt;y&gt;</td>"));
        assert!(text.contains("− &lt;l&gt;") && text.contains("+ r&amp;"));
        assert!(!text.contains("<x>"));
        assert_eq!(escape("\"'"), "&quot;&#39;");
    }

    /// Fails after `left` bytes.
    struct Full {
        left: usize,
    }
    impl std::io::Write for Full {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            if self.left < buf.len() {
                return Err(std::io::Error::other("disk full"));
            }
            self.left -= buf.len();
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_errors_propagate() {
        let sides = Sides { left: "l", right: "r" };
        let d = sample();
        for left in [0, 900] {
            let r = write_diff(&d, &DiffView::default(), DiffExportFormat::Html, &sides, &mut Full { left });
            assert!(r.unwrap_err().to_string().contains("disk full"));
        }
        assert!(write_diff(&d, &DiffView::default(), DiffExportFormat::Csv, &sides, &mut Full { left: 0 }).is_err());
        assert!(std::io::Write::flush(&mut Full { left: 0 }).is_ok());
    }

    #[test]
    fn formats_parse() {
        assert_eq!(parse_format("CSV").unwrap(), DiffExportFormat::Csv);
        assert_eq!(parse_format("html").unwrap().extension(), "html");
        assert_eq!(DiffExportFormat::Csv.extension(), "csv");
        assert!(parse_format("pdf").is_err());
    }
}
