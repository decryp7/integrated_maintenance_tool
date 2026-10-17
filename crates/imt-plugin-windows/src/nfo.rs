//! msinfo32 `.nfo` export (XML).

use anyhow::Result;
use imt_core::importers::{ImportCtx, Imported, Importer};
use imt_core::model::{auto_value, Record};
use quick_xml::events::Event;
use quick_xml::Reader;
use serde_json::Value;

/// Hard cap on element nesting, so hostile input cannot grow the stack/path.
const MAX_DEPTH: usize = 64;

pub struct NfoImporter;

impl Importer for NfoImporter {
    fn id(&self) -> &'static str {
        "msinfo"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
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

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let mut reader = Reader::from_str(content);
        reader.config_mut().check_end_names = false;
        let mut out = Imported::default();
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
                    let name = String::from_utf8_lossy(e.name().as_ref()).into_owned();
                    if stack.len() >= MAX_DEPTH {
                        out.skipped += 1;
                        break;
                    }
                    if name == "Category" {
                        let label = e
                            .attributes()
                            .flatten()
                            .find(|a| a.key.as_ref() == b"name")
                            .map(|a| String::from_utf8_lossy(&a.value).into_owned())
                            .unwrap_or_default();
                        categories.push(label);
                    } else if name == "Data" {
                        row = Some(Vec::new());
                    }
                    text.clear();
                    stack.push(name);
                }
                Ok(Event::Text(t)) => text.push_str(&t.decode().map(|c| c.into_owned()).unwrap_or_default()),
                Ok(Event::GeneralRef(r)) => {
                    let name = String::from_utf8_lossy(r.as_ref()).into_owned();
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
                Ok(Event::CData(c)) => text.push_str(&String::from_utf8_lossy(&c)),
                Ok(Event::End(_)) => {
                    let Some(name) = stack.pop() else { continue };
                    match name.as_str() {
                        "Category" => {
                            categories.pop();
                        }
                        "Data" => {
                            if let Some(cols) = row.take() {
                                out.records.push(make_record(line, &categories, cols));
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
        Ok(out)
    }
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

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::profile::{CompiledProfile, Profile};
    use std::path::Path;

    fn run(content: &str) -> Imported {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.nfo"),
            file_date: None,
        };
        NfoImporter.import(&ctx, content).unwrap()
    }

    fn sniff(s: &str) -> u8 {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x"),
            file_date: None,
        };
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
}
