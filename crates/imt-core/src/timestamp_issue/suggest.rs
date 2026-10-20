//! Proposes a timestamp format for lines whose timestamps IMT could not read. It finds the first
//! date/time-shaped text of each line, describes its tokens as a strftime format (what
//! `[parse] timestamp_formats` takes) and keeps the format only when it really parses the samples.
//! Pure and panic-free on any input.

use crate::time::TimestampParser;

/// How far into a line a timestamp is looked for (bytes).
const SCAN_BYTES: usize = 256;
/// Start positions tried per line.
const MAX_STARTS: usize = 24;

/// A verified format and the text of each sample it reads.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Proposal {
    /// strftime format, e.g. `%a %b %d %H:%M:%S %Y`.
    pub format: String,
    /// The timestamp text found in each sample line the format reads.
    pub texts: Vec<String>,
}

/// A timestamp found in a line: its format, its text and, for a numeric day/month date, its first two numbers.
type Found<'a> = (String, &'a str, Option<(u32, u32)>);

/// The format that reads the timestamps of `lines`, or None when no single format reads at least half of
/// them, or when `parser` (the profile's) already reads them all (then the line pattern, not the
/// timestamp format, is what misses them).
pub fn propose(lines: &[&str], parser: &TimestampParser) -> Option<Proposal> {
    let found: Vec<Found> = lines.iter().filter_map(|l| find(l, None)).collect();
    // Day-first when any sample's first number cannot be a month, month-first when the second cannot be.
    let day_first = if found.iter().any(|(_, _, p)| p.is_some_and(|(a, _)| a > 12)) {
        Some(true)
    } else if found.iter().any(|(_, _, p)| p.is_some_and(|(_, b)| b > 12)) {
        Some(false)
    } else {
        None
    };
    let found: Vec<(String, &str)> = lines.iter().filter_map(|l| find(l, day_first)).map(|(f, t, _)| (f, t)).collect();
    // The most common format (the earliest on ties).
    let mut best: Option<(&str, usize)> = None;
    for (f, _) in &found {
        let n = found.iter().filter(|(g, _)| g == f).count();
        if best.is_none_or(|(_, m)| n > m) {
            best = Some((f, n));
        }
    }
    let (format, n) = best?;
    if n * 2 < lines.len() {
        return None;
    }
    let texts: Vec<&str> = found.iter().filter(|(f, _)| f == format).map(|(_, t)| *t).collect();
    let with = parser.with_format(format);
    if !texts.iter().all(|t| with.parse(t).is_some()) || texts.iter().all(|t| parser.parse(t).is_some()) {
        return None;
    }
    Some(Proposal {
        format: format.to_string(),
        texts: texts.into_iter().map(str::to_string).collect(),
    })
}

/// Whether at least half of `lines` (and at least one) contain timestamp-shaped text.
pub fn looks_timestamped(lines: &[&str]) -> bool {
    let n = lines.iter().filter(|l| find(l, None).is_some()).count();
    n > 0 && n * 2 >= lines.len()
}

/// The first timestamp-shaped text of `line`: its format, the text and, for a numeric day/month date,
/// its first two numbers.
fn find(line: &str, day_first: Option<bool>) -> Option<Found<'_>> {
    let bytes = line.as_bytes();
    let end = bytes.len().min(SCAN_BYTES);
    let mut starts = 0;
    for i in 0..end {
        let b = bytes[i];
        let boundary = i == 0 || !bytes[i - 1].is_ascii_alphanumeric();
        if !boundary || !b.is_ascii_alphanumeric() {
            continue;
        }
        starts += 1;
        if starts > MAX_STARTS {
            break;
        }
        if let Some(shape) = infer(&bytes[i..], day_first) {
            // Every byte the shape consumed is ASCII, so the slice ends on a character boundary.
            let text = line.get(i..i + shape.len)?;
            return Some((shape.format, text, shape.pair));
        }
    }
    None
}

