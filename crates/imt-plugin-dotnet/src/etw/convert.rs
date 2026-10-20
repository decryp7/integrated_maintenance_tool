//! ETW events of one process → NetTrace (ADR 0019).
//!
//! - CLR runtime and rundown events of the target keep their payloads and
//!   their extended-data stacks, under their provider names.
//! - Kernel `SampledProfile` events on the target's threads become
//!   `SampleProfiler/ThreadSample` events; their stacks come from the
//!   `StackWalk` events that follow (matched by thread and timestamp; the
//!   kernel and the deferred user part may arrive separately).
//! - Context switches become `IMT-Windows-Kernel/ThreadWait` events: the time
//!   from a thread's switch-out to its switch-in, its wait reason, the thread
//!   that readied it and the stack where it blocked.
//! - Image loads become `IMT-Windows-Kernel/ImageLoad`, thread names `ThreadName`.
//!
//! Stacks keep user-mode frames only (kernel frames, and for a 32-bit target
//! the 64-bit WOW64 frames, are dropped) so they fit the target's pointer size.

use super::kernel::{self, guids, opcodes, KERNEL_START};
use super::RawEvent;
use crate::clr;
use crate::nettrace::{EventType, NetTraceWriter, TraceHeader};
use imt_plugin_profiling::nettrace::ext;
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::{self, Write};

/// Waits shorter than this are not written (context switches are very frequent).
pub const MIN_WAIT_NS: u64 = 50_000;
/// Pending samples/waits kept per thread while their stacks arrive.
const PENDING_PER_THREAD: usize = 4;
/// Rundown `DCEndComplete`, runtime `GCEnd` and `GCBulkNode`.
const DC_END_COMPLETE: u32 = 146;
const GC_END: u32 = 2;
const GC_BULK_NODE: u32 = 18;
/// Longest name kept.
const MAX_NAME: usize = 1024;

const SAMPLE: EventType<'static> = EventType {
    provider: clr::SAMPLE_PROFILER,
    id: 0,
    version: 0,
    name: "ThreadSample",
};

fn ext_type(id: u32, name: &'static str) -> EventType<'static> {
    EventType {
        provider: ext::PROVIDER,
        id,
        version: 0,
        name,
    }
}

/// CLR events seen that matter to the recording's control flow.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Milestones {
    /// The rundown provider's `DCEndComplete` arrived (method names are complete).
    pub rundown_complete: bool,
    /// Heap-walk events (`GCBulkNode`) arrived.
    pub heap_nodes: bool,
    /// A `GCEnd` arrived after heap-walk events: the induced GC (and its walk) is over.
    pub heap_walk_done: bool,
}

/// What the conversion wrote.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Stats {
    pub samples: u64,
    pub waits: u64,
    pub clr_events: u64,
    pub images: u64,
    /// Events with a payload too short for their fields.
    pub damaged: u64,
}

#[derive(Debug, Clone)]
enum Pending {
    Sample,
    Wait {
        start: i64,
        duration: u64,
        reason: u8,
        readied_by: u32,
    },
}

#[derive(Debug, Clone)]
struct Entry {
    ts: i64,
    what: Pending,
    frames: Vec<u64>,
    has_user: bool,
}

/// Converts the events of one process into a NetTrace stream.
pub struct Converter<W: Write> {
    w: NetTraceWriter<W>,
    pid: u32,
    ptr: usize,
    min_wait_ticks: u64,
    threads: HashSet<u32>,
    pending: HashMap<u32, VecDeque<Entry>>,
    /// Thread → (switch-out time, wait reason, frames captured then).
    switched_out: HashMap<u32, (i64, u8, Vec<u64>)>,
    readied_by: HashMap<u32, u32>,
    milestones: Milestones,
    stats: Stats,
}

impl<W: Write> Converter<W> {
    /// Starts the NetTrace stream for process `header.process_id` (its pointer size in the header).
    pub fn new(out: W, header: TraceHeader) -> io::Result<Converter<W>> {
        let min_wait_ticks = u64::try_from(i128::from(header.ticks_per_second) * i128::from(MIN_WAIT_NS) / 1_000_000_000).unwrap_or(0);
        Ok(Converter {
            pid: header.process_id,
            ptr: header.pointer_size as usize,
            w: NetTraceWriter::new(out, &header)?,
            min_wait_ticks,
            threads: HashSet::new(),
            pending: HashMap::new(),
            switched_out: HashMap::new(),
            readied_by: HashMap::new(),
            milestones: Milestones::default(),
            stats: Stats::default(),
        })
    }

