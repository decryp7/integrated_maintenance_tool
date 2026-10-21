//! The JSON importer, streaming: JSON lines (objects may span lines, as `journalctl -o json-pretty`
//! writes them) are read line by line with bounded look-ahead; a top-level array or a single object
//! (the plan's [`JsonShape`]) is read as one chunk, as the in-memory importer does.

use super::chunk::LineReader;
use super::plan::JsonShape;
use super::problems::ProblemKind;
use super::sink::{EventSink, ParsedEvent};
use crate::importers::{json_to_record, ImportCtx, JsonDepth, MAX_LINE_LEN};
use crate::model::Level;
use anyhow::{Context, Result};
use serde_json::Value;

/// Reads JSON events; returns the number of skipped lines (or array items). `size` is the data size in
/// bytes, which bounds the look-ahead for objects spanning lines (about twice the data in all).
pub fn import_json(ctx: &ImportCtx, shape: JsonShape, size: u64, lines: &mut LineReader, sink: &mut dyn EventSink) -> Result<usize> {
    match shape {
        JsonShape::Lines => json_lines(ctx, size, lines, sink),
        JsonShape::Array | JsonShape::Single => whole(ctx, shape, lines, sink),
    }
}

fn json_lines(ctx: &ImportCtx, size: u64, lines: &mut LineReader, sink: &mut dyn EventSink) -> Result<usize> {
    let cfg = &ctx.profile.format.json;
    let ts = &ctx.profile.timestamps;
    let default_level = Level::parse(&cfg.default_level).unwrap_or(Level::Information);
    let mut budget = usize::try_from(size)
        .unwrap_or(usize::MAX)
        .saturating_mul(2)
        .saturating_add(MAX_LINE_LEN);
    let mut skipped = 0;
    loop {
        let (number, byte_offset, raw, value) = {
            let Some(l) = lines.next_line()? else { break };
            if l.too_long {
                skipped += 1;
                sink.problem(ProblemKind::LineTooLong, l.number);
                continue;
            }
            let text = l.text.trim();
            if text.is_empty() {
                continue;
            }
            let parsed = serde_json::from_str::<Value>(text);
            (l.number, l.byte_offset, text.to_string(), parsed)
        };
        let (raw, value) = match value {
            Err(e) if e.is_eof() && raw.starts_with('{') => match multiline_object(&raw, lines, &mut budget)? {
                Some((v, text)) => (text, Some(v)),
                None => (raw, None),
            },
            other => (raw, other.ok()),
        };
        match value {
            Some(Value::Object(obj)) => sink.event(ParsedEvent {
                record: json_to_record(obj, cfg, ts, default_level, number),
                raw,
                byte_offset,
            })?,
            _ => {
                skipped += 1;
                sink.problem(ProblemKind::LineNotParsed, number);
            }
        }
    }
    Ok(skipped)
}

/// The object that starts with `first` and continues on the next lines, which are consumed when it is
/// complete and valid. When it is not (broken, cut off, or larger than [`MAX_LINE_LEN`]), the lines read
/// ahead are given back and `None` is returned. Every byte looked at is taken from `budget`.
fn multiline_object(first: &str, lines: &mut LineReader, budget: &mut usize) -> Result<Option<(Value, String)>> {
    let mut depth = JsonDepth::default();
    depth.feed(first);
    let mut buf = first.to_string();
    let mut ahead = Vec::new();
    let mut found = None;
    while let Some(line) = lines.next_owned()? {
        if line.too_long || buf.len() + line.text.len() >= MAX_LINE_LEN || line.text.len() >= *budget {
            ahead.push(line);
            break;
        }
        *budget -= line.text.len() + 1;
        buf.push('\n');
        buf.push_str(&line.text);
        let closed = depth.feed(&line.text);
        ahead.push(line);
        if closed {
            found = serde_json::from_str::<Value>(&buf).ok();
            break;
        }
    }
    match found {
        Some(v) => Ok(Some((v, buf))),
        None => {
            lines.unread(ahead);
            Ok(None)
        }
    }
}

