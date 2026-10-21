//! msinfo32 `.nfo` export (XML).

use anyhow::Result;
use imt_plugin_api::config::{ConfigError, ConfigParser, ConfigTree, ConfigValue, Limits, PathSeg, TreeBuilder};
use imt_plugin_api::importer::{ImportCtx, Imported, TextImporter};
use imt_plugin_api::model::{auto_value, Record};
use quick_xml::events::Event;
use quick_xml::{Reader, XmlVersion};
use serde_json::Value;
use std::collections::HashMap;

/// Hard cap on element nesting, so hostile input cannot grow the stack/path.
const MAX_DEPTH: usize = 64;

pub struct NfoImporter;

impl TextImporter for NfoImporter {
    fn id(&self) -> &'static str {
        "msinfo"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        sniff_text(head)
    }

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let parsed = parse_rows(content);
        Ok(Imported {
            records: parsed
                .rows
                .into_iter()
                .map(|row| make_record(row.line, &row.categories, row.cols))
                .collect(),
            skipped: parsed.skipped,
            ..Default::default()
        })
    }
}

/// 100 when the text starts like an msinfo report (`<MsInfo`, maybe after an XML declaration).
fn sniff_text(head: &str) -> u8 {
    let head = head.trim_start_matches('\u{feff}').trim_start();
    let body = if head.starts_with("<?xml") {
        head.find("?>").map_or("", |i| head[i + 2..].trim_start())
    } else {
        head
    };
    if body.starts_with("<MsInfo") {
        100
    } else {
        0
    }
}

/// One `Data` row of an msinfo report.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NfoRow {
    pub line: u32,
    /// Category path, outermost first.
    pub categories: Vec<String>,
    /// Columns (element name, text) in file order; `_x0020_` not yet decoded.
    pub cols: Vec<(String, String)>,
}

/// Rows of a report and the number of problems that stopped reading.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NfoRows {
    pub rows: Vec<NfoRow>,
    pub skipped: usize,
}

/// Reads the `Data` rows of an msinfo32 export (the one parser behind the
/// importer and the configuration-diff format). Never panics; malformed XML
/// and nesting beyond the limit stop reading and count as skipped.
pub fn parse_rows(content: &str) -> NfoRows {
    let mut reader = Reader::from_str(content);
    reader.config_mut().check_end_names = false;
    let mut out = NfoRows::default();
    let mut categories: Vec<String> = Vec::new();
    // Element names currently open, to know where text belongs.
    let mut stack: Vec<String> = Vec::new();
    let mut row: Option<Vec<(String, String)>> = None;
    let mut text = String::new();
    loop {
        let line = line_of(content, reader.buffer_position() as usize);
        match reader.read_event() {
            Err(_) => {
                out.skipped += 1;
                break;
            }
            Ok(Event::Eof) => break,
            Ok(Event::Start(e)) => {
                let name = e.name().as_ref().to_string();
                if stack.len() >= MAX_DEPTH {
                    out.skipped += 1;
                    break;
                }
                if name == "Category" {
                    let label = e
                        .attributes()
                        .flatten()
                        .find(|a| a.key.as_ref() == "name")
                        .map(|a| {
                            a.normalized_value(XmlVersion::Implicit1_0)
                                .map(|v| v.into_owned())
                                .unwrap_or_default()
                        })
                        .unwrap_or_default();
                    categories.push(label);
                } else if name == "Data" {
                    row = Some(Vec::new());
                }
                text.clear();
                stack.push(name);
            }
            Ok(Event::Text(t)) => text.push_str(&t.xml_content(XmlVersion::Implicit1_0)),
            Ok(Event::GeneralRef(r)) => {
                let name = r.as_ref().to_string();
                match r.resolve_char_ref() {
                    Ok(Some(c)) => text.push(c),
                    _ => text.push_str(match name.as_str() {
                        "amp" => "&",
                        "lt" => "<",
                        "gt" => ">",
                        "quot" => "\"",
                        "apos" => "'",
                        _ => "",
                    }),
                }
            }
            Ok(Event::CData(c)) => text.push_str(&c),
            Ok(Event::End(_)) => {
                let Some(name) = stack.pop() else { continue };
                match name.as_str() {
                    "Category" => {
                        categories.pop();
                    }
                    "Data" => {
                        if let Some(cols) = row.take() {
                            out.rows.push(NfoRow {
                                line,
                                categories: categories.clone(),
                                cols,
                            });
                        }
                    }
                    _ => {
                        if let (Some(cols), true) = (row.as_mut(), stack.last().map(String::as_str) == Some("Data")) {
                            cols.push((name, text.trim().to_string()));
                        }
                    }
                }
                text.clear();
            }
            Ok(_) => {}
        }
    }
    out
}

fn line_of(content: &str, pos: usize) -> u32 {
    let end = pos.min(content.len());
    content.as_bytes()[..end].iter().filter(|b| **b == b'\n').count() as u32 + 1
}