    /// Threads known to belong to the target before the trace says so (a live session's start).
    pub fn seed_threads(&mut self, threads: impl IntoIterator<Item = u32>) {
        self.threads.extend(threads);
    }

    pub fn milestones(&self) -> Milestones {
        self.milestones
    }

    pub fn stats(&self) -> Stats {
        self.stats
    }

    /// Bytes of NetTrace so far (about).
    pub fn bytes(&self) -> u64 {
        self.w.bytes()
    }

    /// User-mode frames that fit the target's pointer size.
    fn user_frames(&self, frames: &[u64]) -> Vec<u64> {
        frames
            .iter()
            .copied()
            .filter(|&a| a != 0 && a < KERNEL_START && (self.ptr == 8 || a <= u64::from(u32::MAX)))
            .collect()
    }

    /// Handles one event (events of other processes are ignored).
    pub fn event(&mut self, e: &RawEvent<'_>) -> io::Result<()> {
        match e.provider {
            clr::RUNTIME_GUID | clr::RUNDOWN_GUID => self.clr(e),
            guids::PERF_INFO if e.opcode == opcodes::SAMPLED_PROFILE => self.sample(e),
            guids::STACK_WALK if e.opcode == opcodes::STACK_WALK => self.stack_walk(e),
            guids::THREAD => self.thread(e),
            guids::IMAGE if matches!(e.opcode, opcodes::IMAGE_LOAD | opcodes::DC_START) => self.image(e),
            _ => Ok(()),
        }
    }

    fn clr(&mut self, e: &RawEvent<'_>) -> io::Result<()> {
        if e.process_id != self.pid {
            return Ok(());
        }
        let rundown = e.provider == clr::RUNDOWN_GUID;
        let id = u32::from(e.id);
        if rundown && id == DC_END_COMPLETE {
            self.milestones.rundown_complete = true;
        }
        if !rundown {
            match id {
                GC_BULK_NODE => self.milestones.heap_nodes = true,
                GC_END if self.milestones.heap_nodes => self.milestones.heap_walk_done = true,
                _ => {}
            }
        }
        self.threads.insert(e.thread_id);
        let stack = self.user_frames(e.stack);
        let ty = EventType {
            provider: if rundown { clr::RUNDOWN } else { clr::RUNTIME },
            id,
            version: u32::from(e.version),
            name: "",
        };
        self.stats.clr_events += 1;
        self.w.event(ty, u64::from(e.thread_id), e.timestamp, &stack, e.payload)
    }

    fn push(&mut self, thread: u32, entry: Entry) -> io::Result<()> {
        let q = self.pending.entry(thread).or_default();
        q.push_back(entry);
        if q.len() > PENDING_PER_THREAD {
            if let Some(old) = q.pop_front() {
                self.emit(thread, old)?;
            }
        }
        Ok(())
    }

    fn sample(&mut self, e: &RawEvent<'_>) -> io::Result<()> {
        let Some(s) = kernel::sampled_profile(e.payload, e.ptr()) else {
            self.stats.damaged += 1;
            return Ok(());
        };
        if !self.threads.contains(&s.thread) {
            return Ok(());
        }
        let user = s.ip < KERNEL_START;
        // The sampled instruction is the leaf when the stack walk has not arrived (or is missing).
        let entry = Entry {
            ts: e.timestamp,
            what: Pending::Sample,
            frames: if user { self.user_frames(&[s.ip]) } else { Vec::new() },
            has_user: false,
        };
        self.push(s.thread, entry)
    }

    fn stack_walk(&mut self, e: &RawEvent<'_>) -> io::Result<()> {
        let Some(w) = kernel::stack_walk(e.payload, e.ptr()) else {
            self.stats.damaged += 1;
            return Ok(());
        };
        if w.process_id != self.pid && !self.threads.contains(&w.thread) {
            return Ok(());
        }
        let frames = self.user_frames(&w.frames);
        if let Some(entry) = self
            .pending
            .get_mut(&w.thread)
            .and_then(|q| q.iter_mut().rev().find(|x| x.ts == w.event_ts))
        {
            if !entry.has_user && !frames.is_empty() {
                // The walk includes the sampled instruction; replace the provisional leaf.
                entry.frames = frames;
                entry.has_user = true;
            }
            return Ok(());
        }
        if let Some((ts, _, f)) = self.switched_out.get_mut(&w.thread) {
            if *ts == w.event_ts && f.is_empty() {
                *f = frames;
            }
        }
        Ok(())
    }

