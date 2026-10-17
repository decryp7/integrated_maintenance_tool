//! `perf script` text importer (Linux perf, default output and `-F` field
//! selections such as `comm,tid,time,period,event,ip,sym,dso`).
//!
//! A sample is a header line followed by its call chain, leaf first, and a
//! blank line:
//!
//! ```text
//! orders-svc 4242/4243 [002] 81234.567890:    1000000 cpu-clock:
//!         55d4c3a1b2c4 orders::Repo::save(orders::Order const&)+0x54 (/opt/orders/bin/orders-svc)
//!     ffffffff8a2001a0 entry_SYSCALL_64_after_hwframe+0x63 ([kernel.kallsyms])
//! ```
//!
//! `comm` may contain spaces and is often right-aligned; `pid/` and `[cpu]`
//! and the period are optional. A header may carry its single frame inline
//! (`... cycles:  7f12 malloc+0x1e (/usr/lib/libc.so.6)`) when perf ran
//! without call graphs. Metrics: [`MetricId::Samples`] (1 per sample) and,
//! when the events are `cpu-clock`/`task-clock` with a period,
//! [`MetricId::TimeNs`] (the period in ns); both sampled. Threads are keyed
//! by tid and named by comm; times are relative to the earliest sample.

use crate::util;
use imt_profile::frames::basename;
use imt_profile::{
    Capture, CaptureImporter, FrameKind, FrameSpec, Limits, Measurement, MetricId, MetricInfo, ProfileBuilder, ProfileError, ProfileKind,
    Result,
};
use regex::Regex;
use std::sync::OnceLock;

const ID: &str = "perf-script";

/// Imports `perf script` output.
pub struct PerfScriptImporter;

/// A parsed sample header.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Header<'a> {
    comm: &'a str,
    tid: Option<u64>,
    time_ns: u64,
    period: Option<u64>,
    event: &'a str,
    /// Text after `event:` (an inline frame, tracepoint fields), trimmed.
    rest: &'a str,
}

/// A parsed call chain entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RawFrame<'a> {
    symbol: &'a str,
    dso: Option<&'a str>,
}

/// One classified input line.
#[derive(Debug, PartialEq, Eq)]
enum Line<'a> {
    Blank,
    Comment,
    Header(Header<'a>),
    Frame(RawFrame<'a>),
    Other,
}

fn header_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        // comm (lazy, may contain spaces) pid/tid [cpu] seconds.fraction: [period] event[:mod...]:[ rest]
        Regex::new(
            r"^(?:(?P<comm>.*?)\s+)?(?:-?\d+/)?(?P<tid>-?\d+)\s+(?:\[\d+\]\s+)?(?P<sec>\d+)\.(?P<frac>\d{1,9}):\s+(?:(?P<period>\d+)\s+)?(?P<event>\S+?):(?:\s+(?P<rest>.*))?$",
        )
        .expect("static regex")
    })
}

/// Parses a header line (leading padding allowed).
fn parse_header(line: &str) -> Option<Header<'_>> {
    let c = header_re().captures(line.trim())?;
    let sec: u64 = c.name("sec")?.as_str().parse().ok()?;
    let frac = c.name("frac")?.as_str();
    let frac_ns: u64 = frac.parse::<u64>().ok()? * 10u64.pow(9 - frac.len() as u32);
    Some(Header {
        comm: c.name("comm").map_or("", |m| m.as_str().trim()),
        tid: c.name("tid").and_then(|m| m.as_str().parse::<u64>().ok()),
        time_ns: sec.checked_mul(1_000_000_000)?.checked_add(frac_ns)?,
        period: c.name("period").and_then(|m| m.as_str().parse().ok()),
        event: c.name("event")?.as_str(),
        rest: c.name("rest").map_or("", |m| m.as_str().trim()),
    })
}

