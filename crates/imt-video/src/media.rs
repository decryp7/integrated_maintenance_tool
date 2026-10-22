//! What serving a video to the player needs, as pure functions (ADR 0033 §7): which `Range` of the file a request
//! asks for, bounded; and the content type of a video file. The shell's `media:` scheme handler does the I/O.

/// Most bytes sent in one response (the player asks again for the next range).
pub const MAX_CHUNK: u64 = 8 << 20;

/// What a `Range` header asks for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RangeAnswer {
    /// No (usable) range: the whole file, as a first chunk (`206`, since a full body could be gigabytes).
    Whole { end: u64 },
    /// Bytes `start..=end` of a file of `len` bytes.
    Part { start: u64, end: u64 },
    /// The range lies outside the file (`416`).
    Unsatisfiable,
}

/// Decides the answer to a `Range: bytes=...` header for a file of `len` bytes. One range only (players ask for one);
/// anything else is answered with the start of the file. The answer never covers more than [`MAX_CHUNK`] bytes.
pub fn answer(header: Option<&str>, len: u64) -> RangeAnswer {
    if len == 0 {
        return RangeAnswer::Unsatisfiable;
    }
    let last = len - 1;
    let cap = |start: u64, end: u64| start.saturating_add(MAX_CHUNK - 1).min(end);
    let Some(spec) = header.and_then(|h| h.trim().strip_prefix("bytes=")) else {
        return RangeAnswer::Whole { end: cap(0, last) };
    };
    if spec.contains(',') {
        return RangeAnswer::Whole { end: cap(0, last) };
    }
    let Some((a, b)) = spec.split_once('-') else {
        return RangeAnswer::Whole { end: cap(0, last) };
    };
    let (a, b) = (a.trim(), b.trim());
    match (a.parse::<u64>(), b.parse::<u64>()) {
        (Ok(start), Ok(end)) if start <= end => {
            if start > last {
                RangeAnswer::Unsatisfiable
            } else {
                RangeAnswer::Part {
                    start,
                    end: cap(start, end.min(last)),
                }
            }
        }
        (Ok(start), Err(_)) if b.is_empty() => {
            if start > last {
                RangeAnswer::Unsatisfiable
            } else {
                RangeAnswer::Part {
                    start,
                    end: cap(start, last),
                }
            }
        }
        // `bytes=-500`: the last 500 bytes.
        (Err(_), Ok(n)) if a.is_empty() && n > 0 => {
            let start = len.saturating_sub(n);
            RangeAnswer::Part {
                start,
                end: cap(start, last),
            }
        }
        _ => RangeAnswer::Whole { end: cap(0, last) },
    }
}

/// The content type of a video file by its extension; `None` for anything the player is not offered.
pub fn content_type(name: &str) -> Option<&'static str> {
    let ext = name.rsplit_once('.')?.1.to_ascii_lowercase();
    Some(match ext.as_str() {
        "mp4" | "m4v" => "video/mp4",
        "mov" => "video/quicktime",
        "webm" => "video/webm",
        "mkv" => "video/x-matroska",
        "avi" => "video/x-msvideo",
        _ => return None,
    })
}

/// The extensions of the files IMT offers as videos.
pub const EXTENSIONS: [&str; 6] = ["mp4", "m4v", "mov", "mkv", "webm", "avi"];

/// Whether the first bytes look like a video container IMT opens (MP4/MOV `ftyp`/`moov`/`mdat`, Matroska/WebM EBML, AVI RIFF).
pub fn sniff(head: &[u8]) -> bool {
    let tag = |at: usize, t: &[u8]| head.get(at..at + t.len()) == Some(t);
    tag(4, b"ftyp")
        || tag(4, b"moov")
        || tag(4, b"mdat")
        || tag(4, b"free")
        || tag(0, &[0x1A, 0x45, 0xDF, 0xA3])
        || (tag(0, b"RIFF") && tag(8, b"AVI "))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ranges_are_bounded_and_odd_ones_get_the_start() {
        let len = 100 << 20;
        assert_eq!(answer(Some("bytes=10-19"), 1000), RangeAnswer::Part { start: 10, end: 19 });
        assert_eq!(answer(Some("bytes=990-"), 1000), RangeAnswer::Part { start: 990, end: 999 });
        assert_eq!(answer(Some("bytes=-100"), 1000), RangeAnswer::Part { start: 900, end: 999 });
        assert_eq!(answer(Some("bytes=-5000"), 1000), RangeAnswer::Part { start: 0, end: 999 });
        assert_eq!(answer(Some("bytes=900-5000"), 1000), RangeAnswer::Part { start: 900, end: 999 });
        assert_eq!(
            answer(Some("bytes=0-"), len),
            RangeAnswer::Part {
                start: 0,
                end: MAX_CHUNK - 1
            }
        );
        assert_eq!(
            answer(Some("bytes=5-"), len),
            RangeAnswer::Part {
                start: 5,
                end: 5 + MAX_CHUNK - 1
            }
        );
        assert_eq!(answer(None, 1000), RangeAnswer::Whole { end: 999 });
        assert_eq!(answer(None, len), RangeAnswer::Whole { end: MAX_CHUNK - 1 });
        for odd in [
            "items=0-1",
            "bytes=0-1,5-6",
            "bytes=abc",
            "bytes=9-3",
            "bytes=-0",
            "bytes=-",
            "bytes=a-b",
            "",
        ] {
            assert_eq!(answer(Some(odd), 1000), RangeAnswer::Whole { end: 999 }, "{odd}");
        }
        assert_eq!(answer(Some("bytes=1000-"), 1000), RangeAnswer::Unsatisfiable);
        assert_eq!(answer(Some("bytes=2000-2100"), 1000), RangeAnswer::Unsatisfiable);
        assert_eq!(answer(Some("bytes=0-1"), 0), RangeAnswer::Unsatisfiable);
        assert_eq!(answer(Some(&format!("bytes={}-", u64::MAX)), 1000), RangeAnswer::Unsatisfiable);
    }

    #[test]
    fn only_video_types_are_served() {
        assert_eq!(content_type("Rec.MP4"), Some("video/mp4"));
        assert_eq!(content_type("a.mkv"), Some("video/x-matroska"));
        for ext in EXTENSIONS {
            assert!(content_type(&format!("x.{ext}")).is_some(), "{ext}");
        }
        assert_eq!(content_type("notes.txt"), None);
        assert_eq!(content_type("noext"), None);
    }

    #[test]
    fn containers_are_recognised_by_their_first_bytes() {
        assert!(sniff(b"\0\0\0\x18ftypmp42"));
        assert!(sniff(&[0x1A, 0x45, 0xDF, 0xA3, 1, 2]));
        assert!(sniff(b"RIFF\0\0\0\0AVI LIST"));
        assert!(!sniff(b"2026-10-14 10:41:14 INFO started"));
        assert!(!sniff(b"RIFF\0\0\0\0WAVEfmt "));
        assert!(!sniff(b""));
    }
}
