//! Folded / collapsed stacks importer (Brendan Gregg's format, written by
//! `stackcollapse-*.pl`, `inferno-collapse-*`, async-profiler `collapsed`,
//! py-spy `--format raw` and others): one line per distinct stack,
//! `root;caller;...;leaf COUNT`.
//!
//! The count is the last whitespace-separated token: an integer, or a
//! non-negative decimal (rounded). Lines without a count are skipped and
//! noted. stackcollapse-perf annotations are stripped and set the frame
//! kind: `_[k]` kernel, `_[j]` JIT (managed), `_[i]` inlined and `_[w]`
//! waker (native). A leading `comm-pid/tid` or `comm` frame is kept as a
//! frame; the format has no threads (everything is on "all") and no time.
//! Metric: [`MetricId::Samples`], sampled.

use crate::util;
use imt_profile::frames::basename;
use imt_profile::import::SNIFF_BYTES;
use imt_profile::{
    Capture, CaptureImporter, FrameKind, FrameSpec, Limits, Measurement, MetricId, MetricInfo, ProfileBuilder, ProfileError, ProfileKind,
    Result,
};
use regex::Regex;
use std::sync::OnceLock;

const ID: &str = "folded";

/// Lines looked at when sniffing.
const SNIFF_LINES: usize = 20;

/// Imports folded (collapsed) stacks.
pub struct FoldedImporter;

/// Splits `stack COUNT` into the stack and a count (`None` when the line has no valid count).
fn split_line(line: &str) -> Option<(&str, u64)> {
    let line = line.trim();
    let (stack, count) = line.rsplit_once(char::is_whitespace)?;
    let stack = stack.trim_end();
    let n = match count.parse::<u64>() {
        Ok(n) => n,
        Err(_) => {
            let f: f64 = count.parse().ok()?;
            if !f.is_finite() || f < 0.0 || f >= u64::MAX as f64 {
                return None;
            }
            f.round() as u64
        }
    };
    Some((stack, n))
}

/// A frame without its stackcollapse annotation, and the kind the annotation implies.
fn annotation(frame: &str) -> (&str, FrameKind) {
    for (suffix, kind) in [
        ("_[k]", FrameKind::Kernel),
        ("_[j]", FrameKind::Managed),
        ("_[i]", FrameKind::Native),
        ("_[w]", FrameKind::Native),
    ] {
        if let Some(f) = frame.strip_suffix(suffix) {
            return (f, kind);
        }
    }
    (frame, FrameKind::Unknown)
}

fn strict_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"^[^\s;]+(;[^;]+)* \d+$").expect("static regex"))
}