/// Parses `addr symbol+0xoff (dso)` (already trimmed); the dso is the final
/// balanced parenthesis group, so C++ signatures keep theirs.
fn parse_frame(text: &str) -> Option<RawFrame<'_>> {
    let (addr, rest) = text.split_once(char::is_whitespace)?;
    if addr.is_empty() || addr.len() > 16 || !addr.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let rest = rest.trim();
    let (symbol, dso) = match split_dso(rest) {
        Some((s, d)) => (s, Some(d)),
        None => (rest, None),
    };
    Some(RawFrame {
        symbol: strip_offset(symbol),
        dso,
    })
}

/// Splits `text` into the part before its final balanced `( ... )` and the inside.
fn split_dso(text: &str) -> Option<(&str, &str)> {
    let body = text.strip_suffix(')')?;
    let mut depth = 0usize;
    for (i, ch) in body.char_indices().rev() {
        match ch {
            ')' => depth += 1,
            '(' if depth == 0 => return Some((body[..i].trim_end(), &body[i + 1..])),
            '(' => depth -= 1,
            _ => {}
        }
    }
    None
}

/// `foo+0x1e` → `foo`.
fn strip_offset(symbol: &str) -> &str {
    match symbol.rsplit_once("+0x") {
        Some((s, off)) if !off.is_empty() && off.bytes().all(|b| b.is_ascii_hexdigit()) => s,
        _ => symbol,
    }
}

fn classify(line: &str, in_sample: bool) -> Line<'_> {
    let t = line.trim();
    if t.is_empty() {
        return Line::Blank;
    }
    if !in_sample && t.starts_with('#') {
        return Line::Comment;
    }
    if let Some(h) = parse_header(line) {
        return Line::Header(h);
    }
    if in_sample && line.starts_with(char::is_whitespace) {
        if let Some(f) = parse_frame(t) {
            return Line::Frame(f);
        }
    }
    Line::Other
}

fn is_clock_event(event: &str) -> bool {
    let base = event.split(':').next().unwrap_or(event);
    base == "cpu-clock" || base == "task-clock"
}

fn frame_kind(dso: Option<&str>) -> FrameKind {
    let Some(d) = dso else { return FrameKind::Unknown };
    let lower = d.to_ascii_lowercase();
    if lower.is_empty() || lower == "[unknown]" {
        FrameKind::Unknown
    } else if lower.contains("kernel") || lower.contains("vmlinux") {
        FrameKind::Kernel
    } else if is_jit_map(basename(&lower)) {
        FrameKind::Managed
    } else {
        FrameKind::Native
    }
}

/// `perf-<pid>.map`: symbols a JIT (JVM, Node, .NET) wrote for perf.
fn is_jit_map(base: &str) -> bool {
    base.strip_prefix("perf-")
        .and_then(|r| r.strip_suffix(".map"))
        .is_some_and(|pid| !pid.is_empty() && pid.bytes().all(|b| b.is_ascii_digit()))
}

/// What the first pass learns: the earliest time and whether time can be shown.
struct Survey {
    first_ns: u64,
    has_clock_period: bool,
    headers: u64,
}

fn survey(text: &str, limits: &Limits) -> Result<Survey> {
    let mut s = Survey {
        first_ns: u64::MAX,
        has_clock_period: false,
        headers: 0,
    };
    let mut in_sample = false;
    for line in text.lines() {
        match classify(line, in_sample) {
            Line::Header(h) => {
                s.headers += 1;
                if s.headers > limits.max_samples {
                    return Err(ProfileError::LimitExceeded {
                        what: "samples",
                        limit: limits.max_samples,
                        hint: "open a shorter recording or a lower sampling rate",
                    });
                }
                s.first_ns = s.first_ns.min(h.time_ns);
                s.has_clock_period |= is_clock_event(h.event) && h.period.is_some_and(|p| p > 0);
                in_sample = true;
            }
            Line::Blank => in_sample = false,
            _ => {}
        }
    }
    Ok(s)
}

