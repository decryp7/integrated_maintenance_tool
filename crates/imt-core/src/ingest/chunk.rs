//! Decoded text in bounded chunks of whole lines, with each line's exact byte offset in the file.
//!
//! A [`ChunkSource`] never holds a whole file: [`ReaderChunks`] reads at most [`CHUNK_BYTES`] at a time,
//! splits on the newline of the file's encoding (so UTF-16 offsets stay exact), decodes line by line and
//! skips lines longer than [`MAX_LINE_LEN`] without buffering them (they are reported, numbered, as
//! too long).

use crate::importers::MAX_LINE_LEN;
use std::collections::VecDeque;
use std::io::Read;

/// Raw bytes read per chunk (8 MiB).
pub const CHUNK_BYTES: usize = 8 << 20;

/// A text file's encoding, as the file plan detected it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

impl Encoding {
    /// Bytes per code unit.
    pub fn unit(self) -> usize {
        match self {
            Encoding::Utf8 => 1,
            Encoding::Utf16Le | Encoding::Utf16Be => 2,
        }
    }

    /// Offset of the first newline unit in `bytes` (unit-aligned).
    pub fn find_newline(self, bytes: &[u8]) -> Option<usize> {
        match self {
            Encoding::Utf8 => bytes.iter().position(|b| *b == b'\n'),
            Encoding::Utf16Le => bytes.as_chunks::<2>().0.iter().position(|u| *u == [b'\n', 0]).map(|i| i * 2),
            Encoding::Utf16Be => bytes.as_chunks::<2>().0.iter().position(|u| *u == [0, b'\n']).map(|i| i * 2),
        }
    }

    /// Newlines in `bytes` (unit-aligned).
    pub fn count_newlines(self, bytes: &[u8]) -> usize {
        match self {
            Encoding::Utf8 => bytes.iter().filter(|b| **b == b'\n').count(),
            Encoding::Utf16Le => bytes.as_chunks::<2>().0.iter().filter(|u| **u == [b'\n', 0]).count(),
            Encoding::Utf16Be => bytes.as_chunks::<2>().0.iter().filter(|u| **u == [0, b'\n']).count(),
        }
    }

    /// Decodes one line's bytes (lossy), appending to `out`.
    pub fn decode_into(self, bytes: &[u8], out: &mut String) {
        match self {
            Encoding::Utf8 => match std::str::from_utf8(bytes) {
                Ok(s) => out.push_str(s),
                Err(_) => out.push_str(&String::from_utf8_lossy(bytes)),
            },
            Encoding::Utf16Le => out.push_str(&crate::importers::utf16(bytes, u16::from_le_bytes)),
            Encoding::Utf16Be => out.push_str(&crate::importers::utf16(bytes, u16::from_be_bytes)),
        }
    }
}

/// One line of a chunk: where its text is in [`Chunk::text`] and where it starts in the file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LineSpan {
    pub start: usize,
    pub len: usize,
    /// 1-based line number in the file.
    pub number: u32,
    /// Offset of the line's first byte in the file.
    pub byte_offset: u64,
    /// Longer than [`MAX_LINE_LEN`] bytes as stored: skipped, its text is empty.
    pub too_long: bool,
}

/// Decoded whole lines (without their newline and one trailing `\r`).
#[derive(Debug, Default, Clone)]
pub struct Chunk {
    pub text: String,
    pub lines: Vec<LineSpan>,
}

impl Chunk {
    pub fn clear(&mut self) {
        self.text.clear();
        self.lines.clear();
    }

    pub fn line(&self, span: &LineSpan) -> &str {
        &self.text[span.start..span.start + span.len]
    }
}

/// A source of decoded text in chunks that end on line boundaries.
pub trait ChunkSource {
    /// Replaces `out` with the next chunk; false at the end (then `out` is empty).
    fn next_chunk(&mut self, out: &mut Chunk) -> std::io::Result<bool>;
}

