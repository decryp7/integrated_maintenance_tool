//! The importer port: how a capture file becomes a [`Capture`]. Importers
//! live in plugins (the built-in ones in `imt-plugin-profiling`) and reach
//! the engine through the `profiling.importers` extension point.

use crate::api::{CaptureKind, SniffKind};
use crate::error::{ProfileError, Result};
use crate::heap::HeapSnapshot;
use crate::limits::Limits;
use crate::model::Profile;

/// Id of the extension point importers are contributed to (trait `dyn CaptureImporter`).
pub const IMPORTERS_POINT: &str = "profiling.importers";

/// What an importer produces.
#[derive(Debug, Clone, PartialEq)]
pub enum Capture {
    Profile(Profile),
    Heap(HeapSnapshot),
}

/// Reads one capture format.
///
/// Contract (every implementation): `sniff` and `import` never panic on any
/// input, honour [`Limits`] (input size, nesting, table sizes) and report
/// problems as [`ProfileError`]s that say what is wrong with the file.
pub trait CaptureImporter: Send + Sync {
    /// Stable id, e.g. `speedscope`; recorded as the capture's source.
    fn id(&self) -> &'static str;

    /// Human-readable format name, e.g. `speedscope JSON`.
    fn name(&self) -> &'static str;

    /// Confidence 0 (not mine) to 100 (certain) from the file name and its
    /// first bytes (up to 64 KiB).
    fn sniff(&self, file_name: &str, head: &[u8]) -> u8;

    /// Parses the whole file.
    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture>;

    /// What [`CaptureImporter::import`] produces: profiles unless overridden
    /// (heap snapshot importers return [`CaptureKind::Heap`]).
    fn produces(&self) -> CaptureKind {
        CaptureKind::Profile
    }
}

/// Sniff score from which a file counts as a capture when routing files
/// (drag and drop) without importing them; weaker guesses stay logs.
pub const SNIFF_THRESHOLD: u8 = 50;

/// Bytes of the head handed to [`CaptureImporter::sniff`].
pub const SNIFF_BYTES: usize = 64 * 1024;

/// Picks the importer with the highest sniff score (ties: first) and imports
/// with it. `importers` are tried in order; an importer that fails is
/// reported, it does not fall back to another (the file was recognised).
pub fn import_with(importers: &[&dyn CaptureImporter], file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture> {
    if bytes.len() as u64 > limits.max_input_bytes {
        return Err(ProfileError::LimitExceeded {
            what: "file size",
            limit: limits.max_input_bytes,
            hint: "open a smaller capture",
        });
    }
    let head = &bytes[..bytes.len().min(SNIFF_BYTES)];
    match best(importers, file_name, head) {
        Some((_, importer)) => importer.import(file_name, bytes, limits),
        None => Err(ProfileError::Unrecognised(
            file_name.to_string(),
            importers.iter().map(|i| i.name()).collect::<Vec<_>>().join(", "),
        )),
    }
}

/// The importer with the highest non-zero sniff score (ties: the first).
fn best<'a>(importers: &[&'a dyn CaptureImporter], file_name: &str, head: &[u8]) -> Option<(u8, &'a dyn CaptureImporter)> {
    importers
        .iter()
        .map(|i| (i.sniff(file_name, head).min(100), *i))
        .filter(|(s, _)| *s > 0)
        .fold(None, |best, (s, i)| match best {
            Some((bs, _)) if bs >= s => best,
            _ => Some((s, i)),
        })
}

