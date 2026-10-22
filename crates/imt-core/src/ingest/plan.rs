//! The file plan: one sequential sniff of a file's start that fixes everything an importer would
//! otherwise decide from the whole file (encoding, importer, text format and day order, JSON shape), so
//! the file can be read in chunks, and later in parallel (R2b), with the same result.

use super::chunk::{Encoding, CHUNK_BYTES};
use super::encoding::{detect, EncodingInfo, EncodingRequest};
use crate::importers::lines::{self, LineFormat};
use crate::importers::{first_lines, looks_binary, ImportCtx, Importers};
use std::io::Read;

/// Bytes of a file's start the plan reads (1 MiB): enough for the importer choice (64 KiB of text), the
/// text format (200 lines) and the day order (2,000 lines) of usual logs.
pub const HEAD_BYTES: usize = 1 << 20;

/// Target size of a parallel chunk (64 MiB, ADR 0014 §2).
pub const CHUNK_TARGET: u64 = 64 << 20;

/// Characters of the head the importers are chosen from (as the in-memory import does).
const SELECT_CHARS: usize = 64 * 1024;

/// Lines of the head that decide an ambiguous day/month order.
const DAY_ORDER_LINES: usize = 2000;

/// The shape of a JSON file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum JsonShape {
    /// One object per line (objects may span lines).
    Lines,
    /// A top-level array: read as one chunk.
    Array,
    /// One (pretty-printed) object: read as one chunk.
    Single,
}

/// How a file is read.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PlanKind {
    /// A binary importer reads the bytes.
    Binary,
    /// The streaming text importer, trying this format first on every line (the one most of the head's
    /// lines have; other lines are read with the first format that matches them, ADR 0020).
    Text {
        format: LineFormat,
        day_first: Option<bool>,
        /// The timestamp has no date: later events depend on earlier ones (midnight rollover).
        time_only: bool,
    },
    /// The streaming JSON importer.
    Json(JsonShape),
    /// Another text importer, given the whole decoded file (adapter, until it streams).
    Legacy,
}

/// Where a parallel chunk starts: on a line boundary, with that line's number.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChunkStart {
    pub byte_offset: u64,
    /// 1-based number of the chunk's first line.
    pub line: u32,
}

/// Everything decided about a file before its events are read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FilePlan {
    pub encoding: Encoding,
    /// How the encoding was decided (shown to the user, who can choose another).
    pub encoding_info: Option<EncodingInfo>,
    /// Offset of the first byte after the byte-order mark.
    pub data_start: u64,
    /// Id of the importer that reads the file.
    pub importer: String,
    pub kind: PlanKind,
    /// Whether an event can span several lines.
    pub multiline: bool,
    /// Whether the file can be parsed in independent chunks: no state crosses lines except the event
    /// being built (not time-only text, JSON lines).
    pub splittable: bool,
    /// Line-aligned chunk starts every [`CHUNK_TARGET`] bytes (only for splittable files larger than
    /// that; see [`scan_chunk_starts`]). R2b moves each start to the next line that begins an event.
    pub chunks: Vec<ChunkStart>,
}

/// The encoding of `bytes` (a file's start) and the length of its byte-order mark, decided from the content.
pub fn detect_encoding(bytes: &[u8]) -> (Encoding, u64) {
    let d = detect(bytes, false, EncodingRequest::default());
    (d.encoding, d.bom_len)
}

/// Decodes the head (from after the BOM). Unless it is the whole file, it is cut after its last newline
/// so no partial line or character is decoded.
fn decode_head(head: &[u8], encoding: Encoding, start: usize, whole: bool) -> String {
    let mut body = &head[start.min(head.len())..];
    if !whole {
        let unit = encoding.unit();
        let mut end = 0;
        let mut pos = 0;
        while let Some(nl) = encoding.find_newline(&body[pos..]) {
            pos += nl + unit;
            end = pos;
        }
        body = &body[..end];
    }
    let mut text = String::with_capacity(body.len());
    let mut pos = 0;
    // Line by line, so the text equals what the chunk source decodes.
    let unit = encoding.unit();
    while pos < body.len() {
        let end = encoding.find_newline(&body[pos..]).map_or(body.len(), |nl| pos + nl);
        encoding.decode_into(&body[pos..end], &mut text);
        if end < body.len() {
            text.push('\n');
        }
        pos = end + unit;
    }
    text
}