/// [`ChunkSource`] over a reader positioned at the first byte after any byte-order mark.
pub struct ReaderChunks<R: Read> {
    reader: R,
    encoding: Encoding,
    buf: Vec<u8>,
    /// File offset of `buf[0]`.
    base: u64,
    next_number: u32,
    eof: bool,
    /// Start offset of an over-long line being skipped.
    skipping: Option<u64>,
    block: usize,
    max_line: usize,
}

impl<R: Read> ReaderChunks<R> {
    /// Reads `reader`, whose first byte is at file offset `start` and is line 1.
    pub fn new(reader: R, encoding: Encoding, start: u64) -> ReaderChunks<R> {
        Self::with_limits(reader, encoding, start, CHUNK_BYTES, MAX_LINE_LEN)
    }

    /// Like [`ReaderChunks::new`] with a chosen read size and line limit (for tests).
    pub fn with_limits(reader: R, encoding: Encoding, start: u64, block: usize, max_line: usize) -> ReaderChunks<R> {
        ReaderChunks {
            reader,
            encoding,
            buf: Vec::new(),
            base: start,
            next_number: 1,
            eof: false,
            skipping: None,
            block: block.max(2),
            max_line,
        }
    }

    /// Appends up to one block of bytes to `buf`.
    fn fill(&mut self) -> std::io::Result<()> {
        let len = self.buf.len();
        self.buf.resize(len + self.block, 0);
        let mut read = 0;
        while read < self.block {
            match self.reader.read(&mut self.buf[len + read..]) {
                Ok(0) => {
                    self.eof = true;
                    break;
                }
                Ok(n) => read += n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                Err(e) => {
                    self.buf.truncate(len + read);
                    return Err(e);
                }
            }
        }
        self.buf.truncate(len + read);
        Ok(())
    }

    /// Adds the line `buf[start..end]` (or the over-long line being skipped) to `out`.
    fn emit(&mut self, out: &mut Chunk, start: usize, end: usize) {
        let number = self.next_number;
        self.next_number = self.next_number.saturating_add(1);
        if let Some(offset) = self.skipping.take() {
            out.lines.push(LineSpan {
                start: out.text.len(),
                len: 0,
                number,
                byte_offset: offset,
                too_long: true,
            });
            return;
        }
        let byte_offset = self.base + start as u64;
        let bytes = &self.buf[start..end];
        if bytes.len() > self.max_line {
            out.lines.push(LineSpan {
                start: out.text.len(),
                len: 0,
                number,
                byte_offset,
                too_long: true,
            });
            return;
        }
        let at = out.text.len();
        self.encoding.decode_into(bytes, &mut out.text);
        if out.text.ends_with('\r') {
            out.text.pop();
        }
        out.lines.push(LineSpan {
            start: at,
            len: out.text.len() - at,
            number,
            byte_offset,
            too_long: false,
        });
    }

    /// Drops `buf[..n]`.
    fn consume(&mut self, n: usize) {
        self.buf.drain(..n);
        self.base += n as u64;
    }
}

impl<R: Read> ChunkSource for ReaderChunks<R> {
    fn next_chunk(&mut self, out: &mut Chunk) -> std::io::Result<bool> {
        out.clear();
        let unit = self.encoding.unit();
        loop {
            let mut pos = 0;
            while let Some(nl) = self.encoding.find_newline(&self.buf[pos..]) {
                self.emit(out, pos, pos + nl);
                pos += nl + unit;
            }
            self.consume(pos);
            // The rest is a partial line: past the limit it is skipped without being kept.
            if self.skipping.is_some() || self.buf.len() > self.max_line {
                if self.skipping.is_none() {
                    self.skipping = Some(self.base);
                }
                let whole = self.buf.len() - self.buf.len() % unit;
                self.consume(whole);
            }
            if !out.lines.is_empty() {
                return Ok(true);
            }
            if self.eof {
                if !self.buf.is_empty() || self.skipping.is_some() {
                    let end = self.buf.len() - self.buf.len() % unit;
                    self.emit(out, 0, end);
                    self.buf.clear();
                }
                return Ok(!out.lines.is_empty());
            }
            self.fill()?;
        }
    }
}

