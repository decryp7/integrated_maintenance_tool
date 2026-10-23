//! Background priority for download workers: a large download must not slow the work the user is doing.
//!
//! [`lower_current_thread`] asks the OS for the "background" class for the calling thread (QoS utility on macOS, nice
//! 10 on Linux, background mode on Windows, which also lowers its disk priority). It is best effort: where the OS
//! refuses or the platform has no such call, the thread keeps its priority and nothing is reported.

/// Lowers the calling thread's CPU (and where the OS has it, disk) priority.
pub fn lower_current_thread() {
    imp::lower();
}

#[cfg(target_os = "macos")]
mod imp {
    pub fn lower() {
        // SAFETY: a plain call that only changes the calling thread's quality-of-service class.
        unsafe {
            libc::pthread_set_qos_class_self_np(libc::qos_class_t::QOS_CLASS_UTILITY, 0);
        }
    }
}

#[cfg(all(unix, not(target_os = "macos")))]
mod imp {
    pub fn lower() {
        // SAFETY: gettid and setpriority take and return plain integers; on Linux a priority of a thread id is per thread.
        #[cfg(any(target_os = "linux", target_os = "android"))]
        unsafe {
            let tid = libc::syscall(libc::SYS_gettid) as libc::id_t;
            libc::setpriority(libc::PRIO_PROCESS, tid, 10);
        }
    }
}

#[cfg(windows)]
mod imp {
    use windows::Win32::System::Threading::{GetCurrentThread, SetThreadPriority, THREAD_MODE_BACKGROUND_BEGIN};

    pub fn lower() {
        // SAFETY: the pseudo handle of the calling thread is always valid; a refusal is ignored.
        unsafe {
            let _ = SetThreadPriority(GetCurrentThread(), THREAD_MODE_BACKGROUND_BEGIN);
        }
    }
}

#[cfg(not(any(unix, windows)))]
mod imp {
    pub fn lower() {}
}

#[cfg(test)]
mod tests {
    #[test]
    fn lowering_a_thread_neither_panics_nor_blocks() {
        // The effect is the OS's business; what matters is that a thread that lowers itself still runs to completion.
        let done = std::thread::spawn(|| {
            super::lower_current_thread();
            super::lower_current_thread();
            7
        })
        .join();
        assert_eq!(done.ok(), Some(7));
    }
}