/// The decoded start of a file the parsers are scored on (at most [`crate::parser::set::SCORE_CHARS`]
/// characters, from the first 256 KiB), or `None` when it does not look like text.
pub fn score_text(head: &[u8], whole: bool) -> Option<String> {
    score_text_with(head, whole, EncodingRequest::default())
}

/// [`score_text`] with the encoding the profile or the user asked for.
pub fn score_text_with(head: &[u8], whole: bool, request: EncodingRequest) -> Option<String> {
    const SCORE_BYTES: usize = 256 * 1024;
    let d = detect(head, whole, request);
    let (encoding, data_start) = (d.encoding, d.bom_len);
    let cut = head.len() > SCORE_BYTES;
    let text = decode_head(&head[..head.len().min(SCORE_BYTES)], encoding, data_start as usize, whole && !cut);
    (!looks_binary(&text)).then(|| text.chars().take(crate::parser::set::SCORE_CHARS).collect())
}

/// Plans reading a file from its first bytes (`head`, at most [`HEAD_BYTES`]; `whole` when that is the
/// entire file). The error is the reason shown for an unrecognised file.
pub fn plan(importers: &Importers, ctx: &ImportCtx, head: &[u8], whole: bool) -> Result<FilePlan, String> {
    plan_with(importers, ctx, head, whole, EncodingRequest::default())
}

/// [`plan`] with the encoding the profile or the user asked for (`request`).
pub fn plan_with(importers: &Importers, ctx: &ImportCtx, head: &[u8], whole: bool, request: EncodingRequest) -> Result<FilePlan, String> {
    if head.is_empty() {
        return Err("empty file".into());
    }
    if let Some(binary) = importers.select_binary(ctx, &head[..head.len().min(SELECT_CHARS)]) {
        return Ok(FilePlan {
            encoding: Encoding::Utf8,
            encoding_info: None,
            data_start: 0,
            importer: binary.id().to_string(),
            kind: PlanKind::Binary,
            multiline: false,
            splittable: false,
            chunks: Vec::new(),
        });
    }
    let detected = detect(head, whole, request);
    let (encoding, data_start) = (detected.encoding, detected.bom_len);
    let text = decode_head(head, encoding, data_start as usize, whole);
    if looks_binary(&text) {
        return Err("binary file".into());
    }
    let select: String = text.chars().take(SELECT_CHARS).collect();
    let importer = importers.select(ctx, &select).ok_or("no importer recognises this format")?;
    let kind = match importer.id() {
        "text" => {
            let format = lines::best_format(ctx.profile, &text).ok_or("no output template or regex in the profile matches this file")?;
            PlanKind::Text {
                format,
                day_first: crate::importers::text::day_order(ctx, format, &text, DAY_ORDER_LINES),
                time_only: matches!(format, LineFormat::Text(i) if ctx.profile.text_formats[i].time_only),
            }
        }
        "json" => PlanKind::Json(json_shape(&text, whole)),
        _ => PlanKind::Legacy,
    };
    let splittable = match kind {
        PlanKind::Text { time_only, .. } => !time_only,
        PlanKind::Json(shape) => shape == JsonShape::Lines,
        PlanKind::Binary | PlanKind::Legacy => false,
    };
    Ok(FilePlan {
        encoding,
        encoding_info: Some(detected.info()),
        data_start,
        importer: importer.id().to_string(),
        kind,
        multiline: importer.multiline(),
        splittable,
        chunks: Vec::new(),
    })
}

/// The JSON shape, decided as the in-memory JSON importer decides it from the whole file.
fn json_shape(text: &str, whole: bool) -> JsonShape {
    let trimmed = text.trim_start();
    if trimmed.starts_with('[') {
        return JsonShape::Array;
    }
    let first_parses = first_lines(text, 1)
        .next()
        .is_some_and(|l| serde_json::from_str::<serde_json::Value>(l.trim()).is_ok());
    if !first_parses && whole && matches!(serde_json::from_str::<serde_json::Value>(trimmed), Ok(serde_json::Value::Object(_))) {
        return JsonShape::Single;
    }
    JsonShape::Lines
}

