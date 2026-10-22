//! A supervised child process: killed (with its helper processes) on drop, so nothing leaks.

use std::process::{Child, Command};

pub struct Supervised {
    child: Option<Child>,
}

impl Supervised {
    /// Starts `cmd` in its own process group (Unix) so the whole browser tree can be ended at once.
    pub fn spawn(mut cmd: Command) -> std::io::Result<Supervised> {
        #[cfg(unix)]
        {
            use std::os::unix::process::CommandExt;
            cmd.process_group(0);
        }
        Ok(Supervised { child: Some(cmd.spawn()?) })
    }

    pub fn id(&self) -> Option<u32> {
        self.child.as_ref().map(Child::id)
    }

    /// Whether the process still runs.
    pub fn is_running(&mut self) -> bool {
        match self.child.as_mut() {
            Some(c) => matches!(c.try_wait(), Ok(None)),
            None => false,
        }
    }

    /// Ends the process tree now and reaps it. Idempotent.
    pub fn kill(&mut self) {
        let Some(mut c) = self.child.take() else { return };
        #[cfg(unix)]
        // SAFETY: killpg on the group we created for this child; a stale id only yields an error.
        unsafe {
            libc::killpg(c.id() as libc::pid_t, libc::SIGKILL);
        }
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            let _ = Command::new("taskkill")
                .args(["/T", "/F", "/PID", &c.id().to_string()])
                .creation_flags(0x0800_0000)
                .output();
        }
        let _ = c.kill();
        let _ = c.wait();
    }
}

impl Drop for Supervised {
    fn drop(&mut self) {
        self.kill();
    }
}

/// Whether a process with this id exists (used by leak tests).
#[cfg(unix)]
pub fn process_exists(pid: u32) -> bool {
    // SAFETY: signal 0 only checks existence.
    unsafe { libc::kill(pid as libc::pid_t, 0) == 0 }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    #[test]
    fn drop_kills_the_child_and_its_group() {
        let mut cmd = Command::new("sh");
        cmd.args(["-c", "sleep 60 & sleep 60"]);
        let s = Supervised::spawn(cmd).unwrap();
        let pid = s.id().unwrap();
        assert!(process_exists(pid));
        drop(s);
        std::thread::sleep(std::time::Duration::from_millis(200));
        assert!(!process_exists(pid), "process leaked");
    }

    #[test]
    fn kill_is_idempotent() {
        let mut cmd = Command::new("sleep");
        cmd.arg("60");
        let mut s = Supervised::spawn(cmd).unwrap();
        assert!(s.is_running());
        s.kill();
        s.kill();
        assert!(!s.is_running());
    }
}
