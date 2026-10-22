//! A cheap walk over the top level of a protobuf-encoded pprof profile.
//!
//! Used before decoding to refuse files whose tables exceed the limits
//! (prost would allocate a struct per entry first) and to recognise pprof
//! from a file's first bytes without decoding it.

/// Entries of the top-level repeated fields of a `Profile`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Counts {
    pub samples: u64,
    pub mappings: u64,
    pub locations: u64,
    pub functions: u64,
    pub strings: u64,
}

/// Reads a base-128 varint at `*pos`; `None` when truncated or longer than 10 bytes.
pub(crate) fn varint(b: &[u8], pos: &mut usize) -> Option<u64> {
    let mut v = 0u64;
    for shift in 0..10 {
        let byte = *b.get(*pos)?;
        *pos += 1;
        v |= u64::from(byte & 0x7f) << (shift * 7);
        if byte & 0x80 == 0 {
            return Some(v);
        }
    }
    None
}

/// Skips the value of a field with wire type `wire` at `*pos`.
fn skip(b: &[u8], pos: &mut usize, wire: u64) -> Result<(), String> {
    match wire {
        0 => varint(b, pos).map(|_| ()).ok_or_else(|| "a number is cut off".to_string()),
        1 | 5 => {
            let n = if wire == 1 { 8 } else { 4 };
            if b.len() - *pos < n {
                return Err("a fixed-size value is cut off".to_string());
            }
            *pos += n;
            Ok(())
        }
        2 => {
            let len = varint(b, pos).ok_or("a length is cut off")?;
            let len = usize::try_from(len).map_err(|_| "a length is too large".to_string())?;
            if b.len() - *pos < len {
                return Err("a field is cut off (the file is truncated or not pprof)".to_string());
            }
            *pos += len;
            Ok(())
        }
        w => Err(format!("unsupported protobuf wire type {w}")),
    }
}

/// Wire type `profile.proto` declares for a top-level field (`None` for unknown fields).
fn expected_wire(field: u64) -> Option<&'static [u64]> {
    match field {
        1..=6 | 11 => Some(&[2]),
        7..=10 | 12 | 14 | 15 => Some(&[0]),
        13 => Some(&[0, 2]),
        _ => None,
    }
}

/// Walks every top-level field and counts the repeated tables.
pub(crate) fn scan(b: &[u8]) -> Result<Counts, String> {
    let mut c = Counts::default();
    let mut pos = 0;
    while pos < b.len() {
        let tag = varint(b, &mut pos).ok_or("a field tag is cut off")?;
        let (field, wire) = (tag >> 3, tag & 7);
        if field == 0 {
            return Err("field number 0 is invalid".to_string());
        }
        if expected_wire(field).is_some_and(|w| !w.contains(&wire)) {
            return Err(format!("field {field} has the wrong wire type {wire}"));
        }
        skip(b, &mut pos, wire)?;
        match field {
            2 => c.samples += 1,
            3 => c.mappings += 1,
            4 => c.locations += 1,
            5 => c.functions += 1,
            6 => c.strings += 1,
            _ => {}
        }
    }
    Ok(c)
}

