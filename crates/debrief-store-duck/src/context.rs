//! Source context (engine review finding 12): the lines around an event, read from its file at the
//! event's byte offset when the file is still the one imported (the text there is the event's first
//! line), else from the stored events of the source, with a note saying why.

use crate::{engine, Inner};
use debrief_core::api::{ContextRequest, FileKind, SourceContext, SourceLine};
use debrief_core::ports::{StoreError, StoreResult};
use duckdb::params;
use std::io::{Read, Seek, SeekFrom};

/// Most lines on either side of the event.
const MAX_RADIUS: usize = 500;
/// Most bytes read on either side of the event's offset.
const WINDOW: u64 = 4 << 20;
/// Characters kept of a line.
const MAX_LINE_CHARS: usize = 10_000;

fn cut(line: &str) -> String {
    line.trim_end_matches('\r').chars().take(MAX_LINE_CHARS).collect()
}

/// Source, line, byte offset, raw text, source name, path and file kind of an event.
type EventPlace = (u32, u32, u64, String, String, String, Option<String>);

/// Why the file's own lines are not shown.
enum Fallback {
    Missing,
    Changed,
    NotPlain,
}

/// The lines around byte `offset` of a plain UTF-8 file, when the line there is `first` (the event's
/// first line), numbered so that it is `line`.
fn from_file(path: &str, offset: u64, first: &str, line: u32, before: usize, after: usize) -> Result<Vec<SourceLine>, Fallback> {
    let mut f = std::fs::File::open(path).map_err(|_| Fallback::Missing)?;
    let mut bom = [0u8; 2];
    let n = f.read(&mut bom).map_err(|_| Fallback::Missing)?;
    if n == 2 && (bom == [0xFF, 0xFE] || bom == [0xFE, 0xFF]) {
        return Err(Fallback::NotPlain);
    }
    let start = offset.saturating_sub(WINDOW);
    f.seek(SeekFrom::Start(start)).map_err(|_| Fallback::Changed)?;
    let mut buf = Vec::new();
    f.take(offset - start + WINDOW)
        .read_to_end(&mut buf)
        .map_err(|_| Fallback::Changed)?;
    let (buf_len, window_len) = (buf.len(), offset - start + WINDOW);
    let at = (offset - start) as usize;
    if at > buf.len() {
        return Err(Fallback::Changed);
    }
    let (head, tail) = buf.split_at(at);
    let tail = String::from_utf8_lossy(tail);
    let mut after_lines = tail.split('\n');
    let here = after_lines.next().unwrap_or("");
    if here.trim_end_matches('\r') != first.trim_end_matches('\r') {
        return Err(Fallback::Changed);
    }
    let head = String::from_utf8_lossy(head);
    // The window may start inside a line: drop that partial line unless the window is the file start.
    let mut before_lines: Vec<&str> = head.split('\n').collect();
    before_lines.pop();
    if start > 0 && !before_lines.is_empty() {
        before_lines.remove(0);
    }
    let first_line = line as usize - before.min(before_lines.len()).min(line.saturating_sub(1) as usize);
    let mut out: Vec<SourceLine> = before_lines[before_lines.len() - (line as usize - first_line)..]
        .iter()
        .enumerate()
        .map(|(i, l)| SourceLine {
            line: (first_line + i) as u32,
            text: cut(l),
        })
        .collect();
    out.push(SourceLine { line, text: cut(here) });
    let mut rest: Vec<&str> = after_lines.collect();
    // A file ending with a line break leaves an empty last piece; a window that stopped early, a partial line.
    if rest.last() == Some(&"") {
        rest.pop();
    }
    if buf_len as u64 == window_len {
        rest.pop();
    }
    for (i, l) in rest.iter().take(after).enumerate() {
        out.push(SourceLine {
            line: line + 1 + i as u32,
            text: cut(l),
        });
    }
    Ok(out)
}