/// A line read through a [`LineReader`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Line<'a> {
    pub text: &'a str,
    pub number: u32,
    pub byte_offset: u64,
    pub too_long: bool,
}

/// An owned [`Line`], for look-ahead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OwnedLine {
    pub text: String,
    pub number: u32,
    pub byte_offset: u64,
    pub too_long: bool,
}

impl OwnedLine {
    pub fn as_line(&self) -> Line<'_> {
        Line {
            text: &self.text,
            number: self.number,
            byte_offset: self.byte_offset,
            too_long: self.too_long,
        }
    }
}

/// Lines of a [`ChunkSource`] one at a time, with look-ahead that can be given back.
pub struct LineReader<'s> {
    source: &'s mut dyn ChunkSource,
    chunk: Chunk,
    pos: usize,
    ahead: VecDeque<OwnedLine>,
    current: OwnedLine,
    done: bool,
}

impl<'s> LineReader<'s> {
    pub fn new(source: &'s mut dyn ChunkSource) -> LineReader<'s> {
        LineReader {
            source,
            chunk: Chunk::default(),
            pos: 0,
            ahead: VecDeque::new(),
            current: OwnedLine {
                text: String::new(),
                number: 0,
                byte_offset: 0,
                too_long: false,
            },
            done: false,
        }
    }

    /// Makes sure the current chunk has an unread line; false at the end.
    fn ensure(&mut self) -> std::io::Result<bool> {
        while self.pos >= self.chunk.lines.len() {
            if self.done || !self.source.next_chunk(&mut self.chunk)? {
                self.done = true;
                return Ok(false);
            }
            self.pos = 0;
        }
        Ok(true)
    }

    /// The next line, or `None` at the end.
    pub fn next_line(&mut self) -> std::io::Result<Option<Line<'_>>> {
        if let Some(l) = self.ahead.pop_front() {
            self.current = l;
            return Ok(Some(self.current.as_line()));
        }
        if !self.ensure()? {
            return Ok(None);
        }
        let span = self.chunk.lines[self.pos];
        self.pos += 1;
        Ok(Some(Line {
            text: self.chunk.line(&span),
            number: span.number,
            byte_offset: span.byte_offset,
            too_long: span.too_long,
        }))
    }

    /// The next line as an owned value (for look-ahead), or `None` at the end.
    pub fn next_owned(&mut self) -> std::io::Result<Option<OwnedLine>> {
        Ok(self.next_line()?.map(|l| OwnedLine {
            text: l.text.to_string(),
            number: l.number,
            byte_offset: l.byte_offset,
            too_long: l.too_long,
        }))
    }

    /// Puts look-ahead lines back, to be read again in order.
    pub fn unread(&mut self, lines: Vec<OwnedLine>) {
        for l in lines.into_iter().rev() {
            self.ahead.push_front(l);
        }
    }
}

/// [`ChunkSource`] over decoded text already in memory (one chunk), for importers that read whole files.
pub struct TextChunks<'a> {
    text: Option<&'a str>,
}

