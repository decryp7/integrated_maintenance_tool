//! Windows: put the worker in a job object that kills it when IMT exits or crashes, so no worker is ever
//! left behind, and that caps its committed memory when asked to (a worker past the cap fails its
//! allocations and stops; the supervisor restarts it). (On Unix the worker exits when its stdin closes.)

use std::os::windows::io::AsRawHandle;
use std::process::Child;
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
    JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE, JOB_OBJECT_LIMIT_PROCESS_MEMORY,
};

pub struct JobGuard(HANDLE);

// SAFETY: a job handle may be used and closed from any thread.
unsafe impl Send for JobGuard {}
// SAFETY: the guard is never used through a shared reference (it only closes the handle in `Drop`).
unsafe impl Sync for JobGuard {}

impl JobGuard {
    /// Best effort: `None` when the job cannot be created (the process is still killed explicitly).
    pub fn assign(child: &Child, memory_limit: Option<u64>) -> Option<JobGuard> {
        // SAFETY: plain Win32 calls with valid arguments; the handle is closed in `Drop`.
        unsafe {
            let job = CreateJobObjectW(None, None).ok()?;
            let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if let Some(limit) = memory_limit {
                info.BasicLimitInformation.LimitFlags |= JOB_OBJECT_LIMIT_PROCESS_MEMORY;
                info.ProcessMemoryLimit = usize::try_from(limit).unwrap_or(usize::MAX);
            }
            let ok = SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32,
            )
            .is_ok()
                && AssignProcessToJobObject(job, HANDLE(child.as_raw_handle())).is_ok();
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