fn make_record(line: u32, categories: &[String], cols: Vec<(String, String)>) -> Record {
    let mut r = Record::new(line);
    let category = categories.join("/");
    r.fields.insert("Category".into(), Value::from(category.clone()));
    let (mut item, mut value) = (String::new(), String::new());
    for (k, v) in cols {
        match k.as_str() {
            "Item" => item = v.clone(),
            "Value" => value = v.clone(),
            _ => {}
        }
        if k != "Category" {
            r.fields.insert(k.replace("_x0020_", " "), auto_value(&v));
        }
    }
    r.message = format!("{category} \u{203a} {item} = {value}");
    r
}

/// msinfo reports as configuration trees for the diff view:
/// `Category/.../Item` holds the row's `Value`, other columns sit below it
/// (`.../Adapter/Adapter RAM`). A row without `Item` is named by its first
/// column. Repeated names in one category get `[1]`, `[2]`... Values are
/// typed like the importer's fields (integers and decimals become numbers).
pub struct NfoConfigParser;

impl ConfigParser for NfoConfigParser {
    fn id(&self) -> &str {
        "msinfo"
    }

    fn name(&self) -> &str {
        "System information (msinfo32)"
    }

    fn extensions(&self) -> Vec<String> {
        vec!["nfo".into()]
    }

    fn sniff(&self, file_name: &str, head: &str) -> u8 {
        let score = sniff_text(head);
        if score > 0 {
            score
        } else if imt_plugin_api::config::extension_of(file_name) == "nfo" {
            80
        } else {
            0
        }
    }

