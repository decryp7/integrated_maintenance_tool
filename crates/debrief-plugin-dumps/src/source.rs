//! Getting a minidump into memory without reading all of it. A minidump keeps everything addressed by 32-bit
//! offsets (directory, thread list, contexts, modules, names, handles, memory info) before the bulk memory of a
//! full dump (`Memory64ListStream`, 64-bit offsets, at the end). So a full dump is read as that prefix plus only the
//! memory the stack walks need (each thread's stack from its stack pointer up), with the `Memory64List`
//! rewritten to describe what was kept. Other dumps are read whole, up to a limit.

use debrief_dump::{DumpError, Limits, Result};

/// A file read by ranges (the host lends it; tests use byte slices).
pub trait Source {
    fn len(&self) -> u64;
    /// Up to `len` bytes at `offset`; fewer only at the end of the file (callers loop over short reads).
    fn read(&self, offset: u64, len: usize) -> std::result::Result<Vec<u8>, String>;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Source for &[u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    fn read(&self, offset: u64, len: usize) -> std::result::Result<Vec<u8>, String> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX).min(<[u8]>::len(self));
        let end = start.saturating_add(len).min(<[u8]>::len(self));
        Ok(self[start..end].to_vec())
    }
}

impl Source for Vec<u8> {
    fn len(&self) -> u64 {
        self.as_slice().len() as u64
    }

    fn read(&self, offset: u64, len: usize) -> std::result::Result<Vec<u8>, String> {
        Source::read(&self.as_slice(), offset, len)
    }
}

/// Largest single read asked of a source.
const CHUNK: usize = 1 << 20;
/// Streams a minidump directory may list.
const MAX_STREAMS: u32 = 4096;
/// Memory ranges of a full dump's `Memory64List`.
const MAX_RANGES: u64 = 4 << 20;
/// Largest prefix (everything before the bulk memory) read from a full dump.
const MAX_PREFIX: u64 = 512 << 20;
/// Stack bytes kept per thread of a full dump.
const STACK_KEPT: u64 = 1 << 20;
/// Memory kept from a full dump in all.
const MAX_KEPT: u64 = 256 << 20;

const THREAD_LIST: u32 = 3;
const MEMORY64_LIST: u32 = 9;

/// Exactly `len` bytes at `offset`, or an error naming `what`.
pub fn read_exact(src: &dyn Source, offset: u64, len: u64, what: &str) -> Result<Vec<u8>> {
    let end = offset
        .checked_add(len)
        .filter(|e| *e <= src.len())
        .ok_or_else(|| DumpError::invalid(format!("the dump is cut off: {what} lies beyond its end")))?;
    let mut out = Vec::with_capacity(usize::try_from(len).map_err(|_| DumpError::limit("too large"))?);
    let mut at = offset;
    while at < end {
        let want = usize::try_from(end - at).unwrap_or(usize::MAX).min(CHUNK);
        let chunk = src.read(at, want).map_err(|e| DumpError::new(debrief_dump::ErrorCode::Io, e))?;
        if chunk.is_empty() {
            return Err(DumpError::invalid(format!("the dump ended early while reading {what}")));
        }
        at += chunk.len() as u64;
        out.extend_from_slice(&chunk);
    }
    out.truncate(usize::try_from(len).unwrap_or(usize::MAX));
    Ok(out)
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    b.get(at..at.checked_add(4)?).map(|s| u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    let s = b.get(at..at.checked_add(8)?)?;
    let mut a = [0u8; 8];
    a.copy_from_slice(s);
    Some(u64::from_le_bytes(a))
}

/// A directory entry: stream type, size, offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    kind: u32,
    size: u32,
    rva: u32,
}

/// What [`load`] produced.
#[derive(Debug)]
pub struct Loaded {
    pub bytes: Vec<u8>,
    /// The dump has a `Memory64List` (all memory: a full dump).
    pub full: bool,
    pub notes: Vec<String>,
}

