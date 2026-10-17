//! Which physical file a path names, and whether its content is still the
//! content we read: the (device, inode) / (volume serial, file index) identity
//! survives renames, and fingerprints of the first bytes and of the bytes
//! before the read offset detect files truncated or rewritten in place.

use std::path::Path;

/// The physical file behind a path: (device, inode) on Unix, (volume serial
/// number, file index) on Windows. Stable across renames on the same volume.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FileIdentity {
    pub device: u64,
    pub index: u64,
}

/// The identity of the file at `path`, or `None` when it cannot be read (or
/// the platform has no stable file ids).
pub fn identity_of(path: &Path) -> Option<FileIdentity> {
    platform::identity(path)
}

#[cfg(unix)]
mod platform {
    use super::FileIdentity;
    use std::os::unix::fs::MetadataExt;
    use std::path::Path;

    pub fn identity(path: &Path) -> Option<FileIdentity> {
        let m = std::fs::metadata(path).ok()?;
        Some(FileIdentity {
            device: m.dev(),
            index: m.ino(),
        })
    }
}

#[cfg(windows)]
mod platform {
    //! `std::os::windows::fs::MetadataExt::{volume_serial_number, file_index}`
    //! are unstable, so the identity comes from `GetFileInformationByHandle`.
    use super::FileIdentity;
    use std::os::windows::io::AsRawHandle;
    use std::path::Path;

    #[repr(C)]
    #[derive(Default)]
    struct FileTime {
        low: u32,
        high: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct ByHandleFileInformation {
        attributes: u32,
        creation: FileTime,
        access: FileTime,
        write: FileTime,
        volume_serial: u32,
        size_high: u32,
        size_low: u32,
        links: u32,
        index_high: u32,
        index_low: u32,
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn GetFileInformationByHandle(file: *mut core::ffi::c_void, info: *mut ByHandleFileInformation) -> i32;
    }

    pub fn identity(path: &Path) -> Option<FileIdentity> {
        // std opens with FILE_SHARE_READ | WRITE | DELETE, so this never blocks
        // the writer's rotation.
        let file = std::fs::File::open(path).ok()?;
        let mut info = ByHandleFileInformation::default();
        // SAFETY: the handle is valid while `file` lives and `info` is a
        // properly sized, writable BY_HANDLE_FILE_INFORMATION.
        let ok = unsafe { GetFileInformationByHandle(file.as_raw_handle().cast(), &mut info) };
        (ok != 0).then_some(FileIdentity {
            device: info.volume_serial as u64,
            index: ((info.index_high as u64) << 32) | info.index_low as u64,
        })
    }
}

#[cfg(not(any(unix, windows)))]
mod platform {
    use super::FileIdentity;
    use std::path::Path;

    pub fn identity(_path: &Path) -> Option<FileIdentity> {
        None
    }
}

/// Bytes hashed for the head fingerprint.
pub const HEAD_BYTES: usize = 1024;
/// Bytes before the read offset hashed for the anchor.
pub const ANCHOR_BYTES: u64 = 64;

/// FNV-1a over `bytes`: cheap and good enough to tell two log heads apart.
pub fn hash(bytes: &[u8]) -> u64 {
    bytes
        .iter()
        .fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ *b as u64).wrapping_mul(0x0000_0100_0000_01b3))
}

/// Hash of a file's first bytes (up to [`HEAD_BYTES`]) and how many were hashed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Fingerprint {
    pub len: usize,
    pub hash: u64,
}

impl Fingerprint {
    pub fn of(head: &[u8]) -> Fingerprint {
        let head = &head[..head.len().min(HEAD_BYTES)];
        Fingerprint {
            len: head.len(),
            hash: hash(head),
        }
    }

    /// Whether `head` (the current first bytes of a file) starts with the bytes
    /// this fingerprint was taken from.
    pub fn matches(&self, head: &[u8]) -> bool {
        head.len() >= self.len && hash(&head[..self.len]) == self.hash
    }
}

/// Network filesystems where change notifications are unreliable: UNC paths
/// (`\\server\share`), Windows drives mapped to a share, and NFS/SMB/SSHFS
/// mounts on Linux. Live tail polls these.
pub fn is_network_path(path: &Path) -> bool {
    let s = path.to_string_lossy();
    if is_unc(&s) {
        return true;
    }
    network::is_remote(path)
}

fn is_unc(s: &str) -> bool {
    let lower = s.to_ascii_lowercase();
    if lower.starts_with(r"\\?\unc\") || lower.starts_with("//?/unc/") {
        return true;
    }
    if lower.starts_with(r"\\?\") || lower.starts_with(r"\\.\") {
        return false;
    }
    (s.starts_with(r"\\") || s.starts_with("//")) && s.len() > 2 && !s[2..].starts_with(['\\', '/'])
}

