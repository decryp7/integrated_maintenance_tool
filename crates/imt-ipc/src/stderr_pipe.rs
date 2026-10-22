//! Standard error into a pipe this process reads itself (ADR 0017, amendment 2026-10-22).
//!
//! An elevated child is started through the administrator prompt and has no pipes, so the host cannot capture
//! its standard error. Instead the child redirects its own standard error (what `eprintln!`, panics and
//! libraries print) into a pipe with [`redirect`] and forwards the lines over its connection.

use std::fs::File;
use std::io;

/// Points this process' standard error at a new pipe and returns the pipe's read end. Everything written to
/// standard error afterwards can be read from it; the caller must keep reading, or writers block when the
/// pipe is full.
pub fn redirect() -> io::Result<File> {
    platform::redirect()
}

#[cfg(windows)]
mod platform {
    use std::fs::File;
    use std::io;
    use std::os::windows::io::{FromRawHandle, RawHandle};
    use windows::Win32::Foundation::HANDLE;
    use windows::Win32::System::Console::{SetStdHandle, STD_ERROR_HANDLE};
    use windows::Win32::System::Pipes::CreatePipe;

    pub fn redirect() -> io::Result<File> {
        let (mut read, mut write) = (HANDLE::default(), HANDLE::default());
        // SAFETY: valid out pointers; the handles are ours. The write end becomes the standard error handle for
        // the rest of the process' life (Rust's `stderr` looks the handle up on every write), so it is not closed.
        unsafe {
            CreatePipe(&mut read, &mut write, None, 0).map_err(|e| io::Error::other(format!("cannot create a pipe: {e}")))?;
            if let Err(e) = SetStdHandle(STD_ERROR_HANDLE, write) {
                let _ = windows::Win32::Foundation::CloseHandle(read);
                let _ = windows::Win32::Foundation::CloseHandle(write);
                return Err(io::Error::other(format!("cannot redirect standard error: {e}")));
            }
            Ok(File::from_raw_handle(read.0 as RawHandle))
        }
    }
}

#[cfg(unix)]
mod platform {
    use std::fs::File;
    use std::io;
    use std::os::unix::io::FromRawFd;

    pub fn redirect() -> io::Result<File> {
        let mut fds = [0 as libc::c_int; 2];
        // SAFETY: `fds` has room for the two descriptors; on success both are ours. `dup2` replaces descriptor 2
        // (standard error) with the write end, whose original descriptor is then closed.
        unsafe {
            if libc::pipe(fds.as_mut_ptr()) != 0 {
                return Err(io::Error::last_os_error());
            }
            let _ = libc::fcntl(fds[0], libc::F_SETFD, libc::FD_CLOEXEC);
            if libc::dup2(fds[1], 2) < 0 {
                let e = io::Error::last_os_error();
                libc::close(fds[0]);
                libc::close(fds[1]);
                return Err(e);
            }
            libc::close(fds[1]);
            Ok(File::from_raw_fd(fds[0]))
        }
    }
}