impl<'a> TextChunks<'a> {
    pub fn new(text: &'a str) -> TextChunks<'a> {
        TextChunks { text: Some(text) }
    }
}

impl ChunkSource for TextChunks<'_> {
    fn next_chunk(&mut self, out: &mut Chunk) -> std::io::Result<bool> {
        out.clear();
        let Some(text) = self.text.take() else { return Ok(false) };
        let mut offset = 0u64;
        for (i, line) in text.split_inclusive('\n').enumerate() {
            let body = line.strip_suffix('\n').unwrap_or(line);
            let body = body.strip_suffix('\r').unwrap_or(body);
            out.lines.push(LineSpan {
                start: out.text.len(),
                len: body.len(),
                number: i as u32 + 1,
                byte_offset: offset,
                too_long: body.len() > MAX_LINE_LEN,
            });
            if body.len() <= MAX_LINE_LEN {
                out.text.push_str(body);
            } else if let Some(last) = out.lines.last_mut() {
                last.len = 0;
            }
            offset += line.len() as u64;
        }
        Ok(!out.lines.is_empty())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn all_lines(src: &mut dyn ChunkSource) -> (Vec<OwnedLine>, usize) {
        let mut chunks = 0;
        let mut out = Vec::new();
        let mut c = Chunk::default();
        while src.next_chunk(&mut c).unwrap() {
            chunks += 1;
            for s in &c.lines {
                out.push(OwnedLine {
                    text: c.line(s).to_string(),
                    number: s.number,
                    byte_offset: s.byte_offset,
                    too_long: s.too_long,
                });
            }
        }
        (out, chunks)
    }

    fn texts(lines: &[OwnedLine]) -> Vec<&str> {
        lines.iter().map(|l| l.text.as_str()).collect()
    }

    #[test]
    fn utf8_lines_match_str_lines_with_exact_offsets() {
        let text = "alpha\r\nbeta\n\nγάμμα\ndelta";
        for block in [1, 2, 3, 5, 64] {
            let mut src = ReaderChunks::with_limits(text.as_bytes(), Encoding::Utf8, 0, block, 100);
            let (lines, _) = all_lines(&mut src);
            assert_eq!(texts(&lines), text.lines().collect::<Vec<_>>(), "block {block}");
            let offsets: Vec<u64> = lines.iter().map(|l| l.byte_offset).collect();
            assert_eq!(offsets, [0, 7, 12, 13, 24]);
            assert_eq!(lines.iter().map(|l| l.number).collect::<Vec<_>>(), [1, 2, 3, 4, 5]);
        }
        // A trailing newline adds no empty line; an empty input has none.
        let (lines, _) = all_lines(&mut ReaderChunks::new("a\n".as_bytes(), Encoding::Utf8, 0));
        assert_eq!(texts(&lines), ["a"]);
        assert!(all_lines(&mut ReaderChunks::new("".as_bytes(), Encoding::Utf8, 0)).0.is_empty());
        let (lines, _) = all_lines(&mut ReaderChunks::new("\n".as_bytes(), Encoding::Utf8, 0));
        assert_eq!(texts(&lines), [""]);
    }

    #[test]
    fn invalid_utf8_is_replaced_per_line() {
        let bytes = b"ok\nbad\xff\xfe\nfine";
        let (lines, _) = all_lines(&mut ReaderChunks::new(&bytes[..], Encoding::Utf8, 0));
        assert_eq!(texts(&lines), ["ok", "bad\u{fffd}\u{fffd}", "fine"]);
        assert_eq!(lines[2].byte_offset, 9);
    }

    fn utf16(text: &str, le: bool) -> Vec<u8> {
        text.encode_utf16()
            .flat_map(|u| if le { u.to_le_bytes() } else { u.to_be_bytes() })
            .collect()
    }

    #[test]
    fn utf16_splits_on_aligned_newlines_with_byte_offsets() {
        // U+010A has 0x0A in a byte of its code unit: it must not split a line.
        let text = "Ċ one\r\n𝄞 two\nthree";
        for (le, enc) in [(true, Encoding::Utf16Le), (false, Encoding::Utf16Be)] {
            let bytes = utf16(text, le);
            for block in [2, 3, 4, 7, 1024] {
                // The reader starts after a 2-byte BOM.
                let mut src = ReaderChunks::with_limits(&bytes[..], enc, 2, block, 1000);
                let (lines, _) = all_lines(&mut src);
                assert_eq!(texts(&lines), ["Ċ one", "𝄞 two", "three"], "{enc:?} block {block}");
                let offsets: Vec<u64> = lines.iter().map(|l| l.byte_offset).collect();
                assert_eq!(offsets, [2, 2 + 14, 2 + 14 + 14], "{enc:?}");
            }
        }
    }

    #[test]
    fn oversized_lines_are_skipped_without_buffering_and_keep_numbering() {
        let long = "x".repeat(50);
        let text = format!("a\n{long}\nb\n{long}");
        for block in [4, 16, 1000] {
            let mut src = ReaderChunks::with_limits(text.as_bytes(), Encoding::Utf8, 0, block, 10);
            let (lines, _) = all_lines(&mut src);
            assert_eq!(texts(&lines), ["a", "", "b", ""], "block {block}");
            assert_eq!(lines.iter().map(|l| l.too_long).collect::<Vec<_>>(), [false, true, false, true]);
            assert_eq!(lines.iter().map(|l| l.number).collect::<Vec<_>>(), [1, 2, 3, 4]);
            assert_eq!(lines.iter().map(|l| l.byte_offset).collect::<Vec<_>>(), [0, 2, 53, 55]);
            assert!(src.buf.capacity() < 1000 || block == 1000, "the long line was not buffered");
        }
    }

    #[test]
    fn chunks_are_bounded_by_the_block_size() {
        let text = "0123456789\n".repeat(100);
        let mut src = ReaderChunks::with_limits(text.as_bytes(), Encoding::Utf8, 0, 64, 100);
        let (lines, chunks) = all_lines(&mut src);
        assert_eq!(lines.len(), 100);
        assert!(chunks >= 100 * 11 / 64, "{chunks} chunks");
    }

    struct Failing;
    impl Read for Failing {
        fn read(&mut self, _buf: &mut [u8]) -> std::io::Result<usize> {
            Err(std::io::Error::other("disk gone"))
        }
    }

    #[test]
    fn read_errors_are_reported() {
        let mut src = ReaderChunks::new(Failing, Encoding::Utf8, 0);
        assert!(src.next_chunk(&mut Chunk::default()).is_err());
    }

    #[test]
    fn line_reader_gives_look_ahead_back() {
        let text = "1\n2\n3\n4";
        let mut src = ReaderChunks::with_limits(text.as_bytes(), Encoding::Utf8, 0, 2, 100);
        let mut r = LineReader::new(&mut src);
        assert_eq!(r.next_line().unwrap().unwrap().text, "1");
        let a = r.next_owned().unwrap().unwrap();
        let b = r.next_owned().unwrap().unwrap();
        assert_eq!((a.text.as_str(), b.text.as_str()), ("2", "3"));
        r.unread(vec![a, b]);
        let mut rest = Vec::new();
        while let Some(l) = r.next_line().unwrap() {
            rest.push((l.text.to_string(), l.number));
        }
        assert_eq!(rest, [("2".into(), 2), ("3".into(), 3), ("4".into(), 4)]);
        assert!(r.next_line().unwrap().is_none());
    }

    #[test]
    fn text_chunks_mirror_str_lines() {
        let text = "a\r\nb\n\nc";
        let (lines, chunks) = all_lines(&mut TextChunks::new(text));
        assert_eq!(chunks, 1);
        assert_eq!(texts(&lines), text.lines().collect::<Vec<_>>());
        assert_eq!(lines.iter().map(|l| l.byte_offset).collect::<Vec<_>>(), [0, 3, 5, 6]);
        let huge = format!("{}\nok", "y".repeat(MAX_LINE_LEN + 1));
        let (lines, _) = all_lines(&mut TextChunks::new(&huge));
        assert!(lines[0].too_long && lines[0].text.is_empty());
        assert_eq!(lines[1].text, "ok");
        assert!(all_lines(&mut TextChunks::new("")).0.is_empty());
    }

    #[test]
    fn encodings_count_and_find_newlines() {
        assert_eq!(Encoding::Utf8.count_newlines(b"a\nb\n"), 2);
        assert_eq!(Encoding::Utf16Le.count_newlines(&utf16("a\nb\n", true)), 2);
        assert_eq!(Encoding::Utf16Be.count_newlines(&utf16("a\nb", false)), 1);
        assert_eq!(Encoding::Utf16Be.find_newline(&utf16("ab\n", false)), Some(4));
        assert_eq!(Encoding::Utf8.unit(), 1);
    }
}