/// Builds the profile in the second pass.
struct Importer<'b> {
    b: &'b mut ProfileBuilder,
    first_ns: u64,
    with_time: bool,
    leaf_first: Vec<u32>,
    current: Option<Header<'b>>,
}

impl<'t> Importer<'t> {
    fn frame(&mut self, f: RawFrame<'_>) -> Result<()> {
        let module = f.dso.filter(|d| !d.eq_ignore_ascii_case("[unknown]")).map(basename);
        let symbol = if f.symbol.is_empty() { "[unknown]" } else { f.symbol };
        let id = self.b.frame(FrameSpec::new(symbol).module(module).kind(frame_kind(f.dso)))?;
        // The builder cuts stacks deeper than the limit to their outermost frames.
        self.leaf_first.push(id);
        Ok(())
    }

    fn flush(&mut self) -> Result<()> {
        let Some(h) = self.current.take() else { return Ok(()) };
        if self.leaf_first.is_empty() {
            if let Some(f) = parse_frame(h.rest) {
                self.frame(f)?;
            }
        }
        let thread = match h.tid {
            Some(tid) => self.b.thread(Some(tid), h.comm)?,
            None => self.b.thread(None, h.comm)?,
        };
        let stack = self.b.stack_leaf_first(&self.leaf_first)?;
        self.leaf_first.clear();
        let time = h.time_ns.saturating_sub(self.first_ns);
        if self.with_time {
            let ns = if is_clock_event(h.event) { h.period.unwrap_or(0) } else { 0 };
            self.b.sample(thread, time, stack, &[1, ns])
        } else {
            self.b.sample(thread, time, stack, &[1])
        }
    }
}