#[derive(Debug, PartialEq, Eq)]
struct Shape {
    format: String,
    len: usize,
    pair: Option<(u32, u32)>,
}

struct Cursor<'a> {
    s: &'a [u8],
    i: usize,
}

impl<'a> Cursor<'a> {
    fn peek(&self) -> Option<u8> {
        self.s.get(self.i).copied()
    }

    fn eat(&mut self, b: u8) -> bool {
        if self.peek() == Some(b) {
            self.i += 1;
            true
        } else {
            false
        }
    }

    fn run(&mut self, f: impl Fn(u8) -> bool) -> &'a [u8] {
        let start = self.i;
        while self.peek().is_some_and(&f) {
            self.i += 1;
        }
        &self.s[start..self.i]
    }

    /// A run of 1–9 digits.
    fn number(&mut self) -> Option<(usize, u32)> {
        let save = self.i;
        let d = self.run(|b| b.is_ascii_digit());
        if d.is_empty() || d.len() > 9 {
            self.i = save;
            return None;
        }
        let v = d.iter().fold(0u32, |acc, b| acc * 10 + u32::from(b - b'0'));
        Some((d.len(), v))
    }

    fn word(&mut self) -> &'a [u8] {
        self.run(|b| b.is_ascii_alphabetic())
    }

    /// Spaces, appended to `f` as one space (chrono matches any run of whitespace with it).
    fn spaces(&mut self, f: &mut String) -> bool {
        let n = self.run(|b| b == b' ').len();
        if n > 0 {
            f.push(' ');
        }
        n > 0
    }
}

const MONTHS: [&str; 12] = [
    "january",
    "february",
    "march",
    "april",
    "may",
    "june",
    "july",
    "august",
    "september",
    "october",
    "november",
    "december",
];
const DAYS: [&str; 7] = ["monday", "tuesday", "wednesday", "thursday", "friday", "saturday", "sunday"];

/// `%b`/`%B` (or `%a`/`%A`) when `w` is a 3-letter abbreviation or the full name of one of `names`.
fn name_spec(w: &[u8], names: &[&str], short: &'static str, long: &'static str) -> Option<&'static str> {
    let w = std::str::from_utf8(w).ok()?.to_ascii_lowercase();
    if w.len() == 3 && names.iter().any(|n| n.starts_with(&w)) {
        return Some(short);
    }
    names.contains(&w.as_str()).then_some(long)
}

