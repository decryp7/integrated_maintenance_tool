//! UMDH logs into the engine's [`UmdhLog`]. Two outputs of Windows' User-Mode Dump Heap tool are read:
//!
//! - **Snapshots** (`umdh -p:<pid> -f:<file>`): an optional `// Loaded modules:` list (`base size path`), then per
//!   heap one line per allocation, `<size> bytes + <overhead> at <address> by BackTrace<id>`, with the stack the
//!   first time a back trace appears. Allocations are summed per back trace (count, bytes, largest).
//! - **Diffs** (`umdh <old> <new>`): per back trace `+ <delta> ( <new> - <old>) <count> allocs BackTrace<id>`, a
//!   count line `+ <delta> ( <new> - <old>) BackTrace<id> allocations`, then the stack; a closing
//!   `Total increase == <requested> requested + <overhead> overhead = <total>`.
//!
//! Numbers are hexadecimal unless UMDH was run with `-d`; the reader decides from the total line (whichever radix
//! makes it add up), else from the digits it sees, else hexadecimal. Frames are `module!function+offset` (with an
//! optional `(file, line)`), `module+offset`, or raw addresses, which are placed in the loaded modules when the
//! snapshot lists them. Files may be UTF-8, or UTF-16 with a byte-order mark (PowerShell redirection). Lines,
//! stacks, frames and text are bounded by [`Limits`]; anything unrecognised is skipped.

use crate::source::{read_exact, Source};
use debrief_dump::limits::clip;
use debrief_dump::model::*;
use debrief_dump::{DumpError, ErrorCode, Limits, Result};
use std::collections::HashMap;

/// Longest line looked at; longer ones are skipped.
const MAX_LINE: usize = 4096;

/// How sure we are that `head` (the start of a file) is a UMDH log: 0–100.
pub fn sniff(head: &[u8]) -> u8 {
    let text = decode(head);
    let backtrace = text.contains("BackTrace");
    if text.contains("Start of data for heap") || text.contains("REQUESTED bytes + OVERHEAD at ADDRESS by BackTraceID") {
        return 95;
    }
    if text.contains("// Each log entry has the following syntax") && text.contains("BYTES_DELTA") {
        return 95;
    }
    if backtrace && (text.contains(" allocs") || text.contains(" bytes + ")) {
        return 80;
    }
    if text.contains("UMDH") && backtrace {
        return 60;
    }
    0
}

