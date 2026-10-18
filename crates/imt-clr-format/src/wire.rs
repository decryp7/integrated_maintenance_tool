//! Primitive encoding: LEB128 varints, zigzag, length-prefixed UTF-8 strings.
//!
//! [`Dec`] never panics on any input: every read is bounds-checked and
//! returns [`FormatError`].

use crate::FormatError;

/// Appends values to a byte buffer.
#[derive(Debug, Default, Clone)]
pub struct Enc {
    pub buf: Vec<u8>,
}

impl Enc {
    pub fn new() -> Enc {
        Enc::default()
    }

    pub fn with_capacity(n: usize) -> Enc {
        Enc {
            buf: Vec::with_capacity(n),
        }
    }

    pub fn u8(&mut self, v: u8) -> &mut Self {
        self.buf.push(v);
        self
    }

    /// Unsigned LEB128.
    pub fn u(&mut self, mut v: u64) -> &mut Self {
        loop {
            let byte = (v & 0x7f) as u8;
            v >>= 7;
            if v == 0 {
                self.buf.push(byte);
                return self;
            }
            self.buf.push(byte | 0x80);
        }
    }

    /// Signed (zigzag) LEB128.
    pub fn i(&mut self, v: i64) -> &mut Self {
        self.u(((v << 1) ^ (v >> 63)) as u64)
    }

    pub fn bool(&mut self, v: bool) -> &mut Self {
        self.u8(u8::from(v))
    }

    /// Length-prefixed UTF-8, clipped to `max` bytes on a char boundary.
    pub fn str_max(&mut self, s: &str, max: usize) -> &mut Self {
        let mut end = s.len().min(max);
        while !s.is_char_boundary(end) {
            end -= 1;
        }
        self.u(end as u64);
        self.buf.extend_from_slice(&s.as_bytes()[..end]);
        self
    }

    pub fn str(&mut self, s: &str) -> &mut Self {
        self.str_max(s, crate::MAX_STRING)
    }

    pub fn raw(&mut self, bytes: &[u8]) -> &mut Self {
        self.buf.extend_from_slice(bytes);
        self
    }

    pub fn u32_le(&mut self, v: u32) -> &mut Self {
        self.buf.extend_from_slice(&v.to_le_bytes());
        self
    }
}

/// Reads values from a byte slice.
#[derive(Debug, Clone)]
pub struct Dec<'a> {
    bytes: &'a [u8],
    pos: usize,
}

impl<'a> Dec<'a> {
    pub fn new(bytes: &'a [u8]) -> Dec<'a> {
        Dec { bytes, pos: 0 }
    }

    pub fn pos(&self) -> usize {
        self.pos
    }

    pub fn remaining(&self) -> usize {
        self.bytes.len() - self.pos
    }

    pub fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub fn u8(&mut self) -> Result<u8, FormatError> {
        let b = *self.bytes.get(self.pos).ok_or(FormatError::Truncated)?;
        self.pos += 1;
        Ok(b)
    }

    pub fn u(&mut self) -> Result<u64, FormatError> {
        let mut v: u64 = 0;
        for shift in (0..64).step_by(7) {
            let b = self.u8()?;
            let bits = u64::from(b & 0x7f);
            if shift == 63 && bits > 1 {
                return Err(FormatError::Invalid("varint overflows 64 bits"));
            }
            v |= bits << shift;
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(FormatError::Invalid("varint is longer than 10 bytes"))
    }

    pub fn u32(&mut self) -> Result<u32, FormatError> {
        u32::try_from(self.u()?).map_err(|_| FormatError::Invalid("value does not fit in 32 bits"))
    }

    pub fn i(&mut self) -> Result<i64, FormatError> {
        let v = self.u()?;
        Ok(((v >> 1) as i64) ^ -((v & 1) as i64))
    }

    pub fn bool(&mut self) -> Result<bool, FormatError> {
        match self.u8()? {
            0 => Ok(false),
            1 => Ok(true),
            _ => Err(FormatError::Invalid("boolean is not 0 or 1")),
        }
    }

    pub fn take(&mut self, n: usize) -> Result<&'a [u8], FormatError> {
        if n > self.remaining() {
            return Err(FormatError::Truncated);
        }
        let s = &self.bytes[self.pos..self.pos + n];
        self.pos += n;
        Ok(s)
    }

    pub fn str(&mut self) -> Result<String, FormatError> {
        let n = self.u()?;
        if n > crate::MAX_STRING as u64 {
            return Err(FormatError::Invalid("string is too long"));
        }
        let bytes = self.take(n as usize)?;
        Ok(String::from_utf8_lossy(bytes).into_owned())
    }

    pub fn u32_le(&mut self) -> Result<u32, FormatError> {
        let b = self.take(4)?;
        Ok(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
    }

    /// A count of following items, each at least `min_item_bytes` long, so a
    /// corrupt count cannot make the caller reserve more than the input holds.
    pub fn count(&mut self, min_item_bytes: usize) -> Result<usize, FormatError> {
        let n = self.u()?;
        let max = (self.remaining() / min_item_bytes.max(1)) as u64;
        if n > max {
            return Err(FormatError::Invalid("item count exceeds the remaining input"));
        }
        Ok(n as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints_round_trip_at_the_edges() {
        for v in [0u64, 1, 127, 128, 300, u32::MAX as u64, u64::MAX] {
            let mut e = Enc::new();
            e.u(v);
            assert_eq!(Dec::new(&e.buf).u().unwrap(), v);
        }
        for v in [0i64, -1, 1, i64::MIN, i64::MAX] {
            let mut e = Enc::new();
            e.i(v);
            assert_eq!(Dec::new(&e.buf).i().unwrap(), v);
        }
    }

    #[test]
    fn malformed_input_is_an_error() {
        assert_eq!(Dec::new(&[0x80]).u(), Err(FormatError::Truncated));
        assert!(Dec::new(&[0xff; 11]).u().is_err());
        assert!(Dec::new(&[0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x7f]).u().is_err());
        assert!(Dec::new(&[2]).bool().is_err());
        assert_eq!(Dec::new(&[5, b'a']).str(), Err(FormatError::Truncated));
        assert!(Dec::new(&[0x80, 0x80, 0x80, 0x80, 0x10]).u32().is_err());
        assert!(Dec::new(&[200, 1]).count(1).is_err());
    }

    #[test]
    fn strings_are_clipped_on_char_boundaries() {
        let mut e = Enc::new();
        e.str_max("aé", 2);
        assert_eq!(Dec::new(&e.buf).str().unwrap(), "a");
        let mut e = Enc::new();
        e.u32_le(7).bool(true);
        let mut d = Dec::new(&e.buf);
        assert_eq!(d.u32_le().unwrap(), 7);
        assert!(d.bool().unwrap());
        assert!(d.is_empty());
        assert_eq!(d.pos(), 5);
        assert_eq!(Enc::with_capacity(4).raw(b"xy").buf, b"xy");
    }
}