fn infer(s: &[u8], day_first: Option<bool>) -> Option<Shape> {
    let mut c = Cursor { s, i: 0 };
    let mut f = String::new();
    let mut pair = None;
    // An optional weekday: "Sat ", "Saturday, ".
    let save = c.i;
    let w = c.word();
    match name_spec(w, &DAYS, "%a", "%A") {
        Some(spec) => {
            f.push_str(spec);
            if c.eat(b',') {
                f.push(',');
            }
            if !c.spaces(&mut f) {
                return None;
            }
        }
        None => c.i = save,
    }
    let mut year_after_time = false;
    if c.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
        // "Nov 08 2025", "Nov 8, 2025" or ctime's "Nov  8 00:04:37 2025".
        f.push_str(name_spec(c.word(), &MONTHS, "%b", "%B")?);
        if !c.spaces(&mut f) {
            return None;
        }
        c.number().filter(|(n, _)| *n <= 2)?;
        f.push_str("%d");
        let mark = (c.i, f.len());
        if c.eat(b',') {
            f.push(',');
        }
        c.spaces(&mut f);
        match c.number() {
            Some((4, _)) => f.push_str("%Y"),
            _ => {
                (c.i, _) = mark;
                f.truncate(mark.1);
                year_after_time = true;
            }
        }
    } else {
        let (n1, v1) = c.number()?;
        match n1 {
            8 => f.push_str("%Y%m%d"),
            4 => {
                let sep = c.peek().filter(|b| b"-/.".contains(b))? as char;
                c.i += 1;
                let (_, v2) = c.number().filter(|(n, _)| *n <= 2)?;
                if !c.eat(sep as u8) {
                    return None;
                }
                c.number().filter(|(n, _)| *n <= 2)?;
                f.push_str(&if v2 > 12 {
                    format!("%Y{sep}%d{sep}%m")
                } else {
                    format!("%Y{sep}%m{sep}%d")
                });
            }
            1 | 2 => {
                let sep = c.peek().filter(|b| b"-/. ".contains(b))? as char;
                c.i += 1;
                if c.peek().is_some_and(|b| b.is_ascii_alphabetic()) {
                    // "08 Nov 2025", "8-Nov-2025".
                    let month = name_spec(c.word(), &MONTHS, "%b", "%B")?;
                    let sep2 = c.peek().filter(|b| b"-/ ".contains(b))? as char;
                    c.i += 1;
                    let (n3, _) = c.number()?;
                    let year = match n3 {
                        4 => "%Y",
                        2 => "%y",
                        _ => return None,
                    };
                    f.push_str(&format!("%d{sep}{month}{sep2}{year}"));
                } else {
                    let (_, v2) = c.number().filter(|(n, _)| *n <= 2)?;
                    if !c.eat(sep as u8) {
                        return None;
                    }
                    let year = match c.number()? {
                        (4, _) => "%Y",
                        (2, _) => "%y",
                        _ => return None,
                    };
                    pair = Some((v1, v2));
                    let (a, b) = if day_first.unwrap_or(false) { ("%d", "%m") } else { ("%m", "%d") };
                    f.push_str(&format!("{a}{sep}{b}{sep}{year}"));
                }
            }
            _ => return None,
        }
    }

    // The time, after "T", ",", "_", "-", ":" or spaces (none: a date alone).
    let date_end = (c.i, f.len());
    let mut sep_ok = true;
    match c.peek() {
        Some(b @ (b'T' | b',' | b'_' | b'-' | b':')) => {
            c.i += 1;
            f.push(b as char);
            if b == b',' {
                c.spaces(&mut f);
            }
        }
        Some(b' ') => {
            c.spaces(&mut f);
        }
        _ => sep_ok = false,
    }
    if !sep_ok || !time(&mut c, &mut f) {
        if year_after_time {
            return None;
        }
        (c.i, _) = date_end;
        f.truncate(date_end.1);
    } else if year_after_time {
        if !c.spaces(&mut f) || !matches!(c.number(), Some((4, _))) {
            return None;
        }
        f.push_str("%Y");
    }
    // The text must end here: not in the middle of a word or number.
    if c.peek().is_some_and(|b| b.is_ascii_alphanumeric()) {
        return None;
    }
    Some(Shape { format: f, len: c.i, pair })
}

