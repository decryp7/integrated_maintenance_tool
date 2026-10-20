//! Starting child processes the way IMT always does: without a console window on Windows.
//!
//! IMT is a GUI app; a console program it starts (`nvidia-smi`, `cmd`, `logman`, `relog`, a plugin host,
//! the AI worker...) would otherwise flash a command-prompt window. Every `Command` IMT builds comes from
//! [`quiet_command`] (or [`quiet_command_with`]); the workspace test `no_console_windows` fails when new code
//! calls `Command::new` directly.

use std::ffi::OsStr;
use std::process::Command;

/// `CREATE_NO_WINDOW`: the child gets no console window (Windows process creation flag).
pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// `BELOW_NORMAL_PRIORITY_CLASS` (Windows process creation flag).
pub const BELOW_NORMAL_PRIORITY_CLASS: u32 = 0x0000_4000;

/// A [`Command`] for `program` that opens no console window on Windows (elsewhere a plain `Command`).
pub fn quiet_command(program: impl AsRef<OsStr>) -> Command {
    quiet_command_with(program, 0)
}

/// Like [`quiet_command`], with further Windows creation flags (e.g. [`BELOW_NORMAL_PRIORITY_CLASS`]);
/// `extra_flags` is ignored on other platforms.
pub fn quiet_command_with(program: impl AsRef<OsStr>, extra_flags: u32) -> Command {
    #[allow(clippy::disallowed_methods)]
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(creation_flags(extra_flags));
    }
    #[cfg(not(windows))]
    let _ = extra_flags;
    cmd
}

/// The Windows creation flags [`quiet_command_with`] sets: always `CREATE_NO_WINDOW`, plus `extra`.
pub const fn creation_flags(extra: u32) -> u32 {
    CREATE_NO_WINDOW | extra
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flags_always_hide_the_console() {
        assert_eq!(creation_flags(0), CREATE_NO_WINDOW);
        assert_eq!(creation_flags(BELOW_NORMAL_PRIORITY_CLASS), CREATE_NO_WINDOW | BELOW_NORMAL_PRIORITY_CLASS);
    }

    #[test]
    fn builds_a_command_for_the_program() {
        let cmd = quiet_command_with("imt-no-such-tool", BELOW_NORMAL_PRIORITY_CLASS);
        assert_eq!(cmd.get_program(), "imt-no-such-tool");
        assert!(quiet_command("imt-no-such-tool").spawn().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn runs_the_program() {
        let out = quiet_command("sh").args(["-c", "echo hi"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }

    #[cfg(windows)]
    #[test]
    fn runs_the_program() {
        let out = quiet_command("cmd").args(["/C", "echo hi"]).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "hi");
    }
}