#[cfg(windows)]
mod network {
    use std::path::{Component, Path, Prefix};

    #[link(name = "kernel32")]
    extern "system" {
        fn GetDriveTypeW(root: *const u16) -> u32;
    }
    const DRIVE_REMOTE: u32 = 4;

    pub fn is_remote(path: &Path) -> bool {
        let Some(Component::Prefix(p)) = path.components().next() else {
            return false;
        };
        let letter = match p.kind() {
            Prefix::Disk(l) | Prefix::VerbatimDisk(l) => l,
            Prefix::UNC(..) | Prefix::VerbatimUNC(..) => return true,
            _ => return false,
        };
        let root: Vec<u16> = format!("{}:\\", letter as char).encode_utf16().chain(Some(0)).collect();
        // SAFETY: `root` is a NUL-terminated UTF-16 string that outlives the call.
        unsafe { GetDriveTypeW(root.as_ptr()) == DRIVE_REMOTE }
    }
}

#[cfg(target_os = "linux")]
mod network {
    use std::path::Path;

    const REMOTE: &[&str] = &[
        "nfs",
        "nfs4",
        "cifs",
        "smb3",
        "smbfs",
        "sshfs",
        "fuse.sshfs",
        "9p",
        "afs",
        "ceph",
        "glusterfs",
        "davfs",
        "fuse.rclone",
    ];

    pub fn is_remote(path: &Path) -> bool {
        let Ok(mounts) = std::fs::read_to_string("/proc/self/mounts") else {
            return false;
        };
        super::remote_mount(&mounts, path, REMOTE)
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
mod network {
    use std::path::Path;

    /// No dependency-free way to read the filesystem type here; a watcher
    /// error still falls back to polling, and polling can be forced.
    pub fn is_remote(_path: &Path) -> bool {
        false
    }
}

/// Whether the longest mount point (from `/proc/mounts` text) containing
/// `path` has one of the `remote` filesystem types.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn remote_mount(mounts: &str, path: &Path, remote: &[&str]) -> bool {
    mounts
        .lines()
        .filter_map(|l| {
            let mut parts = l.split_whitespace();
            let (_dev, mount, fs) = (parts.next()?, parts.next()?, parts.next()?);
            Some((mount.replace("\\040", " "), fs))
        })
        .filter(|(mount, _)| path.starts_with(mount))
        .max_by_key(|(mount, _)| mount.len())
        .is_some_and(|(_, fs)| remote.contains(&fs))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints() {
        let fp = Fingerprint::of(b"hello world");
        assert!(fp.matches(b"hello world, and more"));
        assert!(!fp.matches(b"hello"));
        assert!(!fp.matches(b"HELLO WORLD"));
        let long = vec![b'x'; HEAD_BYTES * 2];
        assert_eq!(Fingerprint::of(&long).len, HEAD_BYTES);
        assert_ne!(hash(b"a"), hash(b"b"));
    }

    #[test]
    fn network_paths() {
        assert!(is_network_path(Path::new(r"\\server\share\logs")));
        assert!(is_network_path(Path::new("//server/share/logs")));
        assert!(is_network_path(Path::new(r"\\?\UNC\server\share")));
        assert!(!is_network_path(Path::new(r"\\?\C:\logs")));
        assert!(!is_network_path(Path::new(r"\\.\pipe\x")));
        assert!(!is_network_path(Path::new("///x")));
        assert!(!is_unc(r"\\"));
        let mounts = "/dev/sda1 / ext4 rw 0 0\nserver:/x /mnt/nfs nfs4 rw 0 0\n//s/share /mnt/my\\040smb cifs rw 0 0\n";
        let remote = &["nfs4", "cifs"];
        assert!(remote_mount(mounts, Path::new("/mnt/nfs/app/log.txt"), remote));
        assert!(remote_mount(mounts, Path::new("/mnt/my smb/a"), remote));
        assert!(!remote_mount(mounts, Path::new("/home/x"), remote));
        assert!(!remote_mount("garbage", Path::new("/"), remote));
    }

    #[test]
    fn identity_follows_renames() {
        let dir = std::env::temp_dir().join(format!("imt-identity-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let a = dir.join("a.log");
        std::fs::write(&a, "x").unwrap();
        let before = identity_of(&a);
        let b = dir.join("b.log");
        std::fs::rename(&a, &b).unwrap();
        assert_eq!(identity_of(&b), before);
        assert!(identity_of(&a).is_none());
        std::fs::write(&a, "y").unwrap();
        if before.is_some() {
            assert_ne!(identity_of(&a), before);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
