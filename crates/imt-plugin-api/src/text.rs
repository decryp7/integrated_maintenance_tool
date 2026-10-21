//! Decoding file bytes to text, shared by the engine and plugins.

/// Lines longer than this (4 MiB) are skipped and counted, so one corrupt line cannot blow up memory in later
/// processing.
pub const MAX_LINE_LEN: usize = 4 << 20;

/// Decodes file bytes: UTF-8 (with or without BOM) or UTF-16 LE/BE, which is common for files written by
/// Windows tools. Returns None for binary data.
pub fn decode(bytes: &[u8]) -> Option<String> {
    let text = if let Some(rest) = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]) {
        String::from_utf8_lossy(rest).into_owned()
    } else if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        utf16(rest, u16::from_le_bytes)
    } else if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        utf16(rest, u16::from_be_bytes)
    } else if looks_utf16le(bytes) {
        utf16(bytes, u16::from_le_bytes)
    } else {
        String::from_utf8_lossy(bytes).into_owned()
    };
    if looks_binary(&text) {
        return None;
    }
    Some(text)
}

/// Whether decoded text is binary data: too many control characters in its first 8192 characters.
pub fn looks_binary(text: &str) -> bool {
    let sample: String = text.chars().take(8192).collect();
    let control = sample
        .chars()
        .filter(|c| c.is_control() && !matches!(c, '\n' | '\r' | '\t' | '\x0c'))
        .count();
    control > sample.len() / 100 + 2
}

/// UTF-16 code units from byte pairs (a trailing odd byte is ignored), decoded lossily.
pub fn utf16(bytes: &[u8], f: fn([u8; 2]) -> u16) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| f([c[0], c[1]])).collect();
    String::from_utf16_lossy(&units)
}

/// Whether BOM-less bytes look like UTF-16 LE (mostly zero high bytes).
pub fn looks_utf16le(bytes: &[u8]) -> bool {
    let n = bytes.len().min(512) & !1;
    n >= 8 && bytes[..n].iter().skip(1).step_by(2).filter(|b| **b == 0).count() * 10 > n / 2 * 9
}

/// The first `n` non-blank lines, trimmed at the end.
pub fn first_lines(text: &str, n: usize) -> impl Iterator<Item = &str> {
    text.lines().map(str::trim_end).filter(|l| !l.trim().is_empty()).take(n)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_utf16() {
        let s = "Windows Registry Editor Version 5.00\r\n";
        let mut bytes = vec![0xFF, 0xFE];
        for u in s.encode_utf16() {
            bytes.extend(u.to_le_bytes());
        }
        assert_eq!(decode(&bytes).unwrap(), s);
        assert!(decode(&[0u8, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 1, 2, 3]).is_none());
    }

    #[test]
    fn decodes_variants() {
        assert_eq!(decode(b"\xEF\xBB\xBFhello").unwrap(), "hello");
        let mut be = vec![0xFE, 0xFF];
        for u in "hi".encode_utf16() {
            be.extend(u.to_be_bytes());
        }
        assert_eq!(decode(&be).unwrap(), "hi");
        let le: Vec<u8> = "no bom here".encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
        assert_eq!(decode(&le).unwrap(), "no bom here");
        assert_eq!(first_lines("a\n\n  \nb\nc", 2).collect::<Vec<_>>(), vec!["a", "b"]);
    }
}
