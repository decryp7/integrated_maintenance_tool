//! Byte-level reading for live tail: the text encoding found at import,
//! newline positions in raw bytes (so offsets stay byte offsets for UTF-16
//! too), bounded range reads and decoding of a region.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// How a file's text is encoded, decided from its first bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Encoding {
    Utf8,
    Utf16Le,
    Utf16Be,
}

impl Encoding {
    /// The encoding and the length of the byte-order mark, if any.
    pub fn detect(head: &[u8]) -> (Encoding, u64) {
        if head.starts_with(&[0xEF, 0xBB, 0xBF]) {
            (Encoding::Utf8, 3)
        } else if head.starts_with(&[0xFF, 0xFE]) {
            (Encoding::Utf16Le, 2)
        } else if head.starts_with(&[0xFE, 0xFF]) {
            (Encoding::Utf16Be, 2)
        } else if looks_utf16le(head) {
            (Encoding::Utf16Le, 0)
        } else {
            (Encoding::Utf8, 0)
        }
    }

    fn unit(self) -> usize {
        match self {
            Encoding::Utf8 => 1,
            _ => 2,
        }
    }

    fn is_newline(self, unit: &[u8]) -> bool {
        match self {
            Encoding::Utf8 => unit[0] == b'\n',
            Encoding::Utf16Le => unit == [b'\n', 0],
            Encoding::Utf16Be => unit == [0, b'\n'],
        }
    }

    /// Byte positions just after each newline in `bytes` (which starts on a
    /// code-unit boundary).
    pub fn line_ends(self, bytes: &[u8]) -> impl Iterator<Item = usize> + '_ {
        let unit = self.unit();
        bytes
            .chunks_exact(unit)
            .enumerate()
            .filter(move |(_, u)| self.is_newline(u))
            .map(move |(i, _)| (i + 1) * unit)
    }

    /// The position after the last newline in `bytes`, or 0 without one.
    pub fn complete_len(self, bytes: &[u8]) -> usize {
        let unit = self.unit();
        let whole = bytes.len() - bytes.len() % unit;
        (0..whole / unit)
            .rev()
            .find(|&i| self.is_newline(&bytes[i * unit..(i + 1) * unit]))
            .map_or(0, |i| (i + 1) * unit)
    }

    /// The byte position where 1-based `line` starts in `bytes`.
    pub fn line_start(self, bytes: &[u8], line: u32) -> Option<usize> {
        if line <= 1 {
            return Some(0);
        }
        self.line_ends(bytes).nth(line as usize - 2)
    }

    /// Text of a region that starts on a code-unit boundary after any BOM.
    pub fn decode(self, bytes: &[u8]) -> String {
        match self {
            Encoding::Utf8 => String::from_utf8_lossy(bytes).into_owned(),
            Encoding::Utf16Le => utf16(bytes, u16::from_le_bytes),
            Encoding::Utf16Be => utf16(bytes, u16::from_be_bytes),
        }
    }
}

fn utf16(bytes: &[u8], f: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| f(*c)).collect();
    String::from_utf16_lossy(&units)
}

fn looks_utf16le(bytes: &[u8]) -> bool {
    let n = bytes.len().min(512) & !1;
    n >= 8 && bytes[..n].iter().skip(1).step_by(2).filter(|b| **b == 0).count() * 10 > n / 2 * 9
}

/// Where tail reads bytes from: a file on disk, or the decompressed content
/// of a rotated archive held in memory.
pub trait ByteSource {
    fn len(&self) -> u64;
    /// Bytes `[start, end)`; fewer when the source is shorter.
    fn read(&self, start: u64, end: u64) -> std::io::Result<Vec<u8>>;
}

/// A file on disk; `len` is the size observed when the refresh started, so a
/// file that keeps growing during the read is read only up to that size.
pub struct FileBytes<'a> {
    pub path: &'a Path,
    pub len: u64,
}

impl ByteSource for FileBytes<'_> {
    fn len(&self) -> u64 {
        self.len
    }

    fn read(&self, start: u64, end: u64) -> std::io::Result<Vec<u8>> {
        let end = end.min(self.len);
        if end <= start {
            return Ok(Vec::new());
        }
        let mut file = super::super::open_with_retry(self.path)?;
        file.seek(SeekFrom::Start(start))?;
        let mut out = Vec::with_capacity((end - start) as usize);
        file.take(end - start).read_to_end(&mut out)?;
        Ok(out)
    }
}

