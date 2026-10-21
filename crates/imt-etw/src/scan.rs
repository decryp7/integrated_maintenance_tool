//! The first pass over an `.etl` file: which processes it contains, how many
//! CPU samples each has, whether it ran .NET, and its bitness, so the user
//! (or the importer) can pick the process to convert.

use crate::clr;
use crate::kernel::{self, guids, opcodes};
use crate::RawEvent;
use serde::Serialize;
use std::collections::HashMap;

/// Most processes listed.
pub const MAX_PROCESSES: usize = 4096;

/// A process found in a trace.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EtlProcess {
    pub pid: u32,
    pub name: String,
    pub command_line: Option<String>,
    /// CPU samples on its threads.
    pub samples: u64,
    /// CLR runtime/rundown events (0: not .NET, or not recorded).
    pub clr_events: u64,
    /// 4 or 8.
    pub pointer_size: u32,
}

impl EtlProcess {
    pub fn is_dotnet(&self) -> bool {
        self.clr_events > 0
    }
}

#[derive(Debug, Default)]
struct Seen {
    name: String,
    command_line: Option<String>,
    samples: u64,
    clr_events: u64,
    clr_pointer: Option<u8>,
    wow64: bool,
}

/// Collects [`EtlProcess`]es from a stream of events.
#[derive(Debug, Default)]
pub struct Scanner {
    procs: HashMap<u32, Seen>,
    thread_pid: HashMap<u32, u32>,
    /// Events seen.
    pub events: u64,
}

impl Scanner {
    fn proc(&mut self, pid: u32) -> Option<&mut Seen> {
        if !self.procs.contains_key(&pid) && self.procs.len() >= MAX_PROCESSES {
            return None;
        }
        Some(self.procs.entry(pid).or_default())
    }

    pub fn event(&mut self, e: &RawEvent<'_>) {
        self.events += 1;
        match (e.provider, e.opcode) {
            (clr::RUNTIME_GUID | clr::RUNDOWN_GUID, _) => {
                if let Some(p) = self.proc(e.process_id) {
                    p.clr_events += 1;
                    p.clr_pointer = Some(e.pointer_size);
                }
            }
            (guids::PROCESS, opcodes::START | opcodes::DC_START) => {
                if let Some(pe) = kernel::process(e.payload, e.ptr(), e.version, 1024) {
                    if let Some(p) = self.proc(pe.process_id) {
                        p.name = pe.image;
                        p.command_line = Some(pe.command_line).filter(|c| !c.is_empty());
                    }
                }
            }
            (guids::THREAD, opcodes::START | opcodes::DC_START) => {
                if let Some((pid, tid)) = kernel::thread_group1(e.payload) {
                    if self.thread_pid.len() < 1 << 20 {
                        self.thread_pid.insert(tid, pid);
                    }
                }
            }
            (guids::IMAGE, opcodes::IMAGE_LOAD | opcodes::DC_START) => {
                if let Some(i) = kernel::image_load(e.payload, e.ptr(), 1024) {
                    if i.path.to_ascii_lowercase().contains("\\syswow64\\") {
                        if let Some(p) = self.proc(i.process_id) {
                            p.wow64 = true;
                        }
                    }
                }
            }
            (guids::PERF_INFO, opcodes::SAMPLED_PROFILE) => {
                if let Some(s) = kernel::sampled_profile(e.payload, e.ptr()) {
                    if let Some(&pid) = self.thread_pid.get(&s.thread) {
                        if let Some(p) = self.proc(pid) {
                            p.samples += 1;
                        }
                    }
                }
            }
            _ => {}
        }
    }

    /// The processes, .NET ones first, then by CPU samples (the idle process 0 is left out).
    pub fn finish(self) -> Vec<EtlProcess> {
        self.finish_in(Order::DotnetFirst)
    }

    /// The processes in `order` (the idle process 0 is left out).
    pub fn finish_in(self, order: Order) -> Vec<EtlProcess> {
        let mut out: Vec<EtlProcess> = self
            .procs
            .into_iter()
            .filter(|(pid, s)| *pid != 0 && (s.samples > 0 || s.clr_events > 0 || !s.name.is_empty()))
            .map(|(pid, s)| EtlProcess {
                pid,
                name: if s.name.is_empty() { format!("Process {pid}") } else { s.name },
                command_line: s.command_line,
                samples: s.samples,
                clr_events: s.clr_events,
                pointer_size: match s.clr_pointer {
                    Some(4) => 4,
                    Some(_) => 8,
                    None if s.wow64 => 4,
                    None => 8,
                },
            })
            .collect();
        match order {
            Order::DotnetFirst => out.sort_by_key(|p| (std::cmp::Reverse(p.is_dotnet()), std::cmp::Reverse(p.samples), p.pid)),
            Order::Samples => out.sort_by_key(|p| (std::cmp::Reverse(p.samples), p.pid)),
        }
        out
    }
}