impl Inner {
    pub(crate) fn source_context(&self, req: &ContextRequest) -> StoreResult<SourceContext> {
        let conn = self.conn()?;
        let found: Option<EventPlace> = conn
            .query_row(
                "SELECT e.source, e.line, e.byte_offset, e.raw, s.name, s.path, s.file_kind FROM events e JOIN sources s ON s.id = e.source WHERE e.id = ?",
                params![u64::from(req.record_id)],
                |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?, r.get(4)?, r.get(5)?, r.get(6)?)),
            )
            .ok();
        let Some((source, line, offset, raw, name, path, kind)) = found else {
            return Err(StoreError::NotFound(format!("event #{}", req.record_id)));
        };
        let (before, after) = (req.before.min(MAX_RADIUS), req.after.min(MAX_RADIUS));
        let plain =
            kind.as_deref() == Some(format!("{:?}", FileKind::Text).as_str()) && !path.contains(debrief_core::archive::ENTRY_SEPARATOR);
        let first = raw.lines().next().unwrap_or("");
        let attempt = if plain && !raw.is_empty() {
            from_file(&path, offset, first, line.max(1), before, after)
        } else {
            Err(Fallback::NotPlain)
        };
        let why = match attempt {
            Ok(lines) => {
                return Ok(SourceContext {
                    source_name: name,
                    line,
                    lines,
                    note: None,
                })
            }
            Err(w) => w,
        };
        let note = match why {
            Fallback::Missing => format!("{path} is not available here; showing the events stored in the project"),
            Fallback::Changed => format!("{path} has changed since it was imported; showing the events stored in the project"),
            Fallback::NotPlain => {
                "the file is compressed, in an archive or not UTF-8; showing the events stored in the project".to_string()
            }
        };
        let first_line = line.saturating_sub(before as u32).max(1);
        let last_line = line.saturating_add(after as u32);
        // Events starting before the window may reach into it (multi-line events).
        let mut stmt = conn
            .prepare("SELECT line, raw FROM events WHERE source = ? AND line BETWEEN ? AND ? ORDER BY line")
            .map_err(engine)?;
        let rows = stmt
            .query_map(params![source, first_line.saturating_sub(MAX_RADIUS as u32), last_line], |r| {
                Ok((r.get::<_, u32>(0)?, r.get::<_, String>(1)?))
            })
            .map_err(engine)?;
        let mut lines: Vec<SourceLine> = Vec::new();
        for row in rows {
            let (start, text) = row.map_err(engine)?;
            for (i, l) in text.lines().enumerate() {
                let n = start + i as u32;
                if n >= first_line && n <= last_line && lines.last().is_none_or(|p| p.line < n) {
                    lines.push(SourceLine { line: n, text: cut(l) });
                }
            }
        }
        Ok(SourceContext {
            source_name: name,
            line,
            lines,
            note: Some(note),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(tag: &str, text: &[u8]) -> String {
        let p = std::env::temp_dir().join(format!("debrief-context-{tag}-{}.log", std::process::id()));
        std::fs::write(&p, text).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn reads_around_the_offset_and_checks_the_line() {
        let text = "one\ntwo\r\nthree\nfour\nfive\n";
        let path = file("plain", text.as_bytes());
        let offset = text.find("three").unwrap() as u64;
        let lines = from_file(&path, offset, "three", 3, 1, 1).ok().unwrap();
        let got: Vec<(u32, &str)> = lines.iter().map(|l| (l.line, l.text.as_str())).collect();
        assert_eq!(got, vec![(2, "two"), (3, "three"), (4, "four")]);
        let all = from_file(&path, offset, "three", 3, 10, 10).ok().unwrap();
        assert_eq!(all.len(), 5);
        assert!(matches!(from_file(&path, offset, "other", 3, 1, 1), Err(Fallback::Changed)));
        assert!(matches!(from_file(&path, 999, "three", 3, 1, 1), Err(Fallback::Changed)));
        assert!(matches!(
            from_file(&format!("{path}.none"), 0, "x", 1, 1, 1),
            Err(Fallback::Missing)
        ));
        let utf16 = file("utf16", &[0xFF, 0xFE, b'a', 0]);
        assert!(matches!(from_file(&utf16, 2, "a", 1, 1, 1), Err(Fallback::NotPlain)));
        let _ = std::fs::remove_file(path);
        let _ = std::fs::remove_file(utf16);
    }
}