impl ByteSource for Vec<u8> {
    fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }

    fn read(&self, start: u64, end: u64) -> std::io::Result<Vec<u8>> {
        let end = (end as usize).min(self.as_slice().len());
        let start = (start as usize).min(end);
        Ok(self[start..end].to_vec())
    }
}

/// Counts the newlines of a file up to `end`, streaming (for start-at-end).
pub fn count_lines(src: &dyn ByteSource, enc: Encoding, end: u64) -> std::io::Result<u32> {
    const CHUNK: u64 = 1 << 20;
    let mut lines = 0u32;
    let mut pos = 0;
    while pos < end {
        let next = (pos + CHUNK).min(end);
        let bytes = src.read(pos, next)?;
        if bytes.is_empty() {
            break;
        }
        lines = lines.saturating_add(enc.line_ends(&bytes).count() as u32);
        pos += bytes.len() as u64;
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn le(s: &str) -> Vec<u8> {
        s.encode_utf16().flat_map(|u| u.to_le_bytes()).collect()
    }

    #[test]
    fn encodings_and_lines() {
        assert_eq!(Encoding::detect(b"\xEF\xBB\xBFx"), (Encoding::Utf8, 3));
        assert_eq!(Encoding::detect(&[0xFF, 0xFE, b'a', 0]), (Encoding::Utf16Le, 2));
        assert_eq!(Encoding::detect(&[0xFE, 0xFF, 0, b'a']), (Encoding::Utf16Be, 2));
        assert_eq!(Encoding::detect(&le("abcdefgh")), (Encoding::Utf16Le, 0));
        assert_eq!(Encoding::detect(b"plain"), (Encoding::Utf8, 0));

        let u = Encoding::Utf8;
        assert_eq!(u.complete_len(b"a\nb\nc"), 4);
        assert_eq!(u.complete_len(b"abc"), 0);
        assert_eq!(u.line_start(b"a\nbb\nc", 3), Some(5));
        assert_eq!(u.line_start(b"a\nbb\nc", 1), Some(0));
        assert_eq!(u.line_start(b"a", 5), None);

        let w = Encoding::Utf16Le;
        let bytes = le("ab\ncd\ne");
        assert_eq!(w.complete_len(&bytes), 12);
        assert_eq!(w.line_start(&bytes, 2), Some(6));
        assert_eq!(w.decode(&bytes[6..]), "cd\ne");
        // A newline byte inside another code unit is not a newline.
        assert_eq!(w.complete_len(&[b'\n', 1, b'x']), 0);
        let be: Vec<u8> = "x\ny".encode_utf16().flat_map(|u| u.to_be_bytes()).collect();
        assert_eq!(Encoding::Utf16Be.complete_len(&be), 4);
        assert_eq!(Encoding::Utf16Be.decode(&be), "x\ny");
    }

    #[test]
    fn byte_sources() {
        let mem: Vec<u8> = b"a\nb\nc\n".to_vec();
        assert_eq!(ByteSource::len(&mem), 6);
        assert_eq!(mem.read(2, 100).unwrap(), b"b\nc\n");
        assert!(mem.read(9, 3).unwrap().is_empty());
        assert_eq!(count_lines(&mem, Encoding::Utf8, 6).unwrap(), 3);
        let dir = std::env::temp_dir().join(format!("imt-reader-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("f.log");
        std::fs::write(&p, "0123456789").unwrap();
        let f = FileBytes { path: &p, len: 8 };
        assert_eq!(f.read(2, 100).unwrap(), b"234567");
        assert!(f.read(8, 9).unwrap().is_empty());
        assert_eq!(count_lines(&f, Encoding::Utf8, 8).unwrap(), 0);
        // A file shorter than the size it was seen with ends the count.
        let longer = FileBytes { path: &p, len: 30 };
        assert_eq!(count_lines(&longer, Encoding::Utf8, 30).unwrap(), 0);
        let gone = FileBytes {
            path: &dir.join("missing"),
            len: 4,
        };
        assert!(gone.read(0, 4).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
