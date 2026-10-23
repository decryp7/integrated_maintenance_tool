//! How many threads each kind of background work uses, and how it is kept out of the user's way.
//!
//! One small helper for import, indexing, mining and downloads instead of each place picking a number:
//!
//! | kind      | threads                                                       |
//! |-----------|---------------------------------------------------------------|
//! | `Cpu`     | cores - 1 (at least 1), or the Performance setting when given |
//! | `SsdIo`   | cores, clamped to 4..=8                                       |
//! | `HddIo`   | 1 on a spinning disk (seeks), 2 at most                       |
//! | `Network` | a few (4), never more than the cores                          |
//! | `Memory`  | one per 1 GiB of RAM beyond a 1 GiB reserve, at most 8 and the cores |
//!
//! Pure functions of a [`Hardware`] description, so they are unit-testable; [`Hardware::detect`] reads the machine.
//!
//! Background priority ([`lower_priority`]), where the OS allows it: macOS puts the thread in the QoS class
//! `BACKGROUND` (`pthread_set_qos_class_self_np`; efficiency cores, throttled IO); Linux raises the thread's nice
//! value to 10 (`setpriority` on the thread id); Windows sets `THREAD_PRIORITY_BELOW_NORMAL`. Where the call fails the
//! thread simply keeps its priority.

/// What limits a kind of work.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkKind {
    /// Computation: parsing, tokenizing, mining.
    Cpu,
    /// Reading or writing many files on a solid-state disk.
    SsdIo,
    /// The same on a spinning disk.
    HddIo,
    /// Connections to servers.
    Network,
    /// Work whose threads each hold a lot of memory.
    Memory,
}

/// The machine, as far as sizing needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Hardware {
    pub cores: usize,
    pub ram_mb: u64,
}

/// Where the installed RAM comes from; a port so sizing is testable without the real machine.
pub trait MemoryProbe {
    /// Installed physical memory in MiB, or `None` when the OS will not say.
    fn total_ram_mb(&self) -> Option<u64>;
}

/// The operating system's answer (sysinfo: `hw.memsize` on macOS, `/proc/meminfo` on Linux, `GlobalMemoryStatusEx` on Windows).
#[derive(Debug, Clone, Copy, Default)]
pub struct SystemMemory;

impl MemoryProbe for SystemMemory {
    fn total_ram_mb(&self) -> Option<u64> {
        let mut sys = sysinfo::System::new();
        sys.refresh_memory();
        let bytes = sys.total_memory();
        (bytes > 0).then_some(bytes / (1024 * 1024))
    }
}

/// RAM assumed when the OS does not report it.
pub const FALLBACK_RAM_MB: u64 = 8 * 1024;

impl Hardware {
    /// The machine this runs on.
    pub fn detect() -> Self {
        Self::detect_with(&SystemMemory)
    }

    /// Like [`Hardware::detect`] with the RAM read from `memory` (8 GiB when it cannot say).
    pub fn detect_with(memory: &dyn MemoryProbe) -> Self {
        Hardware {
            cores: std::thread::available_parallelism().map_or(1, |n| n.get()),
            ram_mb: memory.total_ram_mb().unwrap_or(FALLBACK_RAM_MB),
        }
    }
}

/// Upper bound of the default solid-state IO threads.
pub const MAX_SSD_THREADS: usize = 8;

/// Threads for `kind` on `hw`. `setting` is the user's Performance choice for CPU work (0 or none = automatic).
pub fn threads_for(kind: WorkKind, hw: Hardware, setting: Option<usize>) -> usize {
    let cores = hw.cores.max(1);
    let n = match kind {
        WorkKind::Cpu => match setting.filter(|&s| s > 0) {
            Some(s) => s,
            None => cores.saturating_sub(1),
        },
        WorkKind::SsdIo => cores.clamp(4, MAX_SSD_THREADS),
        WorkKind::HddIo => cores.min(2),
        WorkKind::Network => cores.min(4),
        WorkKind::Memory => usize::try_from(hw.ram_mb.saturating_sub(1024) / 1024)
            .unwrap_or(usize::MAX)
            .min(8)
            .min(cores),
    };
    n.max(1)
}

/// [`threads_for`] on this machine.
pub fn threads(kind: WorkKind, setting: Option<usize>) -> usize {
    threads_for(kind, Hardware::detect(), setting)
}