/// UTF-8, or UTF-16 (LE/BE) with a byte-order mark; invalid sequences become U+FFFD.
pub fn decode(bytes: &[u8]) -> String {
    let utf16 = |be: bool| {
        let units: Vec<u16> = bytes[2..]
            .as_chunks::<2>()
            .0
            .iter()
            .map(|c| {
                if be {
                    u16::from_be_bytes([c[0], c[1]])
                } else {
                    u16::from_le_bytes([c[0], c[1]])
                }
            })
            .collect();
        String::from_utf16_lossy(&units)
    };
    match bytes {
        [0xFF, 0xFE, ..] => utf16(false),
        [0xFE, 0xFF, ..] => utf16(true),
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// Reads the UMDH log in `src`; `file_time_ms` is its time when the log records none.
pub fn read_umdh(src: &dyn Source, limits: &Limits, file_time_ms: Option<i64>) -> Result<UmdhLog> {
    if src.len() > limits.max_read_bytes {
        return Err(DumpError::limit(format!(
            "the UMDH log is {} MB; at most {} MB are read",
            src.len() >> 20,
            limits.max_read_bytes >> 20
        )));
    }
    let bytes = read_exact(src, 0, src.len(), "the log")?;
    parse(&decode(&bytes), limits, file_time_ms)
}

fn hex(s: &str) -> Option<u64> {
    let s = s.trim().trim_start_matches("0x").trim_start_matches("0X");
    if s.is_empty() || s.len() > 16 {
        return None;
    }
    u64::from_str_radix(s, 16).ok()
}

fn num(s: &str, radix: u32) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() || s.len() > 20 {
        return None;
    }
    u64::from_str_radix(s.trim_start_matches("0x"), radix).ok()
}

/// The back trace id after `BackTrace` in `s`.
fn backtrace_id(s: &str) -> Option<u64> {
    let i = s.find("BackTrace")?;
    let rest = &s[i + "BackTrace".len()..];
    let id: String = rest.chars().take_while(|c| c.is_ascii_hexdigit()).collect();
    hex(&id)
}

/// The module name UMDH uses for a path: the file name without extension.
fn module_name(path: &str) -> String {
    let file = debrief_dump::modules::file_name(path);
    match file.rsplit_once('.') {
        Some((stem, ext)) if ["dll", "exe", "sys", "drv", "ocx"].contains(&ext.to_ascii_lowercase().as_str()) => stem.to_string(),
        _ => file.to_string(),
    }
}

/// One stack frame line.
fn frame(line: &str, modules: &[UmdhModule], limits: &Limits) -> Option<UmdhFrame> {
    let t = line.trim();
    if t.is_empty() || t.starts_with("//") {
        return None;
    }
    let text = |s: &str| clip(s.trim(), limits.max_text);
    // `(file, line)` or `(file:line)` at the end.
    let (body, source) = match (t.rfind(" ("), t.ends_with(')')) {
        (Some(i), true) => (&t[..i], Some(&t[i + 2..t.len() - 1])),
        _ => (t, None),
    };
    let (file, line_no) = source.map_or((None, None), |s| {
        let (f, l) = s.rsplit_once(',').or_else(|| s.rsplit_once(':')).unwrap_or((s, ""));
        (Some(text(f)), l.trim().parse::<u32>().ok())
    });
    if let Some(address) = hex(body).filter(|_| body.chars().all(|c| c.is_ascii_hexdigit() || c == 'x' || c == 'X')) {
        // A raw address: placed in a loaded module when the log lists one.
        let m = modules.iter().find(|m| address >= m.base && address - m.base < m.size);
        return Some(UmdhFrame {
            module: m.map(|m| text(&m.name)),
            function: None,
            offset: m.map(|m| address - m.base),
            address: Some(address),
            file,
            line: line_no,
        });
    }
    let (module, rest) = match body.split_once('!') {
        Some((m, r)) => (Some(m), r),
        None => (None, body),
    };
    let (name, offset) = match rest.rsplit_once('+') {
        Some((n, o)) if hex(o).is_some() => (n, hex(o)),
        _ => (rest, None),
    };
    let name = name.trim();
    if name.is_empty() && module.is_none() {
        return None;
    }
    let (module, function) = match module {
        Some(m) => (Some(text(m)), (!name.is_empty() && name != "???").then(|| text(name))),
        // `module+offset` without symbols.
        None if offset.is_some() => (Some(text(name)), None),
        None => (None, Some(text(name))),
    };
    Some(UmdhFrame {
        module,
        function,
        offset,
        address: None,
        file,
        line: line_no,
    })
}

/// A diff entry's numbers: delta (signed by its leading `+`/`-`), new, old.
fn diff_numbers(line: &str, radix: u32) -> Option<(u64, u64)> {
    let open = line.find('(')?;
    let close = line[open..].find(')')? + open;
    let (new, old) = line[open + 1..close].split_once('-')?;
    Some((num(new, radix)?, num(old, radix)?))
}

/// Which radix makes `Total increase == a requested + b overhead = c` add up (UMDH's `-d` writes decimal).
fn radix_of(text: &str) -> u32 {
    for line in text.lines().take(10_000_000) {
        let Some(rest) = line.trim().strip_prefix("Total increase ==") else {
            continue;
        };
        let words: Vec<&str> = rest.split_whitespace().collect();
        if let [a, "requested", "+", b, "overhead", "=", c, ..] = words.as_slice() {
            for radix in [16, 10] {
                if let (Some(a), Some(b), Some(c)) = (num(a, radix), num(b, radix), num(c, radix)) {
                    if a.checked_add(b) == Some(c) {
                        return radix;
                    }
                }
            }
        }
    }
    16
}

/// Parses the text of a UMDH log.
pub fn parse(text: &str, limits: &Limits, file_time_ms: Option<i64>) -> Result<UmdhLog> {
    let diff = text.contains(" allocs") && text.contains("BackTrace") && !text.contains(" bytes + ");
    let radix = if diff { radix_of(text) } else { 16 };
    let mut modules: Vec<UmdhModule> = Vec::new();
    let mut in_modules = false;
    let mut stacks: Vec<UmdhStack> = Vec::new();
    let mut by_id: HashMap<u64, usize> = HashMap::new();
    let mut current: Option<usize> = None;
    let mut notes = Vec::new();
    let mut skipped_lines = 0usize;
    let mut dropped = 0usize;
    for raw in text.lines() {
        if raw.len() > MAX_LINE {
            skipped_lines += 1;
            continue;
        }
        let line = raw.trim_end();
        let t = line.trim();
        if t.starts_with("// Loaded modules") {
            in_modules = true;
            continue;
        }
        if in_modules {
            match t.strip_prefix("//") {
                Some(rest) => {
                    let words: Vec<&str> = rest.split_whitespace().collect();
                    if let [base, size, path @ ..] = words.as_slice() {
                        if let (Some(base), Some(size), false) = (hex(base), hex(size), path.is_empty()) {
                            if modules.len() < limits.max_modules {
                                modules.push(UmdhModule {
                                    base,
                                    size,
                                    name: clip(&module_name(&path.join(" ")), limits.max_text),
                                });
                            }
                        }
                    }
                    continue;
                }
                None => in_modules = false,
            }
        }
        // Blank lines end nothing: UMDH puts one between an entry and its stack.
        if t.is_empty() || t.starts_with("//") || t.starts_with("DBGHELP") || t.starts_with('*') {
            continue;
        }
        if let Some(id) = backtrace_id(t) {
            // An allocation (snapshot) or an entry (diff) line.
            let idx = match by_id.get(&id) {
                Some(i) => Some(*i),
                None if stacks.len() < limits.max_stacks => {
                    stacks.push(UmdhStack {
                        id,
                        bytes: 0,
                        count: 0,
                        old_bytes: diff.then_some(0),
                        old_count: diff.then_some(0),
                        largest: 0,
                        frames: Vec::new(),
                    });
                    by_id.insert(id, stacks.len() - 1);
                    Some(stacks.len() - 1)
                }
                None => {
                    dropped += 1;
                    None
                }
            };
            // Frames follow only the first time a back trace appears (an entry's lines all precede its stack).
            current = idx.filter(|i| stacks[*i].frames.is_empty());
            let Some(i) = idx else { continue };
            let s = &mut stacks[i];
            if diff {
                if let Some((new, old)) = diff_numbers(t, radix) {
                    if t.ends_with("allocations") {
                        s.count = new;
                        s.old_count = Some(old);
                    } else {
                        s.bytes = new;
                        s.old_bytes = Some(old);
                    }
                }
            } else {
                // `<size> bytes + <overhead> at <address> by BackTrace<id>`
                let words: Vec<&str> = t.split_whitespace().collect();
                if let [size, "bytes", ..] = words.as_slice() {
                    if let Some(size) = hex(size) {
                        s.bytes = s.bytes.saturating_add(size);
                        s.count = s.count.saturating_add(1);
                        s.largest = s.largest.max(size);
                    }
                }
            }
            continue;
        }
        if t.starts_with("REQUESTED bytes") || t.starts_with("STACK if") || t.starts_with("Total increase") {
            continue;
        }
        // A frame of the current stack (UMDH indents them).
        if let Some(i) = current {
            if raw.starts_with(['\t', ' ']) {
                if let Some(f) = frame(t, &modules, limits) {
                    let s = &mut stacks[i];
                    if s.frames.len() < limits.max_umdh_frames {
                        s.frames.push(f);
                    }
                }
            }
        }
    }
    if stacks.is_empty() {
        return Err(DumpError::new(
            ErrorCode::NotADump,
            "no allocation stacks found: this is not a UMDH log",
        ));
    }
    if dropped > 0 {
        notes.push(format!(
            "{dropped} allocation stacks beyond the first {} were left out.",
            limits.max_stacks
        ));
    }
    if skipped_lines > 0 {
        notes.push(format!("{skipped_lines} overlong lines were skipped."));
    }
    if diff && radix == 10 {
        notes.push("Numbers were read as decimal (umdh -d).".into());
    }
    Ok(UmdhLog {
        kind: if diff { UmdhKind::Diff } else { UmdhKind::Snapshot },
        process: None,
        modules,
        stacks,
        time_ms: file_time_ms,
        notes,
    })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const SNAPSHOT: &str = "\
// Debug library initialized ...
DBGHELP: OrderService - private symbols & lines
//
// Loaded modules:
//     Base Size Module
//            7FF600000000    20000 C:\\Shop\\OrderService.exe
//            7FFA00000000   1F0000 C:\\Windows\\System32\\ntdll.dll
//
// Process modules enumerated.

*- - - - - - - - - - Start of data for heap @ 0000025E3D2A0000 - - - - - - - - - -

REQUESTED bytes + OVERHEAD at ADDRESS by BackTraceID
     STACK if not already dumped.

*- - - - - - - - - - Heap 0000025E3D2A0000 Hogs - - - - - - - - - -

00000100 bytes + 00000010 at 0000025E3D2A1000 by BackTrace00A1
\tntdll!RtlpAllocateHeapInternal+0000000E
\tntdll!RtlAllocateHeap+00000021
\tOrderService!Cache::Add+0000001D (c:\\src\\cache.cpp, 42)
\t7FF600001100

00000300 bytes + 00000010 at 0000025E3D2A2000 by BackTrace00A1

00000040 bytes + 00000010 at 0000025E3D2A3000 by BackTrace00B2
\t7FFA00001234
\t12345678

*- - - - - - - - - - End of data for heap @ 0000025E3D2A0000 - - - - - - - - - -
";

    const DIFF: &str = "\
//
// Each log entry has the following syntax:
//
// + BYTES_DELTA (NEW_BYTES - OLD_BYTES) NEW_COUNT allocs BackTrace TRACEID
// + COUNT_DELTA (NEW_COUNT - OLD_COUNT) BackTrace TRACEID allocations
//     ... stack trace ...
//

+   80000 (  80000 -      0)      1 allocs\tBackTrace4920
+       1 (      1 -      0)\tBackTrace4920\tallocations

\tntdll!RtlAllocateHeap+00000021
\tShop.Orders!Session::New+34

-     100 (    100 -    200)      1 allocs\tBackTrace12
-       1 (      1 -      2)\tBackTrace12\tallocations

\tShop.Orders!Report::Build

Total increase ==  7FF00 requested +   1234 overhead =  81134
";

    #[test]
    fn snapshots_sum_allocations_per_back_trace() {
        assert!(sniff(SNAPSHOT.as_bytes()) >= 90);
        let u = parse(SNAPSHOT, &Limits::default(), Some(5)).unwrap();
        assert_eq!((u.kind, u.time_ms, u.modules.len()), (UmdhKind::Snapshot, Some(5), 2));
        assert_eq!(u.modules[1].name, "ntdll");
        assert_eq!(u.stacks.len(), 2);
        let a = &u.stacks[0];
        assert_eq!((a.id, a.bytes, a.count, a.largest), (0xA1, 0x400, 2, 0x300));
        assert_eq!(a.frames.len(), 4);
        assert_eq!(a.frames[2].label(), "OrderService!Cache::Add+0x1d");
        assert_eq!(
            (a.frames[2].file.as_deref(), a.frames[2].line),
            (Some("c:\\src\\cache.cpp"), Some(42))
        );
        assert_eq!(a.frames[3].label(), "OrderService+0x1100", "a raw address placed in its module");
        let b = &u.stacks[1];
        assert_eq!(b.frames[0].label(), "ntdll+0x1234");
        assert_eq!(b.frames[1].label(), "0x12345678", "outside every module");
        assert!(b.old_bytes.is_none());
    }

    #[test]
    fn diffs_read_both_sides_and_the_radix() {
        assert!(sniff(DIFF.as_bytes()) >= 90);
        let u = parse(DIFF, &Limits::default(), None).unwrap();
        assert_eq!(u.kind, UmdhKind::Diff);
        let a = &u.stacks[0];
        assert_eq!((a.bytes, a.old_bytes, a.count, a.old_count), (0x80000, Some(0), 1, Some(0)));
        assert_eq!(a.frames[1].label(), "Shop.Orders!Session::New+0x34");
        let b = &u.stacks[1];
        assert_eq!((b.bytes, b.old_bytes, b.count, b.old_count), (0x100, Some(0x200), 1, Some(2)));
        assert_eq!(b.frames[0].label(), "Shop.Orders!Report::Build");
        // The same log written with -d: the total adds up in decimal only.
        let dec = DIFF.replace(
            "7FF00 requested +   1234 overhead =  81134",
            "79900 requested +   1234 overhead =  81134",
        );
        let d = parse(&dec, &Limits::default(), None).unwrap();
        assert_eq!(d.stacks[0].bytes, 80000);
        assert!(d.notes.iter().any(|n| n.contains("decimal")));
    }

    #[test]
    fn encodings_bounds_and_garbage() {
        let mut utf16 = vec![0xFF, 0xFE];
        utf16.extend(SNAPSHOT.encode_utf16().flat_map(|u| u.to_le_bytes()));
        assert!(sniff(&utf16) >= 90);
        let u = read_umdh(&utf16, &Limits::default(), None).unwrap();
        assert_eq!(u.stacks.len(), 2);
        let mut be = vec![0xFE, 0xFF];
        be.extend(SNAPSHOT.encode_utf16().flat_map(|u| u.to_be_bytes()));
        assert_eq!(decode(&be), SNAPSHOT);
        assert_eq!(decode(b"\xEF\xBB\xBFhi"), "hi");
        let small = Limits {
            max_stacks: 1,
            max_umdh_frames: 2,
            ..Limits::default()
        };
        let u = parse(SNAPSHOT, &small, None).unwrap();
        assert_eq!((u.stacks.len(), u.stacks[0].frames.len()), (1, 2));
        assert!(u.notes[0].contains("left out"));
        assert_eq!(
            parse("hello\nworld", &Limits::default(), None).unwrap_err().code,
            ErrorCode::NotADump
        );
        assert_eq!(sniff(b"just text"), 0);
        let long = format!("{}\n{SNAPSHOT}", "x".repeat(MAX_LINE + 1));
        assert!(parse(&long, &Limits::default(), None)
            .unwrap()
            .notes
            .iter()
            .any(|n| n.contains("overlong")));
        let tiny = Limits {
            max_read_bytes: 4,
            ..Limits::default()
        };
        assert_eq!(
            read_umdh(&SNAPSHOT.as_bytes().to_vec(), &tiny, None).unwrap_err().code,
            ErrorCode::Limit
        );
    }
}