    fn thread(&mut self, e: &RawEvent<'_>) -> io::Result<()> {
        match e.opcode {
            opcodes::START | opcodes::DC_START => match kernel::thread_group1(e.payload) {
                Some((pid, tid)) if pid == self.pid => {
                    self.threads.insert(tid);
                }
                Some(_) => {}
                None => self.stats.damaged += 1,
            },
            opcodes::END => {
                if let Some((pid, tid)) = kernel::thread_group1(e.payload) {
                    if pid == self.pid {
                        self.flush_thread(tid)?;
                        self.threads.remove(&tid);
                    }
                }
            }
            opcodes::CSWITCH => return self.cswitch(e),
            opcodes::READY_THREAD => {
                if let Some(tid) = kernel::ready_thread(e.payload) {
                    if self.threads.contains(&tid) {
                        self.readied_by.insert(tid, e.thread_id);
                    }
                }
            }
            opcodes::THREAD_SET_NAME => {
                if let Some((pid, tid, name)) = kernel::thread_set_name(e.payload, MAX_NAME) {
                    if pid == self.pid && !name.is_empty() {
                        self.w.event(
                            ext_type(ext::ids::THREAD_NAME, "ThreadName"),
                            u64::from(tid),
                            e.timestamp,
                            &[],
                            &ext::thread_name(&name),
                        )?;
                    }
                }
            }
            _ => {}
        }
        Ok(())
    }

    fn cswitch(&mut self, e: &RawEvent<'_>) -> io::Result<()> {
        let Some(c) = kernel::cswitch(e.payload) else {
            self.stats.damaged += 1;
            return Ok(());
        };
        if self.threads.contains(&c.old_thread) {
            self.switched_out.insert(c.old_thread, (e.timestamp, c.old_wait_reason, Vec::new()));
        }
        if self.threads.contains(&c.new_thread) {
            if let Some((start, reason, frames)) = self.switched_out.remove(&c.new_thread) {
                let duration = e.timestamp.saturating_sub(start).max(0) as u64;
                let readied_by = self.readied_by.remove(&c.new_thread).unwrap_or(0);
                if duration >= self.min_wait_ticks {
                    let has_user = !frames.is_empty();
                    self.push(
                        c.new_thread,
                        Entry {
                            ts: e.timestamp,
                            what: Pending::Wait {
                                start,
                                duration,
                                reason,
                                readied_by,
                            },
                            frames,
                            has_user,
                        },
                    )?;
                }
            }
        }
        Ok(())
    }

    fn image(&mut self, e: &RawEvent<'_>) -> io::Result<()> {
        let Some(i) = kernel::image_load(e.payload, e.ptr(), MAX_NAME) else {
            self.stats.damaged += 1;
            return Ok(());
        };
        if i.process_id != self.pid {
            return Ok(());
        }
        self.stats.images += 1;
        let payload = ext::image_load(&ext::ImageLoad {
            base: i.base,
            size: i.size,
            path: i.path,
        });
        self.w.event(
            ext_type(ext::ids::IMAGE_LOAD, "ImageLoad"),
            u64::from(e.thread_id),
            e.timestamp,
            &[],
            &payload,
        )
    }

    fn emit(&mut self, thread: u32, entry: Entry) -> io::Result<()> {
        match entry.what {
            Pending::Sample => {
                self.stats.samples += 1;
                // 1 = external (native) code, 2 = managed: unknown here; the importer does not use it.
                self.w
                    .event(SAMPLE, u64::from(thread), entry.ts, &entry.frames, &1u32.to_le_bytes())
            }
            Pending::Wait {
                start,
                duration,
                reason,
                readied_by,
            } => {
                self.stats.waits += 1;
                let payload = ext::thread_wait(&ext::ThreadWait {
                    duration,
                    reason,
                    readied_by,
                });
                self.w.event(
                    ext_type(ext::ids::THREAD_WAIT, "ThreadWait"),
                    u64::from(thread),
                    start,
                    &entry.frames,
                    &payload,
                )
            }
        }
    }

    fn flush_thread(&mut self, thread: u32) -> io::Result<()> {
        if let Some(q) = self.pending.remove(&thread) {
            for entry in q {
                self.emit(thread, entry)?;
            }
        }
        Ok(())
    }

    /// Writes what is still pending and ends the stream.
    pub fn finish(mut self) -> io::Result<(W, Stats)> {
        let mut threads: Vec<u32> = self.pending.keys().copied().collect();
        threads.sort_unstable();
        for t in threads {
            self.flush_thread(t)?;
        }
        let stats = self.stats;
        Ok((self.w.finish()?, stats))
    }
}

#[cfg(test)]
mod tests;
