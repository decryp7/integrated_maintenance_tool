//! A bounds-checked little-endian reader over untrusted bytes. Every read
//! checks the remaining length first, so a damaged file is an error with the
//! byte offset, never a panic or an allocation sized by the input.

use super::FORMAT;
use debrief_profile::{ProfileError, Result};

/// Reads primitives from `data`; offsets in errors are `base + pos` (the
/// position in the whole file).
#[derive(Debug, Clone)]
pub(super) struct Cursor<'a> {
    data: &'a [u8],
    pos: usize,
    base: usize,
}

impl<'a> Cursor<'a> {
    pub(super) fn new(data: &'a [u8], base: usize) -> Cursor<'a> {
        Cursor { data, pos: 0, base }
    }

    /// Offset in the whole file.
    pub(super) fn offset(&self) -> usize {
        self.base.saturating_add(self.pos)
    }

    pub(super) fn remaining(&self) -> usize {
        self.data.len() - self.pos
    }

    pub(super) fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    /// A malformed-file error located at the current offset.
    pub(super) fn error(&self, what: impl std::fmt::Display) -> ProfileError {
        ProfileError::malformed(FORMAT, format!("{what} at byte {}", self.offset()))
    }

    pub(super) fn take(&mut self, n: usize) -> Result<&'a [u8]> {
        if n > self.remaining() {
            return Err(self.error(format_args!(
                "the file ends unexpectedly (needed {n} bytes, {} left; is it truncated?)",
                self.remaining()
            )));
        }
        let s = &self.data[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    /// Everything left.
    pub(super) fn rest(&mut self) -> &'a [u8] {
        let s = &self.data[self.pos..];
        self.pos = self.data.len();
        s
    }

    /// A cursor over the next `n` bytes (offsets stay file offsets).
    pub(super) fn sub(&mut self, n: usize) -> Result<Cursor<'a>> {
        let base = self.offset();
        Ok(Cursor::new(self.take(n)?, base))
    }

    fn array<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut out = [0u8; N];
        out.copy_from_slice(self.take(N)?);
        Ok(out)
    }

    pub(super) fn u8(&mut self) -> Result<u8> {
        Ok(self.array::<1>()?[0])
    }

    pub(super) fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.array()?))
    }

    pub(super) fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.array()?))
    }

    pub(super) fn i32(&mut self) -> Result<i32> {
        Ok(i32::from_le_bytes(self.array()?))
    }

    pub(super) fn u64(&mut self) -> Result<u64> {
        Ok(u64::from_le_bytes(self.array()?))
    }

    pub(super) fn i64(&mut self) -> Result<i64> {
        Ok(i64::from_le_bytes(self.array()?))
    }

    /// A non-negative 32-bit length or count.
    pub(super) fn len32(&mut self, what: &str) -> Result<usize> {
        let at = self.clone();
        let v = self.i32()?;
        usize::try_from(v).map_err(|_| at.error(format_args!("negative {what} {v}")))
    }

    /// An unsigned LEB128 value of at most `max_bytes` bytes; bits beyond 64 are dropped.
    fn var(&mut self, max_bytes: u32) -> Result<u64> {
        let mut v = 0u64;
        for i in 0..max_bytes {
            let b = self.u8()?;
            v |= u64::from(b & 0x7f) << (7 * i);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(self.error("a variable-length number is too long"))
    }

    pub(super) fn var_u32(&mut self) -> Result<u32> {
        // Five bytes hold 35 bits; the writer never sets the extra ones.
        Ok(self.var(5)? as u32)
    }

    pub(super) fn var_u64(&mut self) -> Result<u64> {
        self.var(10)
    }

    /// A NUL-terminated UTF-16LE string; keeps at most `max_chars` characters
    /// but always consumes the whole string.
    pub(super) fn utf16z(&mut self, max_chars: usize) -> Result<String> {
        let mut units: Vec<u16> = Vec::new();
        loop {
            if self.remaining() < 2 {
                return Err(self.error("a text field has no terminating NUL"));
            }
            let u = self.u16()?;
            if u == 0 {
                break;
            }
            // Two units per character at most (surrogate pairs).
            if units.len() < max_chars.saturating_mul(2) {
                units.push(u);
            }
        }
        let s = String::from_utf16_lossy(&units);
        Ok(debrief_profile::limits::clip(&s, max_chars))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn primitives_and_bounds() {
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19];
        let mut c = Cursor::new(&data, 100);
        assert_eq!(c.u8().unwrap(), 1);
        assert_eq!(c.u16().unwrap(), 0x0302);
        assert_eq!(c.u32().unwrap(), 0x0706_0504);
        assert_eq!(c.u64().unwrap(), 0x0f0e_0d0c_0b0a_0908);
        assert_eq!(c.offset(), 115);
        let e = c.u64().unwrap_err().to_string();
        assert!(e.contains("ends unexpectedly") && e.contains("at byte 115"), "{e}");
        assert_eq!(c.remaining(), 4, "a failed read consumes nothing");
        assert_eq!(c.i32().unwrap(), 0x1312_1110);
        assert!(c.is_empty());
        let mut n = Cursor::new(&[0xff, 0xff, 0xff, 0xff], 0);
        assert!(n.len32("count").unwrap_err().to_string().contains("negative count -1"));
        let mut s = Cursor::new(&[1, 0, 0, 0, 0, 0, 0, 0], 0);
        assert_eq!(s.i64().unwrap(), 1);
        let mut s = Cursor::new(&[1, 2, 3], 7);
        let mut sub = s.sub(2).unwrap();
        assert_eq!(sub.offset(), 7);
        assert_eq!(sub.take(2).unwrap(), &[1, 2]);
        assert!(s.sub(2).is_err());
    }

    #[test]
    fn varints() {
        let mut c = Cursor::new(&[0x00, 0x7f, 0x80, 0x01, 0xff, 0xff, 0xff, 0xff, 0x0f], 0);
        assert_eq!(c.var_u32().unwrap(), 0);
        assert_eq!(c.var_u32().unwrap(), 127);
        assert_eq!(c.var_u32().unwrap(), 128);
        assert_eq!(c.var_u32().unwrap(), u32::MAX);
        let mut c = Cursor::new(&[0xff; 9], 0);
        assert!(c.var_u32().unwrap_err().to_string().contains("too long"));
        let mut big = vec![0xffu8; 9];
        big.push(0x01);
        assert_eq!(Cursor::new(&big, 0).var_u64().unwrap(), u64::MAX);
        assert!(Cursor::new(&[0xff; 11], 0).var_u64().is_err());
        assert!(Cursor::new(&[0x80], 0).var_u64().is_err(), "truncated");
    }

    #[test]
    fn utf16_strings() {
        let enc = |s: &str| -> Vec<u8> { s.encode_utf16().chain([0]).flat_map(|u| u.to_le_bytes()).collect() };
        let a = enc("Hé😀x");
        let mut c = Cursor::new(&a, 0);
        assert_eq!(c.utf16z(10).unwrap(), "Hé😀x");
        let b = enc("abcdef");
        let mut c = Cursor::new(&b, 0);
        assert_eq!(c.utf16z(3).unwrap(), "abc");
        assert!(c.is_empty(), "the whole string is consumed");
        assert!(Cursor::new(&[0x41, 0], 0).utf16z(5).is_err(), "no NUL");
        assert!(Cursor::new(&[0x41, 0, 0], 0).utf16z(5).is_err(), "odd tail");
        // A lone surrogate is replaced, not an error.
        assert_eq!(Cursor::new(&[0x00, 0xd8, 0, 0], 0).utf16z(5).unwrap(), "\u{fffd}");
    }
}