/// Reads the minidump in `src` as described in the module docs.
pub fn load(src: &dyn Source, limits: &Limits) -> Result<Loaded> {
    if src.len() < 32 {
        return Err(DumpError::new(
            debrief_dump::ErrorCode::NotADump,
            "not a minidump: the file is too short",
        ));
    }
    let header = read_exact(src, 0, 32, "the header")?;
    if &header[..4] != b"MDMP" {
        return Err(DumpError::new(
            debrief_dump::ErrorCode::NotADump,
            "not a minidump (no MDMP signature)",
        ));
    }
    let count = u32_at(&header, 8).unwrap_or(0);
    let dir_rva = u32_at(&header, 12).unwrap_or(0);
    if count > MAX_STREAMS {
        return Err(DumpError::invalid(format!(
            "the dump lists {count} streams; at most {MAX_STREAMS} are read"
        )));
    }
    let dir = read_exact(src, u64::from(dir_rva), u64::from(count) * 12, "the stream directory")?;
    let entries: Vec<Entry> = dir
        .as_chunks::<12>()
        .0
        .iter()
        .filter_map(|e| {
            Some(Entry {
                kind: u32_at(e, 0)?,
                size: u32_at(e, 4)?,
                rva: u32_at(e, 8)?,
            })
        })
        .collect();
    match entries.iter().position(|e| e.kind == MEMORY64_LIST) {
        Some(i) => compact(src, &entries, i, dir_rva),
        None => {
            if src.len() > limits.max_read_bytes {
                return Err(DumpError::limit(format!(
                    "the dump is {} MB; dumps without a full-memory list are read whole, up to {} MB",
                    src.len() >> 20,
                    limits.max_read_bytes >> 20
                )));
            }
            Ok(Loaded {
                bytes: read_exact(src, 0, src.len(), "the dump")?,
                full: false,
                notes: Vec::new(),
            })
        }
    }
}

/// A memory range to keep: its start address and length.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct Range {
    start: u64,
    len: u64,
}

/// Each thread's stack from its recorded start (the stack pointer's side) up to [`STACK_KEPT`] bytes.
fn stack_ranges(prefix: &[u8], threads: Option<&Entry>) -> Vec<Range> {
    let Some(t) = threads else { return Vec::new() };
    let start = t.rva as usize;
    let Some(list) = prefix.get(start..start.saturating_add(t.size as usize)) else {
        return Vec::new();
    };
    let n = u32_at(list, 0).unwrap_or(0) as usize;
    let mut out: Vec<Range> = (0..n.min(list.len() / 48))
        .filter_map(|i| {
            let at = 4 + i * 48;
            let base = u64_at(list, at + 24)?;
            let size = u64::from(u32_at(list, at + 32)?);
            (size > 0).then(|| Range {
                start: base,
                len: size.min(STACK_KEPT),
            })
        })
        .collect();
    out.sort();
    // Merge overlapping ranges so the rewritten list stays sorted and disjoint.
    let mut merged: Vec<Range> = Vec::with_capacity(out.len());
    for r in out {
        match merged.last_mut() {
            Some(m) if r.start <= m.start.saturating_add(m.len) => {
                let end = (r.start.saturating_add(r.len)).max(m.start.saturating_add(m.len));
                m.len = end - m.start;
            }
            _ => merged.push(r),
        }
    }
    merged
}

/// Threads whose stack or context points into the bulk memory (not read) would read the rewritten data at that
/// offset instead: their stack is unlinked (null RVA: the reader finds it by address in the kept memory), their
/// context dropped (noted).
fn unlink_bulk(prefix: &mut [u8], t: &Entry, prefix_end: u64, notes: &mut Vec<String>) {
    let start = t.rva as usize;
    let Some(n) = u32_at(prefix, start).map(|n| n as usize) else {
        return;
    };
    let mut lost = 0;
    for i in 0..n.min((t.size as usize).saturating_sub(4) / 48) {
        let at = start + 4 + i * 48;
        for (size_at, rva_at, context) in [(at + 32, at + 36, false), (at + 40, at + 44, true)] {
            let (Some(size), Some(rva)) = (u32_at(prefix, size_at), u32_at(prefix, rva_at)) else {
                continue;
            };
            if u64::from(rva) + u64::from(size) > prefix_end {
                prefix[rva_at..rva_at + 4].copy_from_slice(&0u32.to_le_bytes());
                if context {
                    prefix[size_at..size_at + 4].copy_from_slice(&0u32.to_le_bytes());
                    lost += 1;
                }
            }
        }
    }
    if lost > 0 {
        notes.push(format!(
            "{lost} thread context(s) lie inside the dump's memory data and were not read."
        ));
    }
}

