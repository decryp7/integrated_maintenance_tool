//! Adapter for importers that do not stream yet (CSV, plugin text and binary importers): they get the
//! whole decoded file (or the bytes) as before, and their records are fed to an [`EventSink`] with
//! their raw text cut from the file by line numbers.

use super::sink::{EventSink, ParsedEvent};
use super::MAX_EVENT_BYTES;
use crate::importers::{BinaryImporter, ImportCtx, Importer};
use anyhow::Result;

/// Runs a text importer on the whole decoded file; returns its skipped count.
///
/// An event's raw text is the file's lines from its line number up to the next event's (when line
/// numbers ascend), capped at [`MAX_EVENT_BYTES`]; its byte offset is the offset of its first line in
/// the decoded text plus `data_start` (exact for UTF-8 files).
pub fn import_text_whole(
    importer: &dyn Importer,
    ctx: &ImportCtx,
    content: &str,
    data_start: u64,
    sink: &mut dyn EventSink,
) -> Result<usize> {
    let imported = importer.import(ctx, content)?;
    let starts = line_starts(content);
    let line_text = |n: u32| -> Option<(usize, usize)> {
        let i = (n as usize).checked_sub(1)?;
        let start = *starts.get(i)?;
        let end = starts.get(i + 1).map_or(content.len(), |e| *e);
        Some((start, end))
    };
    let lines: Vec<u32> = imported.records.iter().map(|r| r.line).collect();
    sink.problems(imported.problems);
    for (i, record) in imported.records.into_iter().enumerate() {
        let (raw, byte_offset) = match line_text(record.line) {
            Some((start, first_end)) => {
                // Up to the next event; the last one runs to the end of the file.
                let end = match lines.get(i + 1) {
                    None => content.len(),
                    Some(n) if *n > record.line => line_text(*n).map_or(content.len(), |(s, _)| s),
                    Some(_) => first_end,
                };
                let end = floor_char_boundary(content, end.min(start + MAX_EVENT_BYTES));
                (content[start..end].trim_end().to_string(), data_start + start as u64)
            }
            None => (record.message.clone(), data_start),
        };
        sink.event(ParsedEvent { record, raw, byte_offset })?;
    }
    Ok(imported.skipped)
}

/// Runs a binary importer on the bytes; events have no raw text.
pub fn import_binary(importer: &dyn BinaryImporter, ctx: &ImportCtx, bytes: &[u8], sink: &mut dyn EventSink) -> Result<usize> {
    let imported = importer.import(ctx, bytes)?;
    sink.problems(imported.problems);
    for record in imported.records {
        sink.event(ParsedEvent {
            record,
            raw: String::new(),
            byte_offset: 0,
        })?;
    }
    Ok(imported.skipped)
}

/// Offset of every line's first byte.
fn line_starts(text: &str) -> Vec<usize> {
    let mut starts = vec![0];
    starts.extend(text.match_indices('\n').map(|(i, _)| i + 1).filter(|i| *i < text.len()));
    starts
}

fn floor_char_boundary(s: &str, mut i: usize) -> usize {
    while i > 0 && !s.is_char_boundary(i) {
        i -= 1;
    }
    i
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importers::{CsvImporter, Imported};
    use crate::ingest::problems::ProblemKind;
    use crate::model::Record;
    use crate::profile::{CompiledProfile, Profile};
    use std::path::Path;

    #[derive(Default)]
    struct Events(Vec<ParsedEvent>, crate::ingest::Problems);

    impl EventSink for Events {
        fn event(&mut self, ev: ParsedEvent) -> Result<()> {
            self.0.push(ev);
            Ok(())
        }
        fn problem(&mut self, _kind: ProblemKind, _line: u32) {}
        fn problems(&mut self, problems: crate::ingest::Problems) {
            self.1.merge(problems);
        }
    }

    #[test]
    fn csv_rows_keep_their_lines_as_raw_text() {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.csv"),
            file_date: None,
        };
        let content =
            "Time,Level,Message\n2026-10-14 10:00:00,Info,first\n2026-10-14 10:00:01,Error,second\n2026-10-14 10:00:02,Info,third\n";
        let mut out = Events::default();
        import_text_whole(&CsvImporter, &ctx, content, 3, &mut out).unwrap();
        assert_eq!(out.0.len(), 3);
        assert_eq!(out.0[0].raw, "2026-10-14 10:00:00,Info,first");
        assert_eq!(out.0[0].byte_offset, 3 + 19);
        assert_eq!(out.0[2].raw, "2026-10-14 10:00:02,Info,third");
    }

    struct Fixed(Vec<u32>);

    impl Importer for Fixed {
        fn id(&self) -> &'static str {
            "fixed"
        }
        fn sniff(&self, _ctx: &ImportCtx, _head: &str) -> u8 {
            1
        }
        fn import(&self, _ctx: &ImportCtx, _content: &str) -> Result<Imported> {
            Ok(Imported {
                records: self
                    .0
                    .iter()
                    .map(|l| {
                        let mut r = Record::new(*l);
                        r.message = format!("m{l}");
                        r
                    })
                    .collect(),
                skipped: 2,
                problems: {
                    let mut p = crate::ingest::Problems::default();
                    p.add(ProblemKind::UnmatchedLine, 4);
                    p
                },
            })
        }
    }

    impl BinaryImporter for Fixed {
        fn id(&self) -> &'static str {
            "fixed-bin"
        }
        fn sniff(&self, _ctx: &ImportCtx, _head: &[u8]) -> u8 {
            1
        }
        fn import(&self, ctx: &ImportCtx, _bytes: &[u8]) -> Result<Imported> {
            Importer::import(self, ctx, "")
        }
    }

    #[test]
    fn out_of_range_or_descending_lines_fall_back() {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x"),
            file_date: None,
        };
        let mut out = Events::default();
        let skipped = import_text_whole(&Fixed(vec![2, 1, 9]), &ctx, "aé\nbb\n", 0, &mut out).unwrap();
        assert_eq!(skipped, 2);
        let raws: Vec<&str> = out.0.iter().map(|e| e.raw.as_str()).collect();
        assert_eq!(raws, ["bb", "aé\nbb", "m9"]);
        assert_eq!(out.1.count(ProblemKind::UnmatchedLine), 1, "the importer's problems reach the sink");
        let mut out = Events::default();
        assert_eq!(import_binary(&Fixed(vec![1]), &ctx, b"x", &mut out).unwrap(), 2);
        assert_eq!(out.1.count(ProblemKind::UnmatchedLine), 1);
        assert_eq!(out.0[0].raw, "");
        assert_eq!(floor_char_boundary("é", 1), 0);
        assert_eq!(line_starts("a\nb\n"), [0, 2]);
    }
}
