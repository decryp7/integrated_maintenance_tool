//! Incremental UTF-8 decoding for token pieces: a multi-byte character can be split across tokens.

#[derive(Default)]
pub struct Utf8Stream {
    pending: Vec<u8>,
}

impl Utf8Stream {
    pub fn new() -> Self {
        Self::default()
    }

    /// Adds bytes; returns the text that is now complete (invalid bytes become U+FFFD).
    pub fn push(&mut self, bytes: &[u8]) -> String {
        self.pending.extend_from_slice(bytes);
        let mut out = String::new();
        loop {
            match std::str::from_utf8(&self.pending) {
                Ok(s) => {
                    out.push_str(s);
                    self.pending.clear();
                    return out;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    out.push_str(&String::from_utf8_lossy(&self.pending[..valid]));
                    match e.error_len() {
                        Some(n) => {
                            out.push('\u{FFFD}');
                            self.pending.drain(..valid + n);
                        }
                        None => {
                            self.pending.drain(..valid);
                            return out;
                        }
                    }
                }
            }
        }
    }

    /// Flushes an incomplete tail at the end of the stream.
    pub fn finish(&mut self) -> String {
        let s = String::from_utf8_lossy(&self.pending).into_owned();
        self.pending.clear();
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joins_split_characters() {
        let mut u = Utf8Stream::new();
        let bytes = "é日🙂".as_bytes();
        let mut out = String::new();
        for b in bytes {
            out.push_str(&u.push(&[*b]));
        }
        assert_eq!(out, "é日🙂");
        assert_eq!(u.finish(), "");
    }

    #[test]
    fn invalid_and_truncated_bytes_do_not_panic() {
        let mut u = Utf8Stream::new();
        assert_eq!(u.push(&[b'a', 0xFF, b'b']), "a\u{FFFD}b");
        assert_eq!(u.push(&[0xE6, 0x97]), "");
        assert_eq!(u.finish(), "\u{FFFD}");
    }
}
