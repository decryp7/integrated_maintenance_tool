//! pprof `profile.proto` importer (Go, `pprof`-compatible tools, async-profiler,
//! py-spy and others), gzip-compressed (the usual form) or plain.
//!
//! Sample types map to metrics (all [`Measurement::Sampled`]):
//!
//! | type / unit | metric |
//! |---|---|
//! | `samples`, `contentions`, `goroutine` / `count` | [`MetricId::Samples`] |
//! | any type / time unit (`nanoseconds`, `microseconds`, `milliseconds`, `seconds`) | [`MetricId::TimeNs`] (converted to ns) |
//! | `delay`, `wait`, `block`, `lock*` / time unit | [`MetricId::WaitNs`] |
//! | `alloc_objects`, `inuse_objects` / `count` | [`MetricId::AllocCount`] |
//! | any type / `bytes` | [`MetricId::AllocBytes`] |
//!
//! When two sample types map to the same metric (a Go heap profile has both
//! `alloc_*` and `inuse_*`), the allocation one wins and the other is noted.
//! Other sample types are skipped with a note. Threads come from a
//! `thread`, `thread_name`, `threadid`, `thread_id`, `tid` or `goroutine`
//! label; without one every sample is on the thread "all". pprof samples
//! carry no time unless every sample has a numeric `timestamp` label.

pub mod proto;
mod wire;

use crate::util;
use imt_profile::frames::basename;
use imt_profile::{
    Capture, CaptureImporter, FrameKind, FrameSpec, Limits, Measurement, MetricId, MetricInfo, ProfileBuilder, ProfileError, ProfileKind,
    Result,
};
use prost::Message;
use std::cell::Cell;
use std::collections::HashMap;
use std::io::Read;

const ID: &str = "pprof";

/// Label keys that name the thread (or goroutine) of a sample.
const THREAD_KEYS: &[&str] = &["thread", "thread_name", "threadid", "thread_id", "tid", "goroutine"];

/// Imports pprof `profile.proto` files.
pub struct PprofImporter;

impl CaptureImporter for PprofImporter {
    fn id(&self) -> &'static str {
        ID
    }

    fn name(&self) -> &'static str {
        "pprof (profile.proto)"
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        let named = has_pprof_name(file_name);
        if util::is_gzip(head) {
            let inner = inflate_head(head);
            if inner.is_empty() {
                // Damaged or unreadable gzip: only a weak claim, so other importers win.
                return if named { 40 } else { 20 };
            }
            match (wire::looks_like_profile(&inner), named) {
                (true, true) => 95,
                (true, false) => 75,
                (false, _) => 0,
            }
        } else if wire::looks_like_profile(head) {
            if named {
                90
            } else {
                10
            }
        } else {
            0
        }
    }

    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture> {
        if bytes.is_empty() {
            return Err(ProfileError::malformed(ID, "the file is empty"));
        }
        let data = util::maybe_gunzip(bytes, limits, ID)?;
        if data.len() as u64 > limits.max_input_bytes {
            return Err(ProfileError::LimitExceeded {
                what: "file size",
                limit: limits.max_input_bytes,
                hint: "open a smaller capture",
            });
        }
        let counts = wire::scan(&data).map_err(|e| ProfileError::malformed(ID, format!("not a pprof profile: {e}")))?;
        check_counts(&counts, limits)?;
        let profile = proto::Profile::decode(&*data).map_err(|e| ProfileError::malformed(ID, format!("not a pprof profile: {e}")))?;
        Converter::new(&profile, *limits).run(file_name).map(Capture::Profile)
    }
}

/// File names pprof tools write: `*.pb.gz`, `*.pprof`, `*.prof`, `pprof.*`.
fn has_pprof_name(file_name: &str) -> bool {
    let base = basename(file_name).to_ascii_lowercase();
    let stem = base.strip_suffix(".gz").unwrap_or(&base);
    [".pb", ".pprof", ".prof"].iter().any(|e| stem.ends_with(e)) || base.starts_with("pprof.")
}