/// Lowers the calling thread's priority so background work yields to the user (see the module docs per OS).
/// Returns whether the OS accepted it.
pub fn lower_priority() -> bool {
    imp::lower_priority()
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn lower_priority() -> bool {
        // SAFETY: a plain libc call that affects only the calling thread.
        unsafe { libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_BACKGROUND, 0) == 0 }
    }
}

#[cfg(target_os = "linux")]
mod imp {
    pub fn lower_priority() -> bool {
        // SAFETY: gettid and setpriority take and return plain integers; the id is this thread's own.
        unsafe {
            let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
            libc::setpriority(libc::PRIO_PROCESS, tid, 10) == 0
        }
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_PRIORITY_BELOW_NORMAL};
    pub fn lower_priority() -> bool {
        // SAFETY: the pseudo handle of the current thread is always valid.
        unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_BELOW_NORMAL).is_ok() }
    }
}

#[cfg(not(any(target_os = "macos", target_os = "linux", windows)))]
mod imp {
    pub fn lower_priority() -> bool {
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hw(cores: usize, ram_mb: u64) -> Hardware {
        Hardware { cores, ram_mb }
    }

    #[test]
    fn each_kind_is_sized_from_the_machine() {
        assert_eq!(threads_for(WorkKind::Cpu, hw(8, 16384), None), 7);
        assert_eq!(threads_for(WorkKind::Cpu, hw(1, 16384), None), 1);
        assert_eq!(threads_for(WorkKind::Cpu, hw(8, 16384), Some(3)), 3, "the setting wins");
        assert_eq!(threads_for(WorkKind::Cpu, hw(8, 16384), Some(0)), 7, "0 is automatic");
        assert_eq!(threads_for(WorkKind::SsdIo, hw(2, 16384), None), 4);
        assert_eq!(threads_for(WorkKind::SsdIo, hw(32, 16384), None), 8);
        assert_eq!(threads_for(WorkKind::HddIo, hw(32, 16384), None), 2);
        assert_eq!(threads_for(WorkKind::HddIo, hw(1, 16384), None), 1);
        assert_eq!(threads_for(WorkKind::Network, hw(32, 16384), None), 4);
        assert_eq!(threads_for(WorkKind::Network, hw(2, 16384), None), 2);
        assert_eq!(threads_for(WorkKind::Memory, hw(32, 4096), None), 3);
        assert_eq!(threads_for(WorkKind::Memory, hw(32, 1024), None), 1, "never zero");
        assert_eq!(threads_for(WorkKind::Memory, hw(32, 1 << 20), None), 8);
        assert_eq!(threads_for(WorkKind::Memory, hw(2, 1 << 20), None), 2);
        assert_eq!(threads_for(WorkKind::Cpu, hw(0, 0), None), 1);
        assert!(threads(WorkKind::Cpu, None) >= 1);
    }

    struct Fake(Option<u64>);
    impl MemoryProbe for Fake {
        fn total_ram_mb(&self) -> Option<u64> {
            self.0
        }
    }

    #[test]
    fn detect_uses_the_probed_ram_and_falls_back_when_unknown() {
        assert_eq!(Hardware::detect_with(&Fake(Some(32768))).ram_mb, 32768);
        assert_eq!(Hardware::detect_with(&Fake(None)).ram_mb, FALLBACK_RAM_MB);
        assert!(Hardware::detect_with(&Fake(Some(1))).cores >= 1);
    }

    #[test]
    fn the_memory_kind_follows_the_probed_ram() {
        let small = Hardware {
            cores: 32,
            ..Hardware::detect_with(&Fake(Some(2048)))
        };
        let big = Hardware {
            cores: 32,
            ..Hardware::detect_with(&Fake(Some(16384)))
        };
        assert_eq!(threads_for(WorkKind::Memory, small, None), 1);
        assert_eq!(threads_for(WorkKind::Memory, big, None), 8);
    }

    #[test]
    fn the_real_machine_reports_some_ram() {
        let mb = SystemMemory.total_ram_mb().expect("this host reports its RAM");
        assert!(mb >= 256, "{mb}");
    }

    #[test]
    fn lowering_the_priority_of_a_throwaway_thread_does_not_panic() {
        let accepted = std::thread::spawn(lower_priority).join().unwrap();
        if cfg!(any(target_os = "macos", target_os = "linux", windows)) {
            assert!(accepted);
        }
    }
}