fn compact(src: &dyn Source, entries: &[Entry], m64: usize, dir_rva: u32) -> Result<Loaded> {
    let e = entries[m64];
    let head = read_exact(src, u64::from(e.rva), 16, "the full-memory list")?;
    let ranges = u64_at(&head, 0).unwrap_or(0);
    let base_rva = u64_at(&head, 8).unwrap_or(0);
    if ranges > MAX_RANGES || u64::from(e.size) < 16 + ranges.saturating_mul(16) {
        return Err(DumpError::invalid("the full-memory list is damaged"));
    }
    let descs = read_exact(src, u64::from(e.rva) + 16, ranges * 16, "the full-memory list")?;
    let prefix_end = base_rva.min(src.len());
    if prefix_end > MAX_PREFIX {
        return Err(DumpError::limit(format!(
            "the dump's streams before its memory take {} MB; at most {} MB are read",
            prefix_end >> 20,
            MAX_PREFIX >> 20
        )));
    }
    let mut notes = Vec::new();
    let mut prefix = read_exact(src, 0, prefix_end, "the dump's streams")?;
    let dir_end = u64::from(dir_rva) + entries.len() as u64 * 12;
    if dir_end > prefix_end {
        return Err(DumpError::invalid("the stream directory lies inside the dump's memory"));
    }
    if entries
        .iter()
        .enumerate()
        .any(|(i, x)| i != m64 && u64::from(x.rva) + u64::from(x.size) > prefix_end)
    {
        notes.push("Some streams lie inside the dump's memory data and were not read.".into());
    }
    // Where each original range's bytes are in the file.
    let mut file_at = base_rva;
    let mut original: Vec<(u64, u64, u64)> = Vec::with_capacity(ranges as usize);
    for d in descs.as_chunks::<16>().0 {
        let (start, len) = (u64_at(d, 0).unwrap_or(0), u64_at(d, 8).unwrap_or(0));
        original.push((start, len, file_at));
        file_at = file_at.saturating_add(len);
    }
    let threads = entries.iter().find(|x| x.kind == THREAD_LIST).copied();
    let wanted = stack_ranges(&prefix, threads.as_ref());
    if let Some(t) = threads {
        unlink_bulk(&mut prefix, &t, prefix_end, &mut notes);
    }
    let mut kept: Vec<(u64, Vec<u8>)> = Vec::new();
    let mut total = 0u64;
    for w in wanted {
        for &(start, len, at) in &original {
            let lo = w.start.max(start);
            let hi = w.start.saturating_add(w.len).min(start.saturating_add(len));
            if lo >= hi {
                continue;
            }
            if total + (hi - lo) > MAX_KEPT {
                notes.push("Stack memory beyond the reading limit was left out; some stacks are shorter.".into());
                break;
            }
            match read_exact(src, at + (lo - start), hi - lo, "stack memory") {
                Ok(bytes) => {
                    total += bytes.len() as u64;
                    kept.push((lo, bytes));
                }
                Err(_) => notes.push(format!("The stack memory at {lo:#x} lies beyond the end of the file.")),
            }
        }
    }
    kept.sort_by_key(|k| k.0);
    // The rewritten Memory64List goes after the prefix, its data right after it.
    let list_rva = prefix.len() as u64;
    let list_size = 16 + kept.len() as u64 * 16;
    if list_rva > u64::from(u32::MAX) || list_size > u64::from(u32::MAX) {
        return Err(DumpError::limit("the dump's streams are too large to rewrite"));
    }
    prefix.extend_from_slice(&(kept.len() as u64).to_le_bytes());
    prefix.extend_from_slice(&(list_rva + list_size).to_le_bytes());
    for (start, bytes) in &kept {
        prefix.extend_from_slice(&start.to_le_bytes());
        prefix.extend_from_slice(&(bytes.len() as u64).to_le_bytes());
    }
    for (_, bytes) in &kept {
        prefix.extend_from_slice(bytes);
    }
    let at = dir_rva as usize + m64 * 12;
    prefix[at + 4..at + 8].copy_from_slice(&(list_size as u32).to_le_bytes());
    prefix[at + 8..at + 12].copy_from_slice(&(list_rva as u32).to_le_bytes());
    Ok(Loaded {
        bytes: prefix,
        full: true,
        notes,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::synth::*;

    #[test]
    fn small_dumps_are_read_whole_and_large_ones_refused() {
        let bytes = sample_crash();
        let l = load(&bytes, &Limits::default()).unwrap();
        assert!(!l.full && l.bytes == bytes && l.notes.is_empty());
        let tiny = Limits {
            max_read_bytes: 10,
            ..Limits::default()
        };
        assert_eq!(load(&bytes, &tiny).unwrap_err().code, debrief_dump::ErrorCode::Limit);
        assert_eq!(
            load(&b"MDMP".to_vec(), &Limits::default()).unwrap_err().code,
            debrief_dump::ErrorCode::NotADump
        );
        let mut not = bytes.clone();
        not[0] = b'X';
        assert_eq!(load(&not, &Limits::default()).unwrap_err().code, debrief_dump::ErrorCode::NotADump);
        let mut cut = bytes.clone();
        cut.truncate(40);
        assert!(load(&cut, &Limits::default()).is_err());
        let mut many = bytes;
        many[8..12].copy_from_slice(&100_000u32.to_le_bytes());
        assert!(load(&many, &Limits::default()).unwrap_err().message.contains("streams"));
    }

    #[test]
    fn full_dumps_keep_only_the_stacks() {
        let full = sample_full(64 << 10);
        let l = load(&full, &Limits::default()).unwrap();
        assert!(l.full);
        assert!(l.bytes.len() < full.len() / 2, "{} of {}", l.bytes.len(), full.len());
        // The rewritten dump still parses and has the stack memory.
        let dump = minidump::Minidump::read(l.bytes.as_slice()).unwrap();
        let mem = dump.get_memory().unwrap();
        let threads = dump.get_stream::<minidump::MinidumpThreadList>().unwrap();
        let t = &threads.threads[0];
        let stack = t.stack_memory(&mem).unwrap();
        assert_eq!(stack.base_address(), STACK_BASE);
        assert!(stack.bytes().len() >= 64);
    }

    #[test]
    fn damaged_full_memory_lists_are_refused() {
        let full = sample_full(4096);
        let dump = minidump::Minidump::read(full.as_slice()).unwrap();
        let e = dump.all_streams().find(|e| e.stream_type == super::MEMORY64_LIST).unwrap();
        let at = e.location.rva as usize;
        let mut bad = full.clone();
        bad[at..at + 8].copy_from_slice(&u64::MAX.to_le_bytes());
        assert!(load(&bad, &Limits::default()).is_err());
        let mut beyond = full.clone();
        beyond[at + 8..at + 16].copy_from_slice(&(u64::from(u32::MAX) * 4).to_le_bytes());
        // The memory starts beyond the file: the prefix is the whole file and stacks are missing (noted).
        let l = load(&beyond, &Limits::default()).unwrap();
        assert!(l.notes.iter().any(|n| n.contains("beyond")), "{:?}", l.notes);
    }

    #[test]
    fn exact_reads_loop_and_fail_on_short_files() {
        let v: Vec<u8> = (0..=255).collect();
        assert_eq!(read_exact(&v, 250, 6, "x").unwrap(), [250, 251, 252, 253, 254, 255]);
        assert!(read_exact(&v, 250, 7, "x").is_err());
        assert!(read_exact(&v, u64::MAX, 2, "x").is_err());
    }
}