/// Up to 512 bytes inflated from the start of a (possibly cut) gzip head.
fn inflate_head(head: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(512);
    let mut buf = [0u8; 512];
    let mut d = flate2::read::GzDecoder::new(head);
    while out.len() < 512 {
        match d.read(&mut buf[..512 - out.len()]) {
            Ok(0) | Err(_) => break,
            Ok(n) => out.extend_from_slice(&buf[..n]),
        }
    }
    out
}

fn check_counts(c: &wire::Counts, limits: &Limits) -> Result<()> {
    let frames = u64::from(limits.max_frames);
    let checks = [
        (
            c.samples,
            limits.max_samples,
            "samples",
            "open a shorter recording or a lower sampling rate",
        ),
        (
            c.locations,
            u64::from(limits.max_stack_nodes),
            "locations",
            "split the capture into shorter recordings",
        ),
        (
            c.functions,
            frames,
            "distinct functions",
            "split the capture or record fewer modules",
        ),
        (c.mappings, frames, "mappings", "the capture maps an unusual number of modules"),
        (
            c.strings,
            frames.saturating_mul(4),
            "strings",
            "the capture has an unusual number of names",
        ),
    ];
    for (n, limit, what, hint) in checks {
        if n > limit {
            return Err(ProfileError::LimitExceeded { what, limit, hint });
        }
    }
    Ok(())
}

/// Nanoseconds per unit of a time unit name.
fn time_scale(unit: &str) -> Option<u64> {
    match unit {
        "nanoseconds" | "nanosecond" | "ns" => Some(1),
        "microseconds" | "microsecond" | "us" | "µs" => Some(1_000),
        "milliseconds" | "millisecond" | "ms" => Some(1_000_000),
        "seconds" | "second" | "s" => Some(1_000_000_000),
        _ => None,
    }
}

/// How a sample type is shown: metric, multiplier to the metric's unit, and
/// whether it is memory in use (rather than allocated).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mapped {
    metric: MetricId,
    scale: u64,
    in_use: bool,
}

fn classify(ty: &str, unit: &str) -> Option<Mapped> {
    let ty = ty.to_ascii_lowercase();
    let unit = unit.to_ascii_lowercase();
    let in_use = ty.starts_with("inuse");
    let m = |metric, scale| Some(Mapped { metric, scale, in_use });
    if let Some(scale) = time_scale(&unit) {
        let waits = ["delay", "wait", "block", "lock"].iter().any(|w| ty.contains(w));
        return m(if waits { MetricId::WaitNs } else { MetricId::TimeNs }, scale);
    }
    match unit.as_str() {
        "bytes" | "byte" => m(MetricId::AllocBytes, 1),
        "count" | "samples" | "" => match ty.as_str() {
            "samples" | "sample" | "contentions" | "goroutine" | "goroutines" | "events" => m(MetricId::Samples, 1),
            t if t.contains("objects") || t == "allocations" || t == "alloc_count" => m(MetricId::AllocCount, 1),
            _ => None,
        },
        _ => None,
    }
}

/// A chosen sample type: its value index and mapping.
#[derive(Debug, Clone, Copy)]
struct Column {
    index: usize,
    mapped: Mapped,
}

/// Picks one sample type per metric (allocated over in use, then first) and
/// describes what was skipped.
fn choose_columns(types: &[(String, String)]) -> (Vec<Column>, Vec<String>) {
    let mut chosen: Vec<Column> = Vec::new();
    let mut notes = Vec::new();
    let mapped: Vec<Option<Mapped>> = types.iter().map(|(t, u)| classify(t, u)).collect();
    for (index, m) in mapped.iter().enumerate() {
        let Some(m) = *m else {
            let (t, u) = &types[index];
            notes.push(format!("sample type {t}/{u} is not supported and was skipped"));
            continue;
        };
        match chosen.iter_mut().find(|c| c.mapped.metric == m.metric) {
            None => chosen.push(Column { index, mapped: m }),
            Some(c) => {
                let replace = c.mapped.in_use && !m.in_use;
                let skipped = if replace { c.index } else { index };
                if replace {
                    *c = Column { index, mapped: m };
                }
                let (t, u) = &types[skipped];
                notes.push(format!(
                    "sample type {t}/{u} was skipped (another sample type gives {})",
                    m.metric.label()
                ));
            }
        }
    }
    chosen.sort_by_key(|c| c.index);
    (chosen, notes)
}