/// What a file is, from its name and head (up to [`SNIFF_BYTES`]), without
/// importing it: the best importer's kind when it is at least
/// [`SNIFF_THRESHOLD`] confident, [`SniffKind::None`] otherwise.
pub fn sniff_with(importers: &[&dyn CaptureImporter], file_name: &str, head: &[u8]) -> SniffKind {
    let head = &head[..head.len().min(SNIFF_BYTES)];
    match best(importers, file_name, head) {
        Some((score, i)) if score >= SNIFF_THRESHOLD => match i.produces() {
            CaptureKind::Profile => SniffKind::Capture,
            CaptureKind::Heap => SniffKind::Heap,
        },
        _ => SniffKind::None,
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::builder::{FrameSpec, ProfileBuilder};
    use crate::model::{Measurement, MetricId, MetricInfo, ProfileKind};

    /// A test importer: lines `a;b;c N`, sniffing files that start with `#t`.
    pub(crate) struct Lines(pub u8);

    impl CaptureImporter for Lines {
        fn id(&self) -> &'static str {
            "lines"
        }
        fn name(&self) -> &'static str {
            "test lines"
        }
        fn sniff(&self, _file_name: &str, head: &[u8]) -> u8 {
            if head.starts_with(b"#t") {
                self.0
            } else {
                0
            }
        }
        fn import(&self, _file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture> {
            let text = std::str::from_utf8(bytes).map_err(|_| ProfileError::malformed("lines", "not UTF-8"))?;
            let mut b = ProfileBuilder::new(
                "lines",
                ProfileKind::Cpu,
                &[MetricInfo::new(MetricId::Samples, Measurement::Sampled)],
                *limits,
            )?;
            let t = b.thread(None, "main")?;
            for line in text.lines().skip(1) {
                let (stack, n) = line.rsplit_once(' ').ok_or_else(|| ProfileError::malformed("lines", "no count"))?;
                let n: u64 = n.parse().map_err(|_| ProfileError::malformed("lines", "bad count"))?;
                let mut frames = Vec::new();
                for f in stack.split(';') {
                    frames.push(b.frame(FrameSpec::new(f))?);
                }
                let s = b.stack(&frames)?;
                b.sample(t, 0, s, &[n])?;
            }
            Ok(Capture::Profile(b.finish()?))
        }
    }

    /// A test heap importer: files starting with `#h`.
    pub(crate) struct Heaps;

    impl CaptureImporter for Heaps {
        fn id(&self) -> &'static str {
            "heaps"
        }
        fn name(&self) -> &'static str {
            "test heaps"
        }
        fn sniff(&self, _file_name: &str, head: &[u8]) -> u8 {
            if head.starts_with(b"#h") {
                95
            } else {
                0
            }
        }
        fn import(&self, _file_name: &str, _bytes: &[u8], limits: &Limits) -> Result<Capture> {
            let mut b = crate::heap::HeapBuilder::new("heaps", *limits);
            b.type_stat("T", 1, 8)?;
            Ok(Capture::Heap(b.finish()?))
        }
        fn produces(&self) -> CaptureKind {
            CaptureKind::Heap
        }
    }

    #[test]
    fn sniffing_tells_profiles_heaps_and_other_files_apart() {
        let weak = Lines(SNIFF_THRESHOLD - 1);
        let list: Vec<&dyn CaptureImporter> = vec![&weak, &Heaps];
        assert_eq!(sniff_with(&list, "x", b"#t\na 1\n"), SniffKind::None, "a weak guess stays a log");
        assert_eq!(sniff_with(&list, "x", b"#h"), SniffKind::Heap);
        assert_eq!(sniff_with(&list, "x", b"2026-01-01 INFO hello"), SniffKind::None);
        let sure = Lines(SNIFF_THRESHOLD);
        let list: Vec<&dyn CaptureImporter> = vec![&sure, &Heaps];
        assert_eq!(sniff_with(&list, "x", b"#t\na 1\n"), SniffKind::Capture);
        let long: Vec<u8> = b"#t".iter().copied().chain(std::iter::repeat_n(b'x', SNIFF_BYTES * 2)).collect();
        assert_eq!(sniff_with(&list, "x", &long), SniffKind::Capture, "long heads are cut, not refused");
        assert_eq!(sniff_with(&[], "x", b"#t"), SniffKind::None);
        assert_eq!(Lines(1).produces(), CaptureKind::Profile);
        assert!(matches!(
            import_with(&[&Heaps], "h", b"#h", &Limits::default()),
            Ok(Capture::Heap(_))
        ));
    }

    #[test]
    fn picks_the_best_importer_and_reports_unknown_files() {
        let low = Lines(10);
        let high = Lines(90);
        let list: Vec<&dyn CaptureImporter> = vec![&low, &high];
        let c = import_with(&list, "x", b"#t\na;b 3\n", &Limits::default()).unwrap();
        assert!(matches!(c, Capture::Profile(_)));
        let e = import_with(&list, "x.bin", b"zzz", &Limits::default()).unwrap_err();
        assert!(e.to_string().contains("x.bin is not a recognised profile"), "{e}");
        assert!(e.to_string().contains("test lines"));
        let small = Limits {
            max_input_bytes: 2,
            ..Limits::default()
        };
        assert!(matches!(
            import_with(&list, "x", b"#t\n", &small),
            Err(ProfileError::LimitExceeded { .. })
        ));
    }
}
