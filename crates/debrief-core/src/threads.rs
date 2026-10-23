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

impl Hardware {
    /// The machine this runs on (RAM is unknown here and taken as 8 GiB; callers that know it build their own).
    pub fn detect() -> Self {
        Hardware {
            cores: std::thread::available_parallelism().map_or(1, |n| n.get()),
            ram_mb: 8 * 1024,
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

    #[test]
    fn lowering_the_priority_of_a_throwaway_thread_does_not_panic() {
        let accepted = std::thread::spawn(lower_priority).join().unwrap();
        if cfg!(any(target_os = "macos", target_os = "linux", windows)) {
            assert!(accepted);
        }
    }
}