impl CaptureImporter for FoldedImporter {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "folded stacks"
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        let text = util::head_text(head);
        if text.is_empty() || text.starts_with(['{', '[']) {
            return 0;
        }
        // The last line of a full head may be cut.
        let complete = if head.len() >= SNIFF_BYTES {
            text.rsplit_once('\n').map_or("", |(a, _)| a)
        } else {
            &text
        };
        let lines: Vec<&str> = complete
            .lines()
            .map(str::trim_end)
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .take(SNIFF_LINES)
            .collect();
        if lines.is_empty() {
            return 0;
        }
        let ext = util::extension(file_name);
        if ["folded", "collapsed", "stacks"].contains(&ext.as_str()) && lines.iter().all(|l| split_line(l).is_some()) {
            return 90;
        }
        let strict = lines.iter().filter(|l| strict_re().is_match(l)).count();
        let has_stack = lines.iter().any(|l| l.contains(';'));
        if strict == lines.len() && has_stack && (strict >= 3 || lines.len() == strict) {
            60
        } else {
            0
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
        let mut b = ProfileBuilder::new(
            ID,
            ProfileKind::Cpu,
            &[MetricInfo::new(MetricId::Samples, Measurement::Sampled)],
            *limits,
        )?;
        b.set_name(basename(file_name));
        let thread = b.thread(None, "all")?;
        let mut skipped = 0u64;
        let mut frames = Vec::new();
        for line in text.lines() {
            let t = line.trim();
            if t.is_empty() || t.starts_with('#') {
                continue;
            }
            let Some((stack, count)) = split_line(t) else {
                skipped += 1;
                continue;
            };
            frames.clear();
            for name in stack.split(';').filter(|f| !f.trim().is_empty()) {
                if frames.len() > limits.max_stack_depth as usize {
                    break; // the builder notes the cut
                }
                let (name, kind) = annotation(name.trim());
                frames.push(b.frame(FrameSpec::new(name).kind(kind))?);
            }
            let s = b.stack(&frames)?;
            b.sample(thread, 0, s, &[count])?;
        }
        if skipped > 0 {
            b.note(format!("{skipped} lines without a sample count were skipped"));
        }
        if b.sample_count() == 0 {
            return Err(ProfileError::malformed(ID, "no \"frame;frame;... COUNT\" lines found"));
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
        match FoldedImporter.import("dir/app.folded", text.as_bytes(), limits)? {
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

    #[test]
    fn imports_stacks_and_counts() {
        let p = import("# comment\nmain;work;leaf 10\nmain;work 2.6\n\nmain;Foo::bar(int, int) 3\nno count here\n").unwrap();
        assert_eq!(p.source, "folded");
        assert_eq!(p.name, "app.folded");
        assert_eq!(p.kind, ProfileKind::Cpu);
        assert!(!p.has_time);
        assert_eq!(p.threads.len(), 1);
        assert_eq!(p.threads[0].name, "all");
        assert_eq!(p.samples.values, [10, 3, 3]);
        assert_eq!(names(&p, 0), ["main", "work", "leaf"]);
        assert_eq!(names(&p, 2), ["main", "Foo::bar(int, int)"]);
        assert_eq!(p.notes, ["1 lines without a sample count were skipped"]);
    }

    #[test]
    fn strips_annotations_and_keeps_the_comm_frame() {
        let p = import("orders-svc-4242/4243;main_[j];malloc_[i];wake_[w];do_syscall_64_[k] 5\n").unwrap();
        let f: Vec<(&str, FrameKind)> = p.frames.iter().map(|f| (f.function.as_str(), f.kind)).collect();
        assert_eq!(
            f,
            [
                ("orders-svc-4242/4243", FrameKind::Unknown),
                ("main", FrameKind::Managed),
                ("malloc", FrameKind::Native),
                ("wake", FrameKind::Native),
                ("do_syscall_64", FrameKind::Kernel),
            ]
        );
        assert_eq!(p.threads.len(), 1, "no invented threads");
    }

    #[test]
    fn counts_and_odd_lines() {
        assert_eq!(split_line("a;b 0"), Some(("a;b", 0)));
        assert_eq!(split_line("a;b\t7"), Some(("a;b", 7)));
        assert_eq!(split_line("a;b 1e3"), Some(("a;b", 1000)));
        assert_eq!(split_line("a;b -1"), None);
        assert_eq!(split_line("a;b NaN"), None);
        assert_eq!(split_line("a;b inf"), None);
        assert_eq!(split_line("a;b 1e300"), None);
        assert_eq!(split_line("12"), None);
        assert_eq!(split_line(" 12"), None);
        assert_eq!(split_line("a;b x"), None);
        // Empty frames are dropped; a stack of only separators is "(no stack)".
        let p = import("a;;b 1\n;; 2\n").unwrap();
        assert_eq!(names(&p, 0), ["a", "b"]);
        assert_eq!(names(&p, 1), ["(no stack)"]);
    }

    #[test]
    fn errors_and_limits() {
        assert!(import("").unwrap_err().to_string().contains("no \"frame"));
        assert!(import("hello world\n").is_err());
        assert!(FoldedImporter.import("x", &[0xff], &Limits::default()).is_err());
        let small = Limits {
            max_input_bytes: 3,
            ..Limits::default()
        };
        assert!(matches!(import_with("a;b 1\n", &small), Err(ProfileError::LimitExceeded { .. })));
        let few = Limits {
            max_samples: 1,
            ..Limits::default()
        };
        assert!(matches!(import_with("a 1\nb 1\n", &few), Err(ProfileError::LimitExceeded { .. })));
        let shallow = Limits {
            max_stack_depth: 3,
            ..Limits::default()
        };
        let deep = (0..100).map(|i| format!("f{i}")).collect::<Vec<_>>().join(";") + " 1\n";
        let p = import_with(&deep, &shallow).unwrap();
        assert_eq!(names(&p, 0), ["f0", "f1", "f2"]);
        assert!(p.notes.iter().any(|n| n.contains("deeper than 3")));
    }

    #[test]
    fn sniffs_folded_only() {
        assert_eq!((FoldedImporter.id(), FoldedImporter.name()), ("folded", "folded stacks"));
        let s = |name: &str, t: &str| FoldedImporter.sniff(name, t.as_bytes());
        let three = "main;a;b 3\nmain;a 1\nmain;c(int x) 2\n";
        assert_eq!(s("x.folded", three), 90);
        assert_eq!(s("x.collapsed", "a;b 1.5\n"), 90);
        assert_eq!(s("x.stacks", "a;b 1\nnot folded\n"), 0);
        assert_eq!(s("x.txt", three), 60);
        assert_eq!(s("x.txt", "main;a 1\n"), 60, "a short file with a stack");
        assert_eq!(s("x.txt", "Total 5\n"), 0, "no stack separator");
        assert_eq!(s("x.txt", "main;a 1\nmain;b 2\nhello world\n"), 0);
        assert_eq!(s("x.folded", "{\"a\":1}"), 0);
        assert_eq!(s("x.folded", "[1]"), 0);
        assert_eq!(s("x.folded", "# just a comment\n"), 0);
        assert_eq!(s("x.txt", ""), 0);
        assert_eq!(
            s(
                "x.txt",
                "orders 4242/4243 [002] 81234.5: 1000000 cpu-clock:\n\t55d4 main (/bin/x)\n"
            ),
            0
        );
        // A full head whose last line is cut still sniffs.
        let mut big = String::new();
        while big.len() < SNIFF_BYTES {
            big.push_str("main;work;leaf 12\n");
        }
        assert_eq!(FoldedImporter.sniff("x.txt", &big.as_bytes()[..SNIFF_BYTES]), 60);
    }
}
