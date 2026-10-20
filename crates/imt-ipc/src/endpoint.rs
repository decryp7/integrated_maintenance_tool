//! A private local address the host listens on, and random tokens. Never
//! TCP. Unix: a socket inside a freshly created directory with mode 0700. Windows: a randomly named pipe
//! that only the current user can open (see `winpipe`).

use std::io;
use std::path::PathBuf;

/// `bytes` random bytes as lowercase hex.
pub fn random_hex(bytes: usize) -> io::Result<String> {
    let mut b = vec![0u8; bytes];
    getrandom::fill(&mut b).map_err(|e| io::Error::other(format!("no random numbers available: {e}")))?;
    Ok(b.iter().map(|x| format!("{x:02x}")).collect())
}

/// A 256-bit random token.
pub fn new_token() -> io::Result<String> {
    random_hex(32)
}

/// Compares in constant time, so the token cannot be guessed byte by byte.
pub fn tokens_equal(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = (a.len() != b.len()) as u8;
    for i in 0..a.len().max(b.len()) {
        diff |= a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(1);
    }
    diff == 0
}

/// A listening address owned by the supervisor; the private directory is removed on drop.
pub struct PrivateEndpoint {
    pub address: String,
    dir: Option<PathBuf>,
}

impl PrivateEndpoint {
    /// A private address named `imt-ipc-<random>`.
    pub fn create() -> io::Result<PrivateEndpoint> {
        Self::create_named("imt-ipc")
    }

    /// A private address whose directory (Unix) or pipe (Windows) is named `<prefix>-<random>`, so a user
    /// looking at open handles can tell which part of IMT it belongs to.
    #[cfg(unix)]
    pub fn create_named(prefix: &str) -> io::Result<PrivateEndpoint> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let mut bases: Vec<PathBuf> = Vec::new();
        if let Some(d) = std::env::var_os("XDG_RUNTIME_DIR").map(PathBuf::from).filter(|d| d.is_dir()) {
            bases.push(d);
        }
        bases.push(std::env::temp_dir());
        bases.push(PathBuf::from("/tmp"));
        let mut last = io::Error::other("no place for a private socket");
        for base in bases {
            let dir = base.join(format!("{prefix}-{}", random_hex(6)?));
            let sock = dir.join("w.sock");
            // Unix socket paths are limited to about 100 bytes.
            if sock.as_os_str().len() > 96 {
                continue;
            }
            match std::fs::DirBuilder::new().mode(0o700).create(&dir) {
                Ok(()) => {
                    // Defend against a permissive umask or an odd file system.
                    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
                    return Ok(PrivateEndpoint {
                        address: sock.to_string_lossy().into_owned(),
                        dir: Some(dir),
                    });
                }
                Err(e) => last = e,
            }
        }
        Err(last)
    }

    #[cfg(windows)]
    pub fn create_named(prefix: &str) -> io::Result<PrivateEndpoint> {
        Ok(PrivateEndpoint {
            address: format!(r"\\.\pipe\{prefix}-{}", random_hex(12)?),
            dir: None,
        })
    }
}

impl PrivateEndpoint {
    /// The private directory holding the socket (Unix); `None` for a named pipe.
    pub fn dir(&self) -> Option<&std::path::Path> {
        self.dir.as_deref()
    }
}

impl Drop for PrivateEndpoint {
    fn drop(&mut self) {
        if let Some(d) = self.dir.take() {
            let _ = std::fs::remove_dir_all(d);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_are_random_and_compared_exactly() {
        let a = new_token().unwrap();
        assert_eq!(a.len(), 64);
        assert_ne!(a, new_token().unwrap());
        assert!(tokens_equal(&a, &a.clone()));
        assert!(!tokens_equal(&a, &a[..63]) && !tokens_equal(&a, "") && !tokens_equal("", &a));
        assert!(tokens_equal("", ""));
        assert!(!tokens_equal("abc", "abd"));
    }

    #[cfg(unix)]
    #[test]
    fn the_socket_directory_is_private_and_removed_on_drop() {
        use std::os::unix::fs::PermissionsExt;
        let e = PrivateEndpoint::create().unwrap();
        let dir = std::path::Path::new(&e.address).parent().unwrap().to_path_buf();
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o700);
        assert!(e.address.ends_with("w.sock") && e.address.len() < 100);
        assert!(dir.file_name().unwrap().to_string_lossy().starts_with("imt-ipc-"));
        let other = PrivateEndpoint::create_named("imt-x").unwrap();
        assert_ne!(e.address, other.address);
        drop(e);
        assert!(!dir.exists());
    }
}
