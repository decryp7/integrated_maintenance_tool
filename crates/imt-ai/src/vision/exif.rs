//! The one EXIF fact IMT uses: when a photo or screenshot was taken (`DateTimeOriginal`, else `DateTime`, with
//! `OffsetTimeOriginal` when present). A bounded reader of the TIFF structure inside EXIF: it follows at most the
//! first IFD and its EXIF sub-IFD, reads only ASCII values, never indexes out of bounds and never loops.

/// Most entries read from one IFD (real files have a few dozen).
const MAX_ENTRIES: usize = 512;

const TAG_DATETIME: u16 = 0x0132;
const TAG_EXIF_IFD: u16 = 0x8769;
const TAG_DATETIME_ORIGINAL: u16 = 0x9003;
const TAG_OFFSET_TIME_ORIGINAL: u16 = 0x9011;
const TYPE_ASCII: u16 = 2;
const TYPE_LONG: u16 = 4;

/// When the image was taken, as the camera or tool wrote it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExifTime {
    /// `YYYY-MM-DD HH:MM:SS` (local time of the device; EXIF has no zone unless `offset` is set).
    pub local: String,
    /// The device's UTC offset, `+HH:MM`, when the file says.
    pub offset: Option<String>,
}

struct Tiff<'a> {
    data: &'a [u8],
    little: bool,
}

impl Tiff<'_> {
    fn u16(&self, at: usize) -> Option<u16> {
        let b = self.data.get(at..at.checked_add(2)?)?;
        Some(if self.little {
            u16::from_le_bytes([b[0], b[1]])
        } else {
            u16::from_be_bytes([b[0], b[1]])
        })
    }

    fn u32(&self, at: usize) -> Option<u32> {
        let b = self.data.get(at..at.checked_add(4)?)?;
        Some(if self.little {
            u32::from_le_bytes([b[0], b[1], b[2], b[3]])
        } else {
            u32::from_be_bytes([b[0], b[1], b[2], b[3]])
        })
    }

    /// The entries of the IFD at `at`: (tag, type, count, value-or-offset position).
    fn entries(&self, at: usize) -> Vec<(u16, u16, u32, usize)> {
        let Some(n) = self.u16(at) else { return vec![] };
        (0..usize::from(n).min(MAX_ENTRIES))
            .filter_map(|i| {
                let e = at.checked_add(2)?.checked_add(i.checked_mul(12)?)?;
                Some((self.u16(e)?, self.u16(e + 2)?, self.u32(e + 4)?, e + 8))
            })
            .collect()
    }

    fn ascii(&self, kind: u16, count: u32, value_at: usize) -> Option<String> {
        if kind != TYPE_ASCII || count == 0 || count > 64 {
            return None;
        }
        let len = count as usize;
        let start = if len <= 4 { value_at } else { self.u32(value_at)? as usize };
        let bytes = self.data.get(start..start.checked_add(len)?)?;
        let text: String = bytes
            .iter()
            .take_while(|b| **b != 0)
            .map(|b| char::from(*b))
            .filter(|c| c.is_ascii_graphic() || *c == ' ')
            .collect();
        Some(text.trim().to_string()).filter(|t| !t.is_empty())
    }
}

/// `YYYY:MM:DD HH:MM:SS` → `YYYY-MM-DD HH:MM:SS`, or `None` for anything else (blank dates are common).
fn normalise(t: &str) -> Option<String> {
    let b = t.as_bytes();
    let digits = |r: std::ops::Range<usize>| b.get(r).is_some_and(|s| s.iter().all(u8::is_ascii_digit));
    let shape = b.len() >= 19
        && digits(0..4)
        && b[4] == b':'
        && digits(5..7)
        && b[7] == b':'
        && digits(8..10)
        && b[10] == b' '
        && digits(11..13)
        && b[13] == b':'
        && digits(14..16)
        && b[16] == b':'
        && digits(17..19);
    (shape && &t[..4] != "0000").then(|| format!("{}-{}-{} {}", &t[..4], &t[5..7], &t[8..10], &t[11..19]))
}

fn offset_ok(o: &str) -> bool {
    let b = o.as_bytes();
    b.len() == 6 && matches!(b[0], b'+' | b'-') && b[3] == b':' && [1, 2, 4, 5].iter().all(|i| b[*i].is_ascii_digit())
}