    fn parse(&self, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError> {
        let parsed = parse_rows(content);
        if parsed.rows.is_empty() && parsed.skipped > 0 {
            return Err(ConfigError::syntax("msinfo", "the report is not well-formed XML"));
        }
        let mut b = TreeBuilder::new(self.id(), limits);
        let mut seen: HashMap<Vec<PathSeg>, usize> = HashMap::new();
        for row in parsed.rows {
            if b.is_full() {
                b.mark_truncated();
                break;
            }
            let category: Vec<PathSeg> = row.categories.iter().map(|c| PathSeg::key(c.as_str())).collect();
            let cols: Vec<(String, String)> = row.cols.into_iter().map(|(k, v)| (k.replace("_x0020_", " "), v)).collect();
            let item = cols.iter().find(|(k, _)| k == "Item").map(|(_, v)| v.clone());
            let label = item.clone().or_else(|| cols.first().map(|(_, v)| v.clone())).unwrap_or_default();
            let mut path = category.clone();
            let count = seen
                .entry([category.clone(), vec![PathSeg::key(label.as_str())]].concat())
                .or_insert(0);
            path.push(PathSeg::Key(if *count == 0 { label.clone() } else { format!("{label}[{count}]") }));
            *count += 1;
            let own = match (&item, cols.iter().find(|(k, _)| k == "Value")) {
                (Some(_), Some((_, v))) => typed(v),
                _ => ConfigValue::Section,
            };
            b.push(path.clone(), own)?;
            for (k, v) in &cols {
                let is_own = item.is_some() && (k == "Item" || k == "Value");
                if is_own || k == "Category" {
                    continue;
                }
                let mut p = path.clone();
                p.push(PathSeg::key(k.as_str()));
                b.push(p, typed(v))?;
            }
        }
        Ok(b.finish())
    }
}

fn typed(v: &str) -> ConfigValue {
    match auto_value(v) {
        Value::Number(n) => n
            .as_i64()
            .map_or_else(|| ConfigValue::Float(n.as_f64().unwrap_or(f64::NAN)), ConfigValue::Integer),
        Value::String(s) => ConfigValue::String(s),
        other => ConfigValue::String(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_plugin_api::time::TimestampParser;
    use std::path::Path;

    fn run(content: &str) -> Imported {
        let ts = TimestampParser::default();
        let ctx = ImportCtx::new(Path::new("x.nfo"), &ts);
        NfoImporter.import(&ctx, content).unwrap()
    }

    fn sniff(s: &str) -> u8 {
        let ts = TimestampParser::default();
        let ctx = ImportCtx::new(Path::new("x"), &ts);
        NfoImporter.sniff(&ctx, s)
    }

    #[test]
    fn nested_categories_and_columns() {
        let out = run(include_str!("../tests/fixtures/msinfo.nfo"));
        assert_eq!(NfoImporter.id(), "msinfo");
        let first = &out.records[0];
        assert_eq!(first.fields["Category"], "System Summary");
        assert_eq!(first.fields["Item"], "OS Name");
        assert_eq!(
            first.message,
            "System Summary \u{203a} OS Name = Microsoft Windows Server 2022 Datacenter"
        );
        let display = out
            .records
            .iter()
            .find(|r| r.fields["Category"] == "System Summary/Components/Display")
            .unwrap();
        assert_eq!(display.fields["Name"], "NVIDIA RTX & Co <A>");
        assert_eq!(display.fields["Adapter RAM"], 8192);
        assert!(display.ts.is_none());
        let mem = out.records.iter().find(|r| r.fields["Item"] == "Total Physical Memory").unwrap();
        assert_eq!(mem.fields["Value"], "16.0 GB \u{e9}\u{263a}");
        assert_eq!(out.skipped, 0);
    }

    #[test]
    fn sniffing() {
        assert_eq!(sniff("\u{feff}<?xml version=\"1.0\"?>\n<MsInfo><x/>"), 100);
        assert_eq!(sniff("<MsInfo>"), 100);
        assert_eq!(sniff("<?xml version=\"1.0\"?><Other/>"), 0);
        assert_eq!(sniff("<?xml broken"), 0);
        assert_eq!(sniff("2026-10-14 log"), 0);
    }

    #[test]
    fn malformed_never_panics() {
        let out = run("<MsInfo><Category name=\"A\"><Data><Item>x</Item></Category></Mismatch><");
        assert!(out.skipped >= 1);
        let deep = "<a>".repeat(200);
        assert_eq!(run(&deep).skipped, 1);
        // stray end tags and unnamed categories
        let out = run("<MsInfo></x><Category><Data><Item>i</Item><Value>v</Value></Data></Category><!-- c --></MsInfo>");
        assert_eq!(out.records[0].fields["Category"], "");
        let seed = "<MsInfo><Category name=\"A\"><Data><Item>a</Item><Value>&#65;&amp;&bogus;</Value></Data></Category></MsInfo>";
        assert_eq!(run(seed).records[0].fields["Value"], "A&");
        let mut x: u32 = 12345;
        for _ in 0..300 {
            let mut s = String::new();
            for _ in 0..80 {
                x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                let pieces = [
                    "<",
                    ">",
                    "/",
                    "Data",
                    "Item",
                    "Category",
                    "name=\"",
                    "\"",
                    "&",
                    "x",
                    " ",
                    "<![CDATA[",
                    "]]>",
                ];
                s.push_str(pieces[(x >> 16) as usize % pieces.len()]);
            }
            let _ = run(&s);
            let _ = sniff(&s);
        }
    }

    #[test]
    fn config_tree_of_the_fixture() {
        let content = include_str!("../tests/fixtures/msinfo.nfo");
        let t = NfoConfigParser.parse(content, &Limits::default()).unwrap();
        let shown: Vec<String> = t
            .nodes
            .iter()
            .map(|n| {
                format!(
                    "{}={}",
                    n.path.iter().map(PathSeg::label).collect::<Vec<_>>().join("/"),
                    n.value.text()
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                "System Summary/OS Name=Microsoft Windows Server 2022 Datacenter",
                "System Summary/Total Physical Memory=16.0 GB \u{e9}\u{263a}",
                "System Summary/Components/Display/Adapter=ok",
                "System Summary/Components/Display/Adapter/Name=NVIDIA RTX & Co <A>",
                "System Summary/Components/Display/Adapter/Adapter RAM=8192",
            ]
        );
        assert_eq!(t.nodes[4].value, ConfigValue::Integer(8192));
        let other = NfoConfigParser
            .parse(&content.replace("8192", "4096.5"), &Limits::default())
            .unwrap();
        assert_eq!(other.nodes[4].value, ConfigValue::Float(4096.5));
    }

    #[test]
    fn config_rows_without_item_and_duplicates() {
        let t = NfoConfigParser
            .parse(
                "<MsInfo><Category name=\"Drivers\"><Data><Name>a</Name><State>Running</State></Data><Data><Name>a</Name><State>Stopped</State></Data>\
                 <Data><Item>X</Item></Data><Data></Data></Category></MsInfo>",
                &Limits::default(),
            )
            .unwrap();
        let shown: Vec<String> = t
            .nodes
            .iter()
            .map(|n| {
                format!(
                    "{}={}",
                    n.path.iter().map(PathSeg::label).collect::<Vec<_>>().join("/"),
                    n.value.text()
                )
            })
            .collect();
        assert_eq!(
            shown,
            [
                "Drivers/a=",
                "Drivers/a/Name=a",
                "Drivers/a/State=Running",
                "Drivers/a[1]=",
                "Drivers/a[1]/Name=a",
                "Drivers/a[1]/State=Stopped",
                "Drivers/X=",
                "Drivers/=",
            ]
        );
        assert_eq!(typed("1.5"), ConfigValue::Float(1.5));
        let limits = Limits {
            max_nodes: 1,
            ..Limits::default()
        };
        assert!(
            NfoConfigParser
                .parse(include_str!("../tests/fixtures/msinfo.nfo"), &limits)
                .unwrap()
                .truncated
        );
    }

    #[test]
    fn config_rejects_malformed_reports() {
        assert!(matches!(
            NfoConfigParser.parse("<MsInfo><", &Limits::default()),
            Err(ConfigError::Syntax { .. })
        ));
        assert_eq!(NfoConfigParser.sniff("x", "<MsInfo>"), 100);
        assert_eq!(NfoConfigParser.sniff("x.NFO", "garbage"), 80);
        assert_eq!(NfoConfigParser.sniff("x.txt", "garbage"), 0);
        assert_eq!(
            (NfoConfigParser.id(), NfoConfigParser.name(), NfoConfigParser.extensions()),
            ("msinfo", "System information (msinfo32)", vec!["nfo".to_string()])
        );
    }
}