/// Line-aligned chunk starts of the data read from `reader` (positioned at `data_start`), one every
/// `target` bytes, the first at `data_start`. One sequential newline scan, bounded memory.
pub fn scan_chunk_starts<R: Read>(mut reader: R, encoding: Encoding, data_start: u64, target: u64) -> std::io::Result<Vec<ChunkStart>> {
    let unit = encoding.unit();
    let mut starts = vec![ChunkStart {
        byte_offset: data_start,
        line: 1,
    }];
    let mut buf = vec![0u8; CHUNK_BYTES.min(target.max(2) as usize * 4).max(2)];
    let mut carry = 0usize;
    let mut base = data_start;
    let mut line: u32 = 1;
    loop {
        let n = reader.read(&mut buf[carry..])?;
        let len = carry + n;
        let aligned = len - len % unit;
        let mut pos = 0;
        while let Some(nl) = encoding.find_newline(&buf[pos..aligned]) {
            pos += nl + unit;
            line = line.saturating_add(1);
            let offset = base + pos as u64;
            if offset - starts[starts.len() - 1].byte_offset >= target {
                starts.push(ChunkStart { byte_offset: offset, line });
            }
        }
        if n == 0 {
            break;
        }
        base += aligned as u64;
        buf.copy_within(aligned..len, 0);
        carry = len - aligned;
    }
    // A start at the very end of the data begins no chunk.
    if starts.len() > 1 && starts.last().is_some_and(|s| s.byte_offset >= base + carry as u64) {
        starts.pop();
    }
    Ok(starts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::CompiledProfile;
    use std::path::Path;

    fn compiled(toml: &str) -> CompiledProfile {
        crate::parser::testing::compiled(toml)
    }

    fn plan_of(profile: &CompiledProfile, bytes: &[u8], whole: bool) -> Result<FilePlan, String> {
        let ctx = ImportCtx {
            profile,
            path: Path::new("x.log"),
            file_date: None,
        };
        plan(&Importers::default(), &ctx, bytes, whole)
    }

    #[test]
    fn plans_text_with_format_and_day_order() {
        let p = compiled(
            "[format]\nregexes = ['^(?P<timestamp>\\d\\d/\\d\\d/\\d{4} \\d\\d:\\d\\d:\\d\\d) (?P<message>.*)$']\noutput_templates = []",
        );
        let plan = plan_of(&p, b"03/04/2026 10:00:00 a\n25/04/2026 10:00:00 b\n", true).unwrap();
        assert_eq!(plan.importer, "text");
        assert_eq!(
            plan.kind,
            PlanKind::Text {
                format: LineFormat::Text(0),
                day_first: Some(true),
                time_only: false
            }
        );
        assert!(plan.multiline && plan.splittable);
        assert_eq!((plan.encoding, plan.data_start), (Encoding::Utf8, 0));
        // The profile's day order wins.
        let p = compiled("[format]\nday_first = false\nregexes = ['^(?P<timestamp>\\d\\d/\\d\\d/\\d{4} \\d\\d:\\d\\d:\\d\\d) (?P<message>.*)$']\noutput_templates = []");
        let plan = plan_of(&p, b"25/04/2026 10:00:00 b\n", true).unwrap();
        assert!(matches!(
            plan.kind,
            PlanKind::Text {
                day_first: Some(false),
                ..
            }
        ));
    }

    #[test]
    fn time_only_text_is_not_splittable() {
        let p =
            compiled("[format]\noutput_templates = [\"[{Timestamp:HH:mm:ss} {Level:u3}] {Message:lj}{NewLine}{Exception}\"]\nregexes = []");
        let plan = plan_of(&p, b"[23:59:59 INF] late\n", true).unwrap();
        assert!(matches!(plan.kind, PlanKind::Text { time_only: true, .. }));
        assert!(!plan.splittable);
    }

    #[test]
    fn plans_json_shapes() {
        let p = compiled("");
        let lines = plan_of(&p, b"{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"a\"}\n", true).unwrap();
        assert_eq!(
            (lines.importer, lines.kind, lines.splittable),
            ("json".to_string(), PlanKind::Json(JsonShape::Lines), true)
        );
        let array = plan_of(&p, b"[{\"@t\":\"2026-10-14T10:00:00Z\"}]", true).unwrap();
        assert_eq!(array.kind, PlanKind::Json(JsonShape::Array));
        assert!(!array.splittable);
        let single = plan_of(&p, b"{\n  \"@t\": \"2026-10-14T10:00:00Z\",\n  \"@m\": \"a\"\n}\n", true).unwrap();
        assert_eq!(single.kind, PlanKind::Json(JsonShape::Single));
        // Cut off: pretty objects one after another are read as lines.
        let cut = plan_of(&p, b"{\n  \"@t\": \"2026-10-14T10:00:00Z\",\n  \"@m\": \"a\"\n}\n{\n", false).unwrap();
        assert_eq!(cut.kind, PlanKind::Json(JsonShape::Lines));
    }

    #[test]
    fn plans_other_importers_and_refuses_unknown_files() {
        let p = compiled("");
        let csv = plan_of(&p, b"a,b\n1,2\n", true).unwrap();
        assert_eq!((csv.importer.as_str(), csv.kind), ("csv", PlanKind::Legacy));
        assert_eq!(plan_of(&p, b"", true).unwrap_err(), "empty file");
        assert_eq!(plan_of(&p, b"just words", true).unwrap_err(), "no importer recognises this format");
        assert_eq!(
            plan_of(&p, &[0u8, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 1, 2, 3], true).unwrap_err(),
            "binary file"
        );
        let forced = compiled("[format]\nkind = 'text'\nregexes = ['^(?P<timestamp>NEVER) (?P<message>.*)$']\noutput_templates = []");
        assert_eq!(
            plan_of(&forced, b"2026-10-14 10:00:00 INFO a\n", true).unwrap_err(),
            "no output template or regex in the profile matches this file"
        );
    }

    #[test]
    fn detects_encodings_and_decodes_whole_lines_of_the_head() {
        assert_eq!(detect_encoding(b"\xEF\xBB\xBFx"), (Encoding::Utf8, 3));
        assert_eq!(detect_encoding(b"\xFF\xFEx\0"), (Encoding::Utf16Le, 2));
        assert_eq!(detect_encoding(b"\xFE\xFF\0x"), (Encoding::Utf16Be, 2));
        let le: Vec<u8> = "no bom here".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(detect_encoding(&le), (Encoding::Utf16Le, 0));
        assert_eq!(detect_encoding(b"plain"), (Encoding::Utf8, 0));
        assert_eq!(decode_head(b"a\nb\npart", Encoding::Utf8, 0, false), "a\nb\n");
        assert_eq!(decode_head(b"a\r\nb", Encoding::Utf8, 0, true), "a\r\nb");
        let p = compiled("");
        let mut bom16: Vec<u8> = vec![0xFF, 0xFE];
        bom16.extend("2026-10-14 10:00:00 INFO a\n".encode_utf16().flat_map(|u| u.to_le_bytes()));
        let plan = plan_of(&p, &bom16, true).unwrap();
        assert_eq!(
            (plan.encoding, plan.data_start, plan.importer.as_str()),
            (Encoding::Utf16Le, 2, "text")
        );
    }

    #[test]
    fn chunk_starts_are_line_aligned() {
        let text = "aaaa\nbbbb\ncccc\ndddd\neeee\n";
        let starts = scan_chunk_starts(text.as_bytes(), Encoding::Utf8, 0, 8).unwrap();
        assert_eq!(
            starts,
            [
                ChunkStart { byte_offset: 0, line: 1 },
                ChunkStart { byte_offset: 10, line: 3 },
                ChunkStart { byte_offset: 20, line: 5 }
            ]
        );
        let one = scan_chunk_starts(text.as_bytes(), Encoding::Utf8, 0, 1 << 20).unwrap();
        assert_eq!(one.len(), 1);
        let le: Vec<u8> = text.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        let starts16 = scan_chunk_starts(&le[..], Encoding::Utf16Le, 2, 16).unwrap();
        assert_eq!(starts16.iter().map(|s| s.line).collect::<Vec<_>>(), [1, 3, 5]);
        assert_eq!(starts16[1].byte_offset, 2 + 20);
    }

    #[test]
    fn a_start_at_the_very_end_of_the_data_begins_no_chunk() {
        // The second line ends exactly where the data does: that offset would start an empty chunk.
        let starts = scan_chunk_starts("aaaa\nbbbb\n".as_bytes(), Encoding::Utf8, 0, 5).unwrap();
        assert_eq!(
            starts,
            [ChunkStart { byte_offset: 0, line: 1 }, ChunkStart { byte_offset: 5, line: 2 }]
        );
    }
}
