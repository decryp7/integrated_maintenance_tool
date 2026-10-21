//! Connections to a target's diagnostics port.

use crate::DotnetError;
use std::io::{Read, Write};
use std::time::Duration;

/// A connection: bytes both ways.
pub trait Stream: Read + Write + Send {}
impl<T: Read + Write + Send> Stream for T {}

/// Opens connections to a process's diagnostics port.
pub trait Connector: Send + Sync {
    /// A new connection (one per command; a tracing session keeps its own).
    fn connect(&self, pid: u32, timeout: Duration) -> Result<Box<dyn Stream>, DotnetError>;
}

/// The real ports.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemConnector;

#[cfg(windows)]
impl Connector for SystemConnector {
    fn connect(&self, pid: u32, timeout: Duration) -> Result<Box<dyn Stream>, DotnetError> {
        use std::time::Instant;
        let path = format!(r"\\.\pipe\dotnet-diagnostic-{pid}");
        let deadline = Instant::now() + timeout;
        loop {
            match std::fs::OpenOptions::new().read(true).write(true).open(&path) {
                Ok(f) => return Ok(Box::new(f)),
                // ERROR_FILE_NOT_FOUND: no listening instance. Between accepting one connection and listening again
                // the runtime has none (a busy machine widens that gap): retry until the deadline, then the port is gone.
                Err(e) if e.raw_os_error() == Some(2) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(e) if e.raw_os_error() == Some(2) => return Err(DotnetError::NotFound { pid }),
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => return Err(DotnetError::Denied),
                // ERROR_PIPE_BUSY: every instance is in use; the runtime creates another soon.
                Err(e) if e.raw_os_error() == Some(231) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20));
                }
                Err(e) if e.raw_os_error() == Some(231) => return Err(DotnetError::Timeout),
                Err(e) => return Err(DotnetError::Io(e)),
            }
        }
    }
}

#[cfg(unix)]
impl Connector for SystemConnector {
    fn connect(&self, pid: u32, timeout: Duration) -> Result<Box<dyn Stream>, DotnetError> {
        let dir = std::env::var_os("TMPDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(std::env::temp_dir);
        let prefix = format!("dotnet-diagnostic-{pid}-");
        let socket = std::fs::read_dir(&dir)?
            .flatten()
            .take(100_000)
            .map(|e| e.path())
            .find(|p| {
                p.file_name()
                    .and_then(|n| n.to_str())
                    .is_some_and(|n| n.starts_with(&prefix) && n.ends_with("-socket"))
            })
            .ok_or(DotnetError::NotFound { pid })?;
        let s = std::os::unix::net::UnixStream::connect(&socket).map_err(|e| match e.kind() {
            std::io::ErrorKind::PermissionDenied => DotnetError::Denied,
            std::io::ErrorKind::ConnectionRefused | std::io::ErrorKind::NotFound => DotnetError::NotFound { pid },
            _ => DotnetError::Io(e),
        })?;
        s.set_write_timeout(Some(timeout))?;
        Ok(Box::new(s))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_missing_port_is_not_found() {
        let e = SystemConnector.connect(u32::MAX - 11, Duration::from_millis(100)).err().unwrap();
        assert!(matches!(e, DotnetError::NotFound { .. }), "{e:?}");
    }
}