/// The capture time in raw EXIF data (a TIFF structure, optionally after the `Exif\0\0` header of JPEG APP1).
pub fn capture_time(raw: &[u8]) -> Option<ExifTime> {
    let data = raw.strip_prefix(b"Exif\0\0").unwrap_or(raw);
    let little = match data.get(..4)? {
        [b'I', b'I', 42, 0] => true,
        [b'M', b'M', 0, 42] => false,
        _ => return None,
    };
    let t = Tiff { data, little };
    let ifd0 = t.u32(4)? as usize;
    let mut datetime = None;
    let mut exif_ifd = None;
    for (tag, kind, count, at) in t.entries(ifd0) {
        match tag {
            TAG_DATETIME => datetime = t.ascii(kind, count, at),
            TAG_EXIF_IFD if kind == TYPE_LONG && count == 1 => exif_ifd = t.u32(at).map(|o| o as usize).filter(|o| *o != ifd0),
            _ => {}
        }
    }
    let (mut original, mut offset) = (None, None);
    if let Some(sub) = exif_ifd {
        for (tag, kind, count, at) in t.entries(sub) {
            match tag {
                TAG_DATETIME_ORIGINAL => original = t.ascii(kind, count, at),
                TAG_OFFSET_TIME_ORIGINAL => offset = t.ascii(kind, count, at).filter(|o| offset_ok(o)),
                _ => {}
            }
        }
    }
    let local = original
        .as_deref()
        .and_then(normalise)
        .or_else(|| datetime.as_deref().and_then(normalise))?;
    Some(ExifTime { local, offset })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// A little-endian TIFF with `DateTime` in IFD0 and an EXIF IFD holding `DateTimeOriginal` (and an offset).
    pub(crate) fn tiff(original: Option<&str>, datetime: Option<&str>, offset: Option<&str>) -> Vec<u8> {
        let mut ifd0: Vec<(u16, u16, u32, Vec<u8>)> = vec![];
        if let Some(d) = datetime {
            ifd0.push((TAG_DATETIME, TYPE_ASCII, 20, format!("{d}\0").into_bytes()));
        }
        let mut sub: Vec<(u16, u16, u32, Vec<u8>)> = vec![];
        if let Some(d) = original {
            sub.push((TAG_DATETIME_ORIGINAL, TYPE_ASCII, 20, format!("{d}\0").into_bytes()));
        }
        if let Some(o) = offset {
            sub.push((TAG_OFFSET_TIME_ORIGINAL, TYPE_ASCII, 7, format!("{o}\0").into_bytes()));
        }
        // Layout: header (8) | IFD0 | sub IFD | values.
        let ifd0_len = 2 + 12 * (ifd0.len() + 1) + 4;
        let sub_at = 8 + ifd0_len;
        let sub_len = 2 + 12 * sub.len() + 4;
        let mut values_at = sub_at + sub_len;
        let mut out = b"II*\0".to_vec();
        out.extend(8u32.to_le_bytes());
        let mut values: Vec<u8> = vec![];
        let mut write_ifd = |out: &mut Vec<u8>, entries: &[(u16, u16, u32, Vec<u8>)], extra: Option<u32>| {
            out.extend(((entries.len() + usize::from(extra.is_some())) as u16).to_le_bytes());
            for (tag, kind, count, v) in entries {
                out.extend(tag.to_le_bytes());
                out.extend(kind.to_le_bytes());
                out.extend(count.to_le_bytes());
                out.extend((values_at as u32).to_le_bytes());
                values.extend(v);
                values_at += v.len();
            }
            if let Some(p) = extra {
                out.extend(TAG_EXIF_IFD.to_le_bytes());
                out.extend(TYPE_LONG.to_le_bytes());
                out.extend(1u32.to_le_bytes());
                out.extend(p.to_le_bytes());
            }
            out.extend(0u32.to_le_bytes());
        };
        write_ifd(&mut out, &ifd0, Some(sub_at as u32));
        write_ifd(&mut out, &sub, None);
        out.extend(values);
        out
    }

    #[test]
    fn reads_the_capture_time() {
        let t = capture_time(&tiff(Some("2026:10:21 09:14:03"), Some("2026:10:22 00:00:00"), Some("+02:00"))).unwrap();
        assert_eq!(t.local, "2026-10-21 09:14:03");
        assert_eq!(t.offset.as_deref(), Some("+02:00"));
        // The JPEG header prefix, the fallback to DateTime, blank and malformed values.
        let mut with_header = b"Exif\0\0".to_vec();
        with_header.extend(tiff(None, Some("2026:01:02 03:04:05"), Some("02:00")));
        let t = capture_time(&with_header).unwrap();
        assert_eq!((t.local.as_str(), t.offset), ("2026-01-02 03:04:05", None));
        assert_eq!(capture_time(&tiff(Some("0000:00:00 00:00:00"), None, None)), None);
        assert_eq!(capture_time(&tiff(Some("yesterday"), None, None)), None);
        assert_eq!(capture_time(b"MM\0*\0\0\0\x08"), None);
        assert_eq!(capture_time(b"nope"), None);
        assert_eq!(capture_time(&[]), None);
    }

    #[test]
    fn hostile_structures_end_without_a_panic() {
        let good = tiff(Some("2026:10:21 09:14:03"), Some("2026:10:22 00:00:00"), None);
        for cut in 0..good.len() {
            let _ = capture_time(&good[..cut]);
        }
        for i in 0..good.len() {
            for v in [0u8, 0xff, 0x7f] {
                let mut b = good.clone();
                b[i] = v;
                let _ = capture_time(&b);
            }
        }
        // An EXIF pointer back to IFD0 is not followed.
        let mut b = b"II*\0".to_vec();
        b.extend(8u32.to_le_bytes());
        b.extend(1u16.to_le_bytes());
        b.extend(TAG_EXIF_IFD.to_le_bytes());
        b.extend(TYPE_LONG.to_le_bytes());
        b.extend(1u32.to_le_bytes());
        b.extend(8u32.to_le_bytes());
        b.extend(0u32.to_le_bytes());
        assert_eq!(capture_time(&b), None);
    }
}