/// `H:MM[:SS][.fff|,fff][ AM][zone]` appended to `f`; false (cursor unspecified) when it is not a time.
fn time(c: &mut Cursor, f: &mut String) -> bool {
    let Some((n, _)) = c.number() else { return false };
    if n > 2 || !c.eat(b':') || !matches!(c.number(), Some((2, _))) {
        return false;
    }
    let hour_at = f.len();
    f.push_str("%H:%M");
    if c.eat(b':') {
        if !matches!(c.number(), Some((2, _))) {
            return false;
        }
        f.push_str(":%S");
        let mark = c.i;
        if c.eat(b'.') {
            match c.number() {
                Some(_) => f.push_str("%.f"),
                None => c.i = mark,
            }
        } else if c.eat(b',') {
            match c.number() {
                Some((3, _)) => f.push_str(",%3f"),
                Some((6, _)) => f.push_str(",%6f"),
                Some((9, _)) => f.push_str(",%9f"),
                _ => c.i = mark,
            }
        }
    }
    // AM/PM.
    let mark = (c.i, f.len());
    c.spaces(f);
    let w = c.word();
    if w.eq_ignore_ascii_case(b"am") || w.eq_ignore_ascii_case(b"pm") {
        f.replace_range(hour_at..hour_at + 2, "%I");
        f.push_str("%p");
    } else {
        (c.i, _) = mark;
        f.truncate(mark.1);
    }
    // A zone: Z, +01, +0100 or +01:00, maybe after one space.
    let mark = (c.i, f.len());
    c.spaces(f);
    match c.peek() {
        Some(b'Z') => {
            c.i += 1;
            f.push_str("%#z");
        }
        Some(b'+' | b'-') => {
            c.i += 1;
            let zone = match c.number() {
                Some((2, _)) if c.eat(b':') => matches!(c.number(), Some((2, _))).then_some("%:z"),
                Some((2, _)) => Some("%#z"),
                Some((4, _)) => Some("%z"),
                _ => None,
            };
            match zone {
                Some(z) => f.push_str(z),
                None => {
                    (c.i, _) = mark;
                    f.truncate(mark.1);
                }
            }
        }
        _ => {
            (c.i, _) = mark;
            f.truncate(mark.1);
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shape(s: &str) -> Option<(String, &str)> {
        find(s, None).map(|(f, t, _)| (f, t))
    }

    #[test]
    fn describes_common_shapes() {
        let cases = [
            (
                "2025/11/08,00:04:37.759+01:00 INFO x",
                "%Y/%m/%d,%H:%M:%S%.f%:z",
                "2025/11/08,00:04:37.759+01:00",
            ),
            (
                "2025-11-08,00:04:37.759+0100 x",
                "%Y-%m-%d,%H:%M:%S%.f%z",
                "2025-11-08,00:04:37.759+0100",
            ),
            ("2025/11/08,00:04:37.759Z", "%Y/%m/%d,%H:%M:%S%.f%#z", "2025/11/08,00:04:37.759Z"),
            ("Sat Nov 08 00:04:37 2025 boot", "%a %b %d %H:%M:%S %Y", "Sat Nov 08 00:04:37 2025"),
            ("Sat Nov  8 00:04:37 2025", "%a %b %d %H:%M:%S %Y", "Sat Nov  8 00:04:37 2025"),
            (
                "Saturday, November 8, 2025 12:04 AM",
                "%A, %B %d, %Y %I:%M %p",
                "Saturday, November 8, 2025 12:04 AM",
            ),
            ("08-Nov-25 00:04:37,759 x", "%d-%b-%y %H:%M:%S,%3f", "08-Nov-25 00:04:37,759"),
            ("08 Nov 2025 00:04:37 +01", "%d %b %Y %H:%M:%S %#z", "08 Nov 2025 00:04:37 +01"),
            ("11.08.2025_00:04:37 x", "%m.%d.%Y_%H:%M:%S", "11.08.2025_00:04:37"),
            ("20251108-00:04:37.123456 x", "%Y%m%d-%H:%M:%S%.f", "20251108-00:04:37.123456"),
            ("{\"t\":\"2025.08.11 00:04\"}", "%Y.%m.%d %H:%M", "2025.08.11 00:04"),
            ("2025/31/08 x", "%Y/%d/%m", "2025/31/08"),
            ("Nov 8 2025", "%b %d %Y", "Nov 8 2025"),
            ("id=7 at 2025-11-08 00:04", "%Y-%m-%d %H:%M", "2025-11-08 00:04"),
            // A broken time or zone leaves the date (and the time) alone.
            ("2025-11-08 00:4 x", "%Y-%m-%d", "2025-11-08"),
            ("2025-11-08 00:04:37,75 x", "%Y-%m-%d %H:%M:%S", "2025-11-08 00:04:37"),
            ("2025-11-08 00:04:37.x", "%Y-%m-%d %H:%M:%S", "2025-11-08 00:04:37"),
            ("2025-11-08 00:04:37 +1 x", "%Y-%m-%d %H:%M:%S", "2025-11-08 00:04:37"),
            ("2025-11-08 00:04:37 +01:0 x", "%Y-%m-%d %H:%M:%S", "2025-11-08 00:04:37"),
        ];
        for (line, format, text) in cases {
            assert_eq!(shape(line), Some((format.to_string(), text)), "{line}");
        }
        assert_eq!(
            find("08/11/2025 x", Some(true)).map(|(f, _, p)| (f, p)),
            Some(("%d/%m/%Y".into(), Some((8, 11))))
        );
    }

    #[test]
    fn rejects_what_is_not_a_timestamp() {
        for line in [
            "",
            "hello world",
            "version 1.2.3",
            "Nov",
            "Nov x",
            "Nov 123",
            "Sat",
            "Satx Nov 08",
            "Sat,Nov 08",
            "2025",
            "2025-11",
            "2025-11/08",
            "2025-1108",
            "08/11",
            "08/11-2025",
            "08/11/202",
            "08 Foo 2025",
            "08 Nov",
            "08 Nov2025",
            "08 Nov 202",
            "123/4/5",
            "1234567890123 x",
            "2025-11-08x",
            "Sat Nov 08 00:04:37",
            "Sat Nov 08 00:04:37 25",
            "Sat Nov 08 x",
            "ééé 2025-11-08",
        ] {
            assert!(shape(line).is_none() || line.ends_with("2025-11-08"), "{line}");
        }
        // Non-ASCII before the date is skipped, not split.
        assert_eq!(shape("ééé 2025-11-08").map(|(_, t)| t), Some("2025-11-08"));
        // Only the start of a long line is scanned.
        let long = format!("{} 2025-11-08", "x ".repeat(200));
        assert_eq!(shape(&long), None);
    }

    #[test]
    fn proposes_a_verified_format() {
        let parser = TimestampParser::default();
        let lines = [
            "Sat Nov 08 00:04:37 2025,INFO,started",
            "Sat Nov 08 00:04:38 2025,WARN,slow",
            "Sun Nov 09 13:00:00 2025,INFO,done",
        ];
        let p = propose(&lines, &parser).unwrap();
        assert_eq!(p.format, "%a %b %d %H:%M:%S %Y");
        assert_eq!(p.texts.len(), 3);
        // Day-first is decided across the samples.
        let p = propose(&["01.02.2025_10:00:00 a", "25.02.2025_10:00:00 b"], &parser).unwrap();
        assert_eq!(p.format, "%d.%m.%Y_%H:%M:%S");
        let p = propose(&["02.01.2025_10:00:00 a", "02.25.2025_10:00:00 b"], &parser).unwrap();
        assert_eq!(p.format, "%m.%d.%Y_%H:%M:%S");
    }

    #[test]
    fn no_proposal_without_evidence() {
        let parser = TimestampParser::default();
        // Nothing timestamp-like, or most lines differ.
        assert_eq!(propose(&["no time here", "nor here"], &parser), None);
        assert_eq!(propose(&[], &parser), None);
        assert_eq!(
            propose(&["Sat Nov 08 00:04:37 2025", "x", "y", "z"], &parser),
            None,
            "one sample of four is not enough"
        );
        assert!(looks_timestamped(&["x 2025-11-08", "y"]));
        assert!(!looks_timestamped(&["x 2025-11-08", "y", "z"]));
        assert!(!looks_timestamped(&[]));
        // The profile already reads these timestamps: the line pattern is what misses them.
        assert_eq!(propose(&["2025-11-08 00:04:37 x", "2025-11-08 00:04:38 y"], &parser), None);
        // A shape whose values are not a real date does not verify.
        assert_eq!(propose(&["2025/13/45_00:04:37 a"], &parser), None);
    }
}
