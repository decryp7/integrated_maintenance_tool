//! Windows: put the worker in a job object that kills it when IMT exits or crashes, so no worker is ever
//! left behind, and that caps its committed memory when asked to (a worker past the cap fails its
//! allocations and stops; the supervisor restarts it). (On Unix the worker exits when its stdin closes.)

use std::os::windows::io::AsRawHandle;
use std::process::Child;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectCpuRateControlInformation, JobObjectExtendedLimitInformation,
    SetInformationJobObject, JOBOBJECT_CPU_RATE_CONTROL_INFORMATION, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_CPU_RATE_CONTROL_ENABLE, JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP, JOB_OBJECT_LIMIT_ACTIVE_PROCESS,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY, JOB_OBJECT_LIMIT_PROCESS_TIME,
};

/// The limits of a child's job object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct JobLimits {
    pub memory: Option<u64>,
    pub cpu_rate_percent: Option<u8>,
    pub cpu_seconds: Option<u64>,
    /// No processes besides the child itself.
    pub no_children: bool,
}

impl JobLimits {
    pub fn of(spec: &crate::process::SpawnSpec) -> JobLimits {
        JobLimits::from_sandbox(&spec.sandbox)
    }

    pub fn from_sandbox(s: &crate::sandbox::SandboxSpec) -> JobLimits {
        JobLimits {
            memory: s.memory_limit,
            cpu_rate_percent: s.cpu_rate_percent,
            cpu_seconds: s.cpu_seconds,
            no_children: !s.children,
        }
    }
}

pub struct JobGuard(HANDLE);

// SAFETY: a job handle may be used and closed from any thread.
unsafe impl Send for JobGuard {}
// SAFETY: the guard is never used through a shared reference (it only closes the handle in `Drop`).
unsafe impl Sync for JobGuard {}

impl JobGuard {
    /// Best effort: `None` when the job cannot be created (the process is still killed explicitly).
    pub fn assign(child: &Child, limits: &JobLimits) -> Option<JobGuard> {
        Self::assign_handle(HANDLE(child.as_raw_handle()), limits)
    }

    /// The current process into a new job with `limits` (an elevated child, which IMT cannot put in its own).
    pub fn assign_self(limits: &JobLimits) -> Option<JobGuard> {
        // SAFETY: the pseudo handle of the current process is always valid.
        Self::assign_handle(unsafe { windows::Win32::System::Threading::GetCurrentProcess() }, limits)
    }

    fn assign_handle(process: HANDLE, limits: &JobLimits) -> Option<JobGuard> {
        // SAFETY: plain Win32 calls with valid arguments; the handle is closed in `Drop`.
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Some(limit) = limits.memory {
                info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                info.ProcessMemoryLimit = usize::try_from(limit).unwrap_or(usize::MAX);
            }
            if limits.no_children {
                info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_ACTIVE_PROCESS;
                info.BasicLimitInformation.ActiveProcessLimit = 1;
            }
            if let Some(secs) = limits.cpu_seconds {
                // In 100 ns units.
                info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_TIME;
                info.BasicLimitInformation.PerProcessUserTimeLimit = i64::try_from(secs.saturating_mul(10_000_000)).unwrap_or(i64::MAX);
            }
            let mut ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .is_ok();
            if let (true, Some(pct)) = (ok, limits.cpu_rate_percent) {
                let mut rate = JOBOBJECT_CPU_RATE_CONTROL_INFORMATION {
                    ControlFlags: JOB_OBJECT_CPU_RATE_CONTROL_ENABLE | JOB_OBJECT_CPU_RATE_CONTROL_HARD_CAP,
                    ..Default::default()
                };
                // In 1/100 of a percent.
                rate.Anonymous.CpuRate = u32::from(pct.clamp(1, 100)) * 100;
                ok = SetInformationJobObject(
                    job,
                    JobObjectCpuRateControlInformation,
                    &rate as *const _ as *const std::ffi::c_void,
                    std::mem::size_of::<JOBOBJECT_CPU_RATE_CONTROL_INFORMATION>() as u32,
                )
                .is_ok();
            }
            ok = ok && AssignProcessToJobObject(job, process).is_ok();
            if !ok {
                let _ = CloseHandle(job);
                return None;
            }
            Some(JobGuard(job))
        }
    }
}

impl Drop for JobGuard {
    fn drop(&mut self) {
        // SAFETY: the handle was created by `assign` and is closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}