impl CaptureImporter for PerfScriptImporter {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "perf script"
    }

    fn sniff(&self, _file_name: &str, head: &[u8]) -> u8 {
        let text = util::head_text(head);
        if text.starts_with(['{', '[']) {
            return 0;
        }
        let mut lines = text.lines().filter(|l| !l.trim().is_empty() && !l.trim_start().starts_with('#'));
        let Some(first) = lines.next() else { return 0 };
        let Some(h) = parse_header(first) else { return 0 };
        match lines.next() {
            Some(next) if next.starts_with(char::is_whitespace) && parse_header(next).is_none() && parse_frame(next.trim()).is_some() => 85,
            Some(next) if parse_header(next).is_some() => 60,
            None if parse_frame(h.rest).is_some() => 60,
            _ => 0,
        }
    }

    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture> {
        if bytes.len() as u64 > limits.max_input_bytes {
            return Err(ProfileError::LimitExceeded {
                what: "file size",
                limit: limits.max_input_bytes,
                hint: "open a smaller capture",
            });
        }
        let text = util::text(bytes, ID)?;
        let s = survey(text, limits)?;
        if s.headers == 0 {
            return Err(ProfileError::malformed(
                ID,
                "no sample headers found (expected lines like \"comm 123/456 [000] 12.345678: 1000000 cpu-clock:\")",
            ));
        }
        let mut metrics = vec![MetricInfo::new(MetricId::Samples, Measurement::Sampled)];
        if s.has_clock_period {
            metrics.push(MetricInfo::new(MetricId::TimeNs, Measurement::Sampled));
        }
        let mut b = ProfileBuilder::new(ID, ProfileKind::Cpu, &metrics, *limits)?;
        b.set_name(basename(file_name)).set_has_time(true);
        let mut skipped = 0u64;
        let mut orphans = 0u64;
        {
            let mut imp = Importer {
                b: &mut b,
                first_ns: s.first_ns,
                with_time: s.has_clock_period,
                leaf_first: Vec::new(),
                current: None,
            };
            for line in text.lines() {
                match classify(line, imp.current.is_some()) {
                    Line::Header(h) => {
                        imp.flush()?;
                        imp.current = Some(h);
                    }
                    Line::Frame(f) => imp.frame(f)?,
                    Line::Blank => imp.flush()?,
                    Line::Comment => {}
                    Line::Other if imp.current.is_some() => skipped += 1,
                    Line::Other => orphans += 1,
                }
            }
            imp.flush()?;
        }
        if skipped + orphans > 0 {
            b.note(format!(
                "{} lines that are not perf script samples or frames were skipped",
                skipped + orphans
            ));
        }
        Ok(Capture::Profile(b.finish()?))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_profile::Profile;

    fn import(text: &str) -> Result<Profile> {
        import_with(text, &Limits::default())
    }

    fn import_with(text: &str, limits: &Limits) -> Result<Profile> {
        match PerfScriptImporter.import("/tmp/perf.txt", text.as_bytes(), limits)? {
            Capture::Profile(p) => Ok(p),
            Capture::Heap(_) => unreachable!(),
        }
    }

    fn names(p: &Profile, i: usize) -> Vec<String> {
        p.stack_frames(p.samples.stack[i])
            .iter()
            .map(|&f| p.frames[f as usize].function.clone())
            .collect()
    }

    const TWO: &str = "\
# ========
# captured on: today
# ========
#
      orders-svc 4242/4243 [002] 81234.500000:    1000000 cpu-clock:
\t    55d4c3a1b2c4 orders::Repo::save(orders::Order const&)+0x54 (/opt/orders/bin/orders-svc)
\t    55d4c3a10000 main+0x10 (/opt/orders/bin/orders-svc)
\t    7f3a2b1c0000 __libc_start_main+0xf3 (/usr/lib/x86_64-linux-gnu/libc.so.6)

Web Content 4242/4244 81234.502500: 1000000 cpu-clock:u:
\tffffffff8a2001a0 entry_SYSCALL_64_after_hwframe+0x63 ([kernel.kallsyms])
\t    7f3a2b1c4d5e [unknown] ([unknown])
\t    55d4c3a10000 main+0x10 (/opt/orders/bin/orders-svc)
";

    #[test]
    fn imports_samples_with_threads_kinds_and_time() {
        let p = import(TWO).unwrap();
        assert_eq!(p.source, "perf-script");
        assert_eq!(p.name, "perf.txt");
        assert_eq!(p.kind, ProfileKind::Cpu);
        assert!(p.has_time);
        assert_eq!(
            p.metrics.iter().map(|m| m.id).collect::<Vec<_>>(),
            [MetricId::Samples, MetricId::TimeNs]
        );
        assert_eq!(p.samples.len(), 2);
        assert_eq!(
            names(&p, 0),
            ["__libc_start_main", "main", "orders::Repo::save(orders::Order const&)"]
        );
        assert_eq!(names(&p, 1), ["main", "[unknown]", "entry_SYSCALL_64_after_hwframe"]);
        assert_eq!(p.samples.time, [0, 2_500_000]);
        assert_eq!(p.samples.values, [1, 1_000_000, 1, 1_000_000]);
        let threads: Vec<(Option<u64>, &str)> = p.threads.iter().map(|t| (t.os_id, t.name.as_str())).collect();
        assert_eq!(threads, [(Some(4243), "orders-svc"), (Some(4244), "Web Content")]);
        let by = |n: &str| p.frames.iter().find(|f| f.function == n).unwrap();
        assert_eq!(by("entry_SYSCALL_64_after_hwframe").kind, FrameKind::Kernel);
        assert_eq!(by("[unknown]").kind, FrameKind::Unknown);
        assert_eq!(by("[unknown]").module, None);
        assert_eq!(by("main").kind, FrameKind::Native);
        assert_eq!(by("main").module.as_deref(), Some("orders-svc"));
        assert!(by("__libc_start_main").is_framework);
        assert!(p.notes.is_empty(), "{:?}", p.notes);
    }

    #[test]
    fn header_variants() {
        let h = parse_header("perf 51279 [004] 112693.399005:          1 cycles:ppp:").unwrap();
        assert_eq!((h.comm, h.tid, h.period, h.event), ("perf", Some(51279), Some(1), "cycles:ppp"));
        assert_eq!(h.time_ns, 112_693_399_005_000);
        let h = parse_header("java 12345 1234.5: cpu-clock:").unwrap();
        assert_eq!(
            (h.comm, h.tid, h.period, h.event, h.time_ns),
            ("java", Some(12345), None, "cpu-clock", 1_234_500_000_000)
        );
        let h = parse_header("swapper     0 [000] 1.000000001: sched:sched_switch: prev_comm=swapper next_comm=x").unwrap();
        assert_eq!((h.event, h.rest), ("sched:sched_switch", "prev_comm=swapper next_comm=x"));
        let h = parse_header(":-1 -1 [001] 5.0: 10 cpu-clock:  7f12 malloc+0x1e (/usr/lib/libc.so.6)").unwrap();
        assert_eq!((h.comm, h.tid, h.rest), (":-1", None, "7f12 malloc+0x1e (/usr/lib/libc.so.6)"));
        let h = parse_header("4243 3.25: 1 task-clock:").unwrap();
        assert_eq!((h.comm, h.tid), ("", Some(4243)));
        assert!(parse_header("my comm 1 2 3/4 [000] 9.5: 1 cycles:").is_some(), "comm with numbers");
        assert!(parse_header("\t55d4c3a1b2c4 main+0x10 (/bin/x)").is_none());
        assert!(parse_header("a;b;c 12").is_none());
        assert!(parse_header("x 1 99999999999999999999.5: 1 cycles:").is_none(), "time overflows");
        assert!(is_clock_event("cpu-clock:u") && is_clock_event("task-clock") && !is_clock_event("cycles"));
    }

    #[test]
    fn frame_variants() {
        let f = parse_frame("7f12 std::vector<int, std::allocator<int> >::push_back(int const&) (/usr/lib/libstdc++.so.6)").unwrap();
        assert_eq!(f.symbol, "std::vector<int, std::allocator<int> >::push_back(int const&)");
        assert_eq!(f.dso, Some("/usr/lib/libstdc++.so.6"));
        let f = parse_frame("7f12 foo (anonymous namespace)::bar+0x1 (/x (deleted))").unwrap();
        assert_eq!((f.symbol, f.dso), ("foo (anonymous namespace)::bar", Some("/x (deleted)")));
        let f = parse_frame("7f12 bare_symbol+0xzz").unwrap();
        assert_eq!((f.symbol, f.dso), ("bare_symbol+0xzz", None));
        let f = parse_frame("7f12 sym+0x (lib)").unwrap();
        assert_eq!(f.symbol, "sym+0x");
        assert_eq!(parse_frame("7f12 unbalanced)").unwrap().dso, None);
        assert!(parse_frame("xyz main (/bin/x)").is_none());
        assert!(parse_frame("12345678901234567 main (/bin/x)").is_none(), "address too long");
        assert!(parse_frame("7f12").is_none());
        assert_eq!(frame_kind(Some("/boot/vmlinux-6.1")), FrameKind::Kernel);
        assert_eq!(frame_kind(Some("/tmp/perf-4242.map")), FrameKind::Managed);
        assert_eq!(frame_kind(Some("/tmp/perf-.map")), FrameKind::Native);
        assert_eq!(frame_kind(Some("")), FrameKind::Unknown);
        assert_eq!(frame_kind(None), FrameKind::Unknown);
    }

    #[test]
    fn other_events_inline_frames_and_junk() {
        let text = "\
prog 10 1.0: 100 cycles:  7f12 malloc+0x1e (/usr/lib/libc.so.6)
prog 10 1.5: 100 cycles:
prog 11 2.0: 100 cycles:
\t7f12 work (/bin/prog)
\tsrc/work.c:12
garbage before nothing

trailing junk line
";
        let p = import(text).unwrap();
        assert_eq!(p.metrics.len(), 1, "cycles: samples only");
        assert_eq!(p.samples.len(), 3);
        assert_eq!(names(&p, 0), ["malloc"]);
        assert_eq!(names(&p, 1), ["(no stack)"]);
        assert_eq!(names(&p, 2), ["work"]);
        assert!(p.notes.iter().any(|n| n.starts_with("3 lines")), "{:?}", p.notes);

        // Mixed events: clock samples carry time, others 0.
        let p = import("a 1 1.0: 500 cpu-clock:\n\na 1 2.0: 7 cycles:\n").unwrap();
        assert_eq!(p.samples.values, [1, 500, 1, 0]);
        // A clock event without a period: samples only.
        assert_eq!(import("a 1 1.0: cpu-clock:\n").unwrap().metrics.len(), 1);
    }

    #[test]
    fn deep_stacks_keep_the_outermost_frames() {
        let mut text = String::from("p 1 1.0: 1 cpu-clock:\n");
        for i in 0..30 {
            text.push_str(&format!("\t{i:x} f{i} (/bin/p)\n"));
        }
        let limits = Limits {
            max_stack_depth: 8,
            ..Limits::default()
        };
        let p = import_with(&text, &limits).unwrap();
        let n = names(&p, 0);
        assert_eq!(n.len(), 8);
        assert_eq!(n[0], "f29", "root kept");
        assert!(p.notes.iter().any(|n| n.contains("deeper than 8")));
    }

    #[test]
    fn errors_and_limits() {
        assert!(import("").unwrap_err().to_string().contains("no sample headers"));
        assert!(import("main;foo 3\n").is_err());
        assert!(PerfScriptImporter.import("x", &[0xff, 0xfe], &Limits::default()).is_err());
        let few = Limits {
            max_samples: 1,
            ..Limits::default()
        };
        assert!(matches!(
            import_with(TWO, &few),
            Err(ProfileError::LimitExceeded { what: "samples", .. })
        ));
        let small = Limits {
            max_input_bytes: 10,
            ..Limits::default()
        };
        assert!(matches!(import_with(TWO, &small), Err(ProfileError::LimitExceeded { .. })));
        let frames = Limits {
            max_frames: 2,
            ..Limits::default()
        };
        assert!(matches!(import_with(TWO, &frames), Err(ProfileError::LimitExceeded { .. })));
    }

    #[test]
    fn sniffs_perf_script_only() {
        assert_eq!((PerfScriptImporter.id(), PerfScriptImporter.name()), ("perf-script", "perf script"));
        let s = |t: &str| PerfScriptImporter.sniff("x.txt", t.as_bytes());
        assert_eq!(s(TWO), 85);
        assert_eq!(s("a 1 1.0: 1 cycles:\na 1 2.0: 1 cycles:\n"), 60, "no call graphs");
        assert_eq!(s("a 1 1.0: 1 cycles:  7f12 malloc (/lib/libc.so.6)\n"), 60);
        assert_eq!(s("a 1 1.0: 1 cycles:\n"), 0, "a lone header is too little");
        assert_eq!(s("a 1 1.0: 1 cycles:\nnot a frame\n"), 0);
        assert_eq!(s("Shop.Program.Main;Shop.Orders.OrderService.PlaceOrder 12\nA;B 3\n"), 0);
        assert_eq!(s("{\"traceEvents\":[]}"), 0);
        assert_eq!(s("[{\"ph\":\"X\"}]"), 0);
        assert_eq!(s("# only comments\n"), 0);
        assert_eq!(s(""), 0);
        assert_eq!(s("2026-10-14 12:00:00.123: INFO started\n  at foo\n"), 0);
    }
}