/// How [`Scanner::finish_in`] orders the processes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Order {
    /// .NET processes first (the .NET profiler), then by CPU samples.
    DotnetFirst,
    /// By CPU samples (the native profiler: any process).
    Samples,
}

/// The process to open when the user did not choose: the .NET process with the most samples,
/// else the process with the most samples.
pub fn default_process(procs: &[EtlProcess]) -> Option<&EtlProcess> {
    procs.first()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::encode as k;

    fn ev<'a>(provider: u128, opcode: u8, pid: u32, ptr: u8, payload: &'a [u8]) -> RawEvent<'a> {
        RawEvent {
            provider,
            id: 0,
            version: 4,
            opcode,
            process_id: pid,
            thread_id: 1,
            timestamp: 0,
            pointer_size: ptr,
            payload,
            stack: &[],
        }
    }

    #[test]
    fn lists_processes_dotnet_first_with_bitness() {
        let mut s = Scanner::default();
        let p1 = k::process(10, 1, "native.exe", "native.exe -x", None, 8);
        let p2 = k::process(20, 1, "Shop.exe", "", Some(2), 8);
        let t1 = k::thread_group1(10, 100);
        let t2 = k::thread_group1(20, 200);
        let img = k::image_load(0x7700_0000, 0x1000, 20, r"C:\Windows\SysWOW64\ntdll.dll", 8);
        s.event(&ev(guids::PROCESS, opcodes::DC_START, 0, 8, &p1));
        s.event(&ev(guids::PROCESS, opcodes::START, 0, 8, &p2));
        s.event(&ev(guids::THREAD, opcodes::DC_START, 0, 8, &t1));
        s.event(&ev(guids::THREAD, opcodes::START, 0, 8, &t2));
        s.event(&ev(guids::IMAGE, opcodes::IMAGE_LOAD, 0, 8, &img));
        let sample10 = k::sampled_profile(1, 100, 8);
        let sample20 = k::sampled_profile(1, 200, 8);
        for _ in 0..5 {
            s.event(&ev(guids::PERF_INFO, opcodes::SAMPLED_PROFILE, 0, 8, &sample10));
        }
        s.event(&ev(guids::PERF_INFO, opcodes::SAMPLED_PROFILE, 0, 8, &sample20));
        s.event(&ev(clr::RUNTIME_GUID, 0, 30, 8, &[]));
        s.event(&ev(guids::PERF_INFO, opcodes::SAMPLED_PROFILE, 0, 8, &[1]));
        assert_eq!(s.events, 13);
        let list = s.finish();
        assert_eq!(list.iter().map(|p| p.pid).collect::<Vec<_>>(), [30, 10, 20]);
        assert_eq!(list[0].name, "Process 30");
        assert!(list[0].is_dotnet());
        assert_eq!(list[1].command_line.as_deref(), Some("native.exe -x"));
        assert_eq!(list[1].samples, 5);
        assert_eq!(list[2].pointer_size, 4, "SysWOW64 images mean a 32-bit process");
        assert_eq!(list[2].command_line, None);
        assert_eq!(default_process(&list).map(|p| p.pid), Some(30));
        assert!(default_process(&[]).is_none());
    }

    #[test]
    fn native_order_is_by_samples() {
        let mut s = Scanner::default();
        let t1 = k::thread_group1(10, 100);
        s.event(&ev(guids::THREAD, opcodes::DC_START, 0, 8, &t1));
        let sample = k::sampled_profile(1, 100, 8);
        s.event(&ev(guids::PERF_INFO, opcodes::SAMPLED_PROFILE, 0, 8, &sample));
        s.event(&ev(clr::RUNTIME_GUID, 0, 30, 8, &[]));
        let list = s.finish_in(Order::Samples);
        assert_eq!(list.iter().map(|p| p.pid).collect::<Vec<_>>(), [10, 30]);
    }

    #[test]
    fn process_count_is_bounded() {
        let mut s = Scanner::default();
        for pid in 1..(MAX_PROCESSES as u32 + 10) {
            s.event(&ev(clr::RUNTIME_GUID, 0, pid, 4, &[]));
        }
        let list = s.finish();
        assert_eq!(list.len(), MAX_PROCESSES);
        assert!(list.iter().all(|p| p.pointer_size == 4));
    }
}