fn kind_of(metrics: &[MetricId]) -> ProfileKind {
    let all = |set: &[MetricId]| metrics.iter().all(|m| set.contains(m));
    if all(&[MetricId::TimeNs, MetricId::Samples]) {
        ProfileKind::Cpu
    } else if all(&[MetricId::AllocCount, MetricId::AllocBytes]) {
        ProfileKind::Alloc
    } else if all(&[MetricId::WaitNs, MetricId::Samples]) {
        ProfileKind::Wall
    } else {
        ProfileKind::Mixed
    }
}

/// Frame kind from the mapping's file and the function's source file.
fn frame_kind(mapping_file: &str, source_file: &str) -> FrameKind {
    let m = mapping_file.to_ascii_lowercase();
    if m.starts_with("[kernel") || m.contains("vmlinux") {
        return FrameKind::Kernel;
    }
    let ext = util::extension(source_file);
    const MANAGED: &[&str] = &["js", "mjs", "cjs", "ts", "java", "kt", "scala", "cs", "fs", "vb", "py", "rb", "php"];
    if MANAGED.contains(&ext.as_str()) {
        FrameKind::Managed
    } else {
        FrameKind::Native
    }
}

/// Turns a decoded profile into the common model.
struct Converter<'p> {
    p: &'p proto::Profile,
    limits: Limits,
    functions: HashMap<u64, usize>,
    mappings: HashMap<u64, usize>,
    locations: HashMap<u64, usize>,
    /// Leaf-first frame ids per location index, built on first use.
    location_frames: Vec<Option<Vec<u32>>>,
    unknown_location_frame: Option<u32>,
    bad_strings: Cell<u64>,
    unknown_locations: u64,
    unknown_functions: u64,
    unsymbolized: u64,
    duplicates: u64,
}

/// Maps ids to their first index; counts duplicates.
fn index_by_id(ids: impl Iterator<Item = u64>, duplicates: &mut u64) -> HashMap<u64, usize> {
    let mut map = HashMap::new();
    for (i, id) in ids.enumerate() {
        match map.entry(id) {
            std::collections::hash_map::Entry::Vacant(e) => {
                e.insert(i);
            }
            std::collections::hash_map::Entry::Occupied(_) => *duplicates += 1,
        }
    }
    map
}

impl<'p> Converter<'p> {
    fn new(p: &'p proto::Profile, limits: Limits) -> Converter<'p> {
        let mut duplicates = 0;
        let functions = index_by_id(p.function.iter().map(|f| f.id), &mut duplicates);
        let mappings = index_by_id(p.mapping.iter().map(|m| m.id), &mut duplicates);
        let locations = index_by_id(p.location.iter().map(|l| l.id), &mut duplicates);
        Converter {
            p,
            limits,
            functions,
            mappings,
            locations,
            location_frames: vec![None; p.location.len()],
            unknown_location_frame: None,
            bad_strings: Cell::new(0),
            unknown_locations: 0,
            unknown_functions: 0,
            unsymbolized: 0,
            duplicates,
        }
    }

    /// A string table entry; a bad index is counted and reads as "".
    fn str(&self, i: i64) -> &'p str {
        match usize::try_from(i).ok().and_then(|i| self.p.string_table.get(i)) {
            Some(s) => s,
            None => {
                self.bad_strings.set(self.bad_strings.get() + 1);
                ""
            }
        }
    }