/// A top-level array (one event per object item, numbered by position) or a single object.
fn whole(ctx: &ImportCtx, shape: JsonShape, lines: &mut LineReader, sink: &mut dyn EventSink) -> Result<usize> {
    let cfg = &ctx.profile.format.json;
    let ts = &ctx.profile.timestamps;
    let default_level = Level::parse(&cfg.default_level).unwrap_or(Level::Information);
    let mut content = String::new();
    while let Some(l) = lines.next_line()? {
        if !content.is_empty() {
            content.push('\n');
        }
        content.push_str(l.text);
    }
    let trimmed = content.trim_start();
    let mut skipped = 0;
    if shape == JsonShape::Array {
        let items: Vec<Value> = serde_json::from_str(trimmed).context("file starts with '[' but is not a JSON array")?;
        for (i, item) in items.into_iter().enumerate() {
            let line = i as u32 + 1;
            match item {
                Value::Object(obj) => {
                    let raw = serde_json::to_string(&obj).unwrap_or_default();
                    sink.event(ParsedEvent {
                        record: json_to_record(obj, cfg, ts, default_level, line),
                        raw,
                        byte_offset: 0,
                    })?;
                }
                _ => {
                    skipped += 1;
                    sink.problem(ProblemKind::LineNotParsed, line);
                }
            }
        }
        return Ok(skipped);
    }
    match serde_json::from_str::<Value>(trimmed) {
        Ok(Value::Object(obj)) => sink.event(ParsedEvent {
            record: json_to_record(obj, cfg, ts, default_level, 1),
            raw: trimmed.trim_end().to_string(),
            byte_offset: 0,
        })?,
        _ => {
            skipped += 1;
            sink.problem(ProblemKind::LineNotParsed, 1);
        }
    }
    Ok(skipped)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importers::{Importer, JsonImporter};
    use crate::ingest::chunk::{Encoding, ReaderChunks, TextChunks};
    use crate::ingest::problems::Problems;
    use crate::profile::{CompiledProfile, Profile};
    use std::path::Path;

    #[derive(Default)]
    struct Events {
        events: Vec<ParsedEvent>,
        problems: Problems,
    }

    impl EventSink for Events {
        fn event(&mut self, ev: ParsedEvent) -> Result<()> {
            self.events.push(ev);
            Ok(())
        }
        fn problem(&mut self, kind: ProblemKind, line: u32) {
            self.problems.add(kind, line);
        }
    }

    fn run(content: &str, shape: JsonShape, block: usize) -> (Events, usize) {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.json"),
            file_date: None,
        };
        let mut src = ReaderChunks::with_limits(content.as_bytes(), Encoding::Utf8, 0, block, MAX_LINE_LEN);
        let mut lines = LineReader::new(&mut src);
        let mut out = Events::default();
        let skipped = import_json(&ctx, shape, content.len() as u64, &mut lines, &mut out).unwrap();
        (out, skipped)
    }

    fn old(content: &str) -> crate::importers::Imported {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.json"),
            file_date: None,
        };
        JsonImporter.import(&ctx, content).unwrap()
    }

    const LINES: &str = concat!(
        r#"{"@t":"2026-10-14T10:41:22.318Z","@mt":"Order {OrderId} failed","@l":"Error","OrderId":88213}"#,
        "\n",
        "{\n  \"@t\": \"2026-10-14T10:41:23Z\",\n  \"@mt\": \"pretty {A}\",\n  \"A\": 1\n}\n",
        "not json\n",
        "{\"@t\": \"2026-10-14T10:41:24Z\", \"@mt\": \"broken\"\n",
        r#"{"@t":"2026-10-14T10:41:25.000Z","@mt":"last"}"#,
        "\n"
    );

    #[test]
    fn json_lines_match_the_in_memory_importer_at_any_chunk_size() {
        let before = old(LINES);
        for block in [3, 16, 1 << 20] {
            let (out, skipped) = run(LINES, JsonShape::Lines, block);
            assert_eq!(skipped, before.skipped, "block {block}");
            assert_eq!(out.events.len(), before.records.len());
            for (a, b) in out.events.iter().zip(&before.records) {
                assert_eq!((a.record.ts, &a.record.message, a.record.line), (b.ts, &b.message, b.line));
                assert_eq!(a.record.fields, b.fields);
            }
        }
        let (out, _) = run(LINES, JsonShape::Lines, 16);
        assert!(out.events[1].raw.starts_with("{\n  \"@t\"") && out.events[1].raw.ends_with('}'));
        assert_eq!(out.events[1].byte_offset, LINES.find("{\n").unwrap() as u64);
        assert_eq!(out.problems.count(ProblemKind::LineNotParsed), before.skipped as u64);
    }

    #[test]
    fn arrays_and_single_objects_are_read_whole() {
        let array = r#"[{"@t":"2026-10-14T10:00:00Z","@m":"a"}, 3, {"@t":"2026-10-14T10:00:01Z","@m":"b"}]"#;
        let (out, skipped) = run(array, JsonShape::Array, 8);
        assert_eq!((out.events.len(), skipped), (2, 1));
        assert_eq!(out.events[1].record.line, 3);
        assert!(out.events[0].raw.contains("\"@m\":\"a\""));
        let single = "{\n  \"@t\": \"2026-10-14T10:00:00Z\",\n  \"@m\": \"one\"\n}\n";
        let (out, _) = run(single, JsonShape::Single, 8);
        assert_eq!(out.events.len(), 1);
        assert_eq!(out.events[0].record.message, "one");
        let (out, skipped) = run("[1,2", JsonShape::Single, 8);
        assert_eq!((out.events.len(), skipped), (0, 1));
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.json"),
            file_date: None,
        };
        let mut src = TextChunks::new("[1, 2");
        let mut lines = LineReader::new(&mut src);
        assert!(import_json(&ctx, JsonShape::Array, 5, &mut lines, &mut Events::default()).is_err());
    }

    #[test]
    fn long_lines_are_reported() {
        let content = format!("{}\n{{\"@m\":\"x\"}}\n", "{".repeat(40));
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.json"),
            file_date: None,
        };
        let mut src = ReaderChunks::with_limits(content.as_bytes(), Encoding::Utf8, 0, 8, 20);
        let mut lines = LineReader::new(&mut src);
        let mut out = Events::default();
        let skipped = import_json(&ctx, JsonShape::Lines, content.len() as u64, &mut lines, &mut out).unwrap();
        assert_eq!((skipped, out.events.len()), (1, 1));
        assert_eq!(out.problems.count(ProblemKind::LineTooLong), 1);
    }
}
