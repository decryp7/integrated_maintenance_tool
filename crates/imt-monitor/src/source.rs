//! Where process measurements come from. The tracker depends on [`ProcessSource`] only,
//! so tests inject scripted samples; [`SysinfoSource`] is the real adapter.

use crate::config::Targets;
use std::collections::{HashMap, HashSet};
use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

/// One measurement of one process.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ProcSample {
    pub pid: u32,
    pub name: String,
    pub exe: Option<String>,
    /// CPU in "per core" units (200 = two cores busy). `None` until a baseline exists.
    pub cpu_core_percent: Option<f32>,
    pub working_set: u64,
    pub virtual_bytes: u64,
    pub private_bytes: Option<u64>,
    pub threads: Option<u32>,
    pub handles: Option<u32>,
    /// Bytes read / written since the previous sample.
    pub read_bytes: Option<u64>,
    pub write_bytes: Option<u64>,
    pub uptime_sec: u64,
    /// The process exists but its details could not be read.
    pub access_denied: bool,
}

/// Port: yields a sample for every monitored process currently alive.
pub trait ProcessSource {
    /// Logical CPU count, used to normalise CPU to the whole machine.
    fn cpu_count(&self) -> usize;
    /// Name of this machine.
    fn host_name(&self) -> String;
    fn sample(&mut self, targets: &Targets, include_children: bool) -> Vec<ProcSample>;
}

/// Adds descendants of matched processes (`parents` maps pid to parent pid) until stable.
pub fn expand_children(matched: &mut HashSet<u32>, parents: &HashMap<u32, u32>) {
    loop {
        let before = matched.len();
        for (pid, parent) in parents {
            if matched.contains(parent) {
                matched.insert(*pid);
            }
        }
        if matched.len() == before {
            return;
        }
    }
}

/// [`ProcessSource`] backed by `sysinfo`; refreshes only matched processes in detail.
pub struct SysinfoSource {
    sys: System,
    /// PIDs measured before: CPU and IO deltas are meaningless on the first sight.
    seen: HashSet<u32>,
}

impl Default for SysinfoSource {
    fn default() -> Self {
        Self::new()
    }
}

impl SysinfoSource {
    pub fn new() -> Self {
        SysinfoSource {
            sys: System::new(),
            seen: HashSet::new(),
        }
    }

    fn matched_pids(&mut self, targets: &Targets, include_children: bool) -> Vec<Pid> {
        if let (Some(pids), false) = (targets.only_pids(), include_children) {
            let pids: Vec<Pid> = pids.into_iter().map(Pid::from_u32).collect();
            self.sys.refresh_processes_specifics(
                ProcessesToUpdate::Some(&pids),
                true,
                ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
            );
            return pids.into_iter().filter(|p| self.sys.process(*p).is_some()).collect();
        }
        self.sys.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet),
        );
        let mut matched: HashSet<u32> = HashSet::new();
        let mut parents = HashMap::new();
        for (pid, p) in self.sys.processes() {
            let exe = p.exe().map(|e| e.to_string_lossy());
            if targets.matches(pid.as_u32(), &p.name().to_string_lossy(), exe.as_deref()) {
                matched.insert(pid.as_u32());
            }
            if let Some(parent) = p.parent() {
                parents.insert(pid.as_u32(), parent.as_u32());
            }
        }
        if include_children {
            expand_children(&mut matched, &parents);
        }
        matched.into_iter().map(Pid::from_u32).collect()
    }
}

impl ProcessSource for SysinfoSource {
    fn cpu_count(&self) -> usize {
        std::thread::available_parallelism().map(|n| n.get()).unwrap_or(1)
    }

    fn host_name(&self) -> String {
        System::host_name().unwrap_or_else(|| "localhost".into())
    }

    fn sample(&mut self, targets: &Targets, include_children: bool) -> Vec<ProcSample> {
        let pids = self.matched_pids(targets, include_children);
        let kind = ProcessRefreshKind::nothing()
            .with_cpu()
            .with_memory()
            .with_disk_usage()
            .with_exe(UpdateKind::OnlyIfNotSet);
        self.sys.refresh_processes_specifics(ProcessesToUpdate::Some(&pids), true, kind);
        let mut out = Vec::new();
        let mut alive = HashSet::new();
        for pid in pids {
            let Some(p) = self.sys.process(pid) else { continue };
            let id = pid.as_u32();
            alive.insert(id);
            let known = self.seen.contains(&id);
            let exe = p.exe().map(|e| e.to_string_lossy().into_owned());
            let disk = p.disk_usage();
            out.push(ProcSample {
                pid: id,
                name: p.name().to_string_lossy().into_owned(),
                access_denied: exe.is_none() && p.memory() == 0,
                exe,
                cpu_core_percent: known.then(|| p.cpu_usage()),
                working_set: p.memory(),
                virtual_bytes: p.virtual_memory(),
                private_bytes: None,
                threads: p.tasks().map(|t| t.len() as u32),
                handles: None,
                read_bytes: known.then_some(disk.read_bytes),
                write_bytes: known.then_some(disk.written_bytes),
                uptime_sec: p.run_time(),
            });
        }
        self.seen = alive;
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn children_are_found_transitively() {
        let parents = HashMap::from([(2, 1), (3, 2), (4, 99), (5, 4)]);
        let mut m = HashSet::from([1]);
        expand_children(&mut m, &parents);
        assert_eq!(m, HashSet::from([1, 2, 3]));
    }
}