    fn run(mut self, file_name: &str) -> Result<imt_profile::Profile> {
        let p = self.p;
        let types: Vec<(String, String)> = p
            .sample_type
            .iter()
            .map(|t| (self.str(t.r#type).to_string(), self.str(t.unit).to_string()))
            .collect();
        if types.is_empty() {
            return Err(ProfileError::malformed(ID, "the profile declares no sample types"));
        }
        let (columns, type_notes) = choose_columns(&types);
        if columns.is_empty() {
            let list = types.iter().map(|(t, u)| format!("{t}/{u}")).collect::<Vec<_>>().join(", ");
            return Err(ProfileError::unsupported(
                ID,
                format!("none of the sample types ({list}) is supported"),
            ));
        }
        let metrics: Vec<MetricInfo> = columns
            .iter()
            .map(|c| MetricInfo::new(c.mapped.metric, Measurement::Sampled))
            .collect();
        let ids: Vec<MetricId> = metrics.iter().map(|m| m.id).collect();
        let mut b = ProfileBuilder::new(ID, kind_of(&ids), &metrics, self.limits)?;
        let base = basename(file_name);
        let base = base.strip_suffix(".gz").unwrap_or(base);
        let in_use = columns.iter().any(|c| c.mapped.in_use);
        b.set_name(&if in_use { format!("{base} (in use)") } else { base.to_string() });
        if in_use {
            b.note("values are memory in use when the profile was taken (inuse_* sample types), not all allocations");
        }
        for n in type_notes {
            b.note(n);
        }
        if p.time_nanos > 0 {
            b.set_started_ms(Some(p.time_nanos / 1_000_000));
        }
        if p.duration_nanos > 0 {
            b.set_duration_ns(p.duration_nanos as u64);
        }
        for c in p.comment.iter().take(5) {
            let text = self.str(*c);
            if !text.is_empty() {
                b.note(format!("comment: {text}"));
            }
        }
        let times = self.timestamps();
        b.set_has_time(times.is_some());

        let mut bad_values = 0u64;
        let mut negative = 0u64;
        let cap = self.limits.max_stack_depth as usize + 1;
        let mut root_first: Vec<u32> = Vec::new();
        let mut row = vec![0u64; columns.len()];
        for (si, s) in p.sample.iter().enumerate() {
            if s.value.len() != types.len() {
                bad_values += 1;
                continue;
            }
            for (slot, c) in row.iter_mut().zip(&columns) {
                let v = s.value[c.index];
                if v < 0 {
                    negative += 1;
                }
                *slot = (v.max(0) as u64).saturating_mul(c.mapped.scale);
            }
            root_first.clear();
            'stack: for &lid in s.location_id.iter().rev() {
                let frames = match self.location(&mut b, lid)? {
                    Some(li) => self.location_frames[li].as_deref().unwrap_or(&[]),
                    None => self.unknown_location_frame.as_slice(),
                };
                for &f in frames.iter().rev() {
                    root_first.push(f);
                    if root_first.len() >= cap {
                        break 'stack;
                    }
                }
            }
            let stack = b.stack(&root_first)?;
            let thread = self.thread(&mut b, s)?;
            let time = times.as_ref().map_or(0, |t| t[si]);
            b.sample(thread, time, stack, &row)?;
        }
        let counted = [
            (bad_values, "samples with the wrong number of values were skipped"),
            (negative, "negative values (a difference profile?) were counted as 0"),
            (self.unknown_locations, "stack entries refer to unknown locations"),
            (self.unknown_functions, "source lines refer to unknown functions"),
            (self.unsymbolized, "locations have no symbols and are shown by address"),
            (
                self.duplicates,
                "functions, mappings or locations have duplicate ids (the first was used)",
            ),
            (self.bad_strings.get(), "references to missing strings were read as empty"),
        ];
        for (n, what) in counted {
            if n > 0 {
                b.note(format!("{n} {what}"));
            }
        }
        b.finish()
    }

    /// Per-sample times (ns since the earliest) when every sample has a numeric `timestamp` label.
    fn timestamps(&self) -> Option<Vec<u64>> {
        if self.p.sample.is_empty() {
            return None;
        }
        let mut raw = Vec::with_capacity(self.p.sample.len());
        for s in &self.p.sample {
            let l = s
                .label
                .iter()
                .find(|l| l.str == 0 && self.str(l.key).eq_ignore_ascii_case("timestamp"))?;
            let scale = time_scale(&self.str(l.num_unit).to_ascii_lowercase()).unwrap_or(1);
            raw.push(i128::from(l.num) * i128::from(scale));
        }
        let min = raw.iter().copied().min().unwrap_or(0);
        Some(raw.into_iter().map(|t| u64::try_from(t - min).unwrap_or(u64::MAX)).collect())
    }

    fn thread(&self, b: &mut ProfileBuilder, s: &proto::Sample) -> Result<u32> {
        let mut name: Option<&str> = None;
        let mut os: Option<(u64, &str)> = None;
        for l in &s.label {
            let key = self.str(l.key);
            if !THREAD_KEYS.iter().any(|k| key.eq_ignore_ascii_case(k)) {
                continue;
            }
            if l.str != 0 {
                name = Some(self.str(l.str));
            } else if let Ok(n) = u64::try_from(l.num) {
                os = Some((n, key));
            }
        }
        match (os, name) {
            (Some((id, _)), Some(name)) => b.thread(Some(id), name),
            (Some((id, key)), None) => b.thread(Some(id), &format!("{key} {id}")),
            (None, Some(name)) => b.thread(None, name),
            (None, None) => b.thread(None, "all"),
        }
    }

    /// Resolves a location's frames into the cache: `Some(index)` of a known
    /// location (frames leaf first: inlined functions first, the caller
    /// last), `None` for an unknown id (whose frame is `unknown_location_frame`).
    fn location(&mut self, b: &mut ProfileBuilder, id: u64) -> Result<Option<usize>> {
        let Some(&li) = self.locations.get(&id) else {
            self.unknown_locations += 1;
            if self.unknown_location_frame.is_none() {
                self.unknown_location_frame = Some(b.frame(FrameSpec::new("(unknown location)").kind(FrameKind::Unknown))?);
            }
            return Ok(None);
        };
        if self.location_frames[li].is_some() {
            return Ok(Some(li));
        }
        let p = self.p;
        let loc = &p.location[li];
        let mapping = self.mappings.get(&loc.mapping_id).map(|&i| &p.mapping[i]);
        let mapping_file = mapping.map_or("", |m| self.str(m.filename));
        let module = Some(basename(mapping_file)).filter(|m| !m.is_empty());
        let mut frames = Vec::with_capacity(loc.line.len().max(1));
        if loc.line.is_empty() {
            self.unsymbolized += 1;
            let name = format!("0x{:x}", loc.address);
            let kind = if mapping.is_some() {
                frame_kind(mapping_file, "")
            } else {
                FrameKind::Unknown
            };
            frames.push(b.frame(FrameSpec::new(&name).module(module).kind(kind))?);
        }
        for line in &loc.line {
            let Some(f) = self.functions.get(&line.function_id).map(|&i| &p.function[i]) else {
                self.unknown_functions += 1;
                let kind = frame_kind(mapping_file, "");
                frames.push(b.frame(FrameSpec::new("(unknown function)").module(module).kind(kind))?);
                continue;
            };
            let mut name = self.str(f.name);
            if name.is_empty() {
                name = self.str(f.system_name);
            }
            let file = self.str(f.filename);
            let start = u32::try_from(f.start_line).ok().filter(|&l| l > 0);
            let spec = FrameSpec::new(name)
                .module(module)
                .file(Some(file))
                .line(start)
                .kind(frame_kind(mapping_file, file));
            frames.push(b.frame(spec)?);
        }
        self.location_frames[li] = Some(frames);
        Ok(Some(li))
    }
}

#[cfg(test)]
mod tests;