/// Whether `head` (possibly cut anywhere) starts like a pprof profile: a
/// first `sample_type` (field 1) holding only small `type`/`unit` indexes,
/// followed by well-formed top-level fields of `profile.proto`.
pub(crate) fn looks_like_profile(head: &[u8]) -> bool {
    if head.first() != Some(&0x0a) {
        return false;
    }
    let mut pos = 1;
    let Some(len) = varint(head, &mut pos) else { return false };
    if !(2..=32).contains(&len) || head.len() - pos < len as usize {
        return false;
    }
    let vt = &head[pos..pos + len as usize];
    let mut p = 0;
    while p < vt.len() {
        match vt[p] {
            0x08 | 0x10 => p += 1,
            _ => return false,
        }
        if varint(vt, &mut p).is_none() {
            return false;
        }
    }
    pos += len as usize;
    // A few more top-level fields; the head may end in the middle of one.
    for _ in 0..16 {
        if pos >= head.len() {
            return true;
        }
        let Some(tag) = varint(head, &mut pos) else { return true };
        let (field, wire) = (tag >> 3, tag & 7);
        match expected_wire(field) {
            Some(w) if w.contains(&wire) => {}
            _ => return false,
        }
        match skip(head, &mut pos, wire) {
            Ok(()) => {}
            Err(_) => return true,
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn varints() {
        let mut p = 0;
        assert_eq!(varint(&[0x96, 0x01], &mut p), Some(150));
        assert_eq!(p, 2);
        let mut p = 0;
        assert_eq!(varint(&[0x80], &mut p), None, "cut off");
        let mut p = 0;
        assert_eq!(varint(&[0xff; 11], &mut p), None, "too long");
    }

    #[test]
    fn scan_counts_and_rejects() {
        // sample_type {1,2}, sample {}, sample {}, string "", string "a", unknown field 20 varint
        let b = [
            0x0a, 0x04, 0x08, 0x01, 0x10, 0x02, 0x12, 0x00, 0x12, 0x00, 0x32, 0x00, 0x32, 0x01, b'a', 0xa0, 0x01, 0x05,
        ];
        let c = scan(&b).unwrap();
        assert_eq!((c.samples, c.strings), (2, 2));
        assert!(scan(&[0x00]).unwrap_err().contains("field number 0"));
        assert!(scan(&[0x08, 0x01]).unwrap_err().contains("wrong wire type"));
        assert!(scan(&[0x0a, 0x05, 0x00]).unwrap_err().contains("cut off"));
        assert!(scan(&[0x0b]).unwrap_err().contains("wire type 3"));
        assert!(scan(&[0x80]).unwrap_err().contains("tag"));
        assert!(scan(&[0x48]).unwrap_err().contains("number"));
        assert!(scan(&[0xa1, 0x01, 0x00]).unwrap_err().contains("fixed-size"));
        assert!(scan(&[0xa5, 0x01, 0, 0, 0, 0]).is_ok(), "unknown fixed32 field");
        assert!(scan(&[0x0a]).unwrap_err().contains("length"));
        assert!(scan(&[0x0a, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x01]).is_err());
        assert_eq!(scan(&[]).unwrap(), Counts::default());
    }

    #[test]
    fn recognises_heads() {
        let good = [0x0a, 0x04, 0x08, 0x01, 0x10, 0x02, 0x12, 0x00, 0x32, 0x05, b'a'];
        assert!(looks_like_profile(&good), "cut in a string is fine");
        assert!(looks_like_profile(&good[..8]));
        assert!(looks_like_profile(&[0x0a, 0x04, 0x08, 0x01, 0x10, 0x02, 0x80]), "cut in a tag");
        assert!(!looks_like_profile(&good[..4]), "sample type cut");
        assert!(!looks_like_profile(b"\nhello world"));
        assert!(!looks_like_profile(b"{\"a\":1}"));
        assert!(!looks_like_profile(&[0x0a, 0x02, 0x18, 0x01]), "field 3 in ValueType");
        assert!(!looks_like_profile(&[0x0a, 0x02, 0x08, 0x80]), "varint cut inside ValueType");
        assert!(!looks_like_profile(&[0x0a, 0x80]));
        assert!(!looks_like_profile(&[0x0a, 0x02, 0x08, 0x01, 0x08]), "field 1 wire 0 at top level");
        assert!(!looks_like_profile(&[0x0a, 0x02, 0x08, 0x01, 0xf8, 0x01, 0x00]), "unknown field");
        let mut many = vec![0x0a, 0x02, 0x08, 0x01];
        for _ in 0..20 {
            many.extend([0x12, 0x00]);
        }
        assert!(looks_like_profile(&many));
    }
}
