//! Downloading one model file safely: free-space check, resumable `.part` file, retries, size cap,
//! SHA-256 verification before the file appears under its real name.

use super::disk::DiskSpace;
use super::source::{ModelSource, Phase, Progress, RemoteFile};
use crate::catalog::is_safe_file_name;
use crate::error::{AiError, Result};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub max_bytes: u64,
    /// Free space that must remain after the download.
    pub margin_bytes: u64,
    pub attempts: u32,
    pub backoff: Duration,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        DownloadOptions {
            max_bytes: 20 * 1024 * 1024 * 1024,
            margin_bytes: 512 * 1024 * 1024,
            attempts: 3,
            backoff: Duration::from_secs(2),
        }
    }
}

pub struct Downloader<'a> {
    pub source: &'a dyn ModelSource,
    pub disk: &'a dyn DiskSpace,
    pub opts: DownloadOptions,
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// SHA-256 of a file, reporting progress and honouring cancellation.
pub fn sha256_file(path: &Path, progress: &Progress) -> Result<String> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if progress.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        progress.add(n as u64);
    }
    Ok(hex(&h.finalize()))
}

fn io_err(e: std::io::Error) -> AiError {
    if e.kind() == std::io::ErrorKind::StorageFull {
        AiError::Disk("the disk is full".into())
    } else {
        AiError::Io(e.to_string())
    }
}

impl Downloader<'_> {
    /// Downloads `remote` into `dir` and returns the final path. If a complete, correct file is
    /// already there it is kept. Cancelling keeps the `.part` file so the next run resumes.
    pub fn download(&self, remote: &RemoteFile, dir: &Path, progress: &Progress) -> Result<PathBuf> {
        self.download_checked(remote, dir, progress, is_safe_file_name)
    }

    /// [`Self::download`] stored under `name` instead of the host's file name (an image projector is often just
    /// `mmproj-F16.gguf` in every repository, so it is kept as `<model stem>.mmproj.gguf`).
    pub fn download_as(&self, remote: &RemoteFile, dir: &Path, name: &str, progress: &Progress) -> Result<PathBuf> {
        self.download_named(remote, dir, name, progress, is_safe_file_name)
    }

    /// [`Self::download`] for files other than models (e.g. inference runtimes): `name_ok` decides which
    /// file names are acceptable in `dir`; it must reject path separators and `..`.
    pub fn download_checked(&self, remote: &RemoteFile, dir: &Path, progress: &Progress, name_ok: fn(&str) -> bool) -> Result<PathBuf> {
        self.download_named(remote, dir, &remote.file, progress, name_ok)
    }

    fn download_named(
        &self,
        remote: &RemoteFile,
        dir: &Path,
        name: &str,
        progress: &Progress,
        name_ok: fn(&str) -> bool,
    ) -> Result<PathBuf> {
        for n in [remote.file.as_str(), name] {
            if !name_ok(n) || n.contains(['/', '\\']) || n.contains("..") {
                return Err(AiError::Refused(format!("'{n}' is not an acceptable file name")));
            }
        }
        if remote.size == 0 || remote.size > self.opts.max_bytes {
            return Err(AiError::Refused(format!(
                "the file is {:.1} GB, above the {:.1} GB download limit",
                remote.size as f64 / 1e9,
                self.opts.max_bytes as f64 / 1e9
            )));
        }
        if remote.sha256.len() != 64 {
            return Err(AiError::Integrity(
                "the host gave no SHA-256 checksum, so the file cannot be verified".into(),
            ));
        }
        std::fs::create_dir_all(dir)?;
        let dest = dir.join(name);
        let part = dir.join(format!("{name}.part"));
        if dest.is_file() && std::fs::metadata(&dest)?.len() == remote.size {
            progress.begin(Phase::Verifying, remote.size, 0);
            if sha256_file(&dest, progress)? == remote.sha256 {
                return Ok(dest);
            }
            std::fs::remove_file(&dest)?;
        }
        let mut have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        if have > remote.size {
            std::fs::remove_file(&part)?;
            have = 0;
        }
        if let Some(free) = self.disk.available_bytes(dir) {
            let need = (remote.size - have).saturating_add(self.opts.margin_bytes);
            if free < need {
                return Err(AiError::Disk(format!(
                    "{:.1} GB are needed but only {:.1} GB are free on that drive",
                    need as f64 / 1e9,
                    free as f64 / 1e9
                )));
            }
        }
        progress.begin(Phase::Downloading, remote.size, have);
        let mut failures = 0u32;
        while have < remote.size {
            let result = self.fetch_into(remote, &part, have, progress);
            have = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
            if have > remote.size {
                let _ = std::fs::remove_file(&part);
                return Err(AiError::Integrity("the host sent more data than the file size".into()));
            }
            match result {
                Ok(()) if have >= remote.size => {}
                // The stream ended early without an error: treat it like a dropped connection.
                Ok(()) => failures += 1,
                Err(e) if e.is_transient() => {
                    failures += 1;
                    if failures >= self.opts.attempts {
                        return Err(e);
                    }
                }
                Err(e) => return Err(e),
            }
            if have < remote.size {
                if failures >= self.opts.attempts {
                    return Err(AiError::Network(
                        "the connection kept dropping before the download finished; run it again to resume".into(),
                    ));
                }
                std::thread::sleep(self.opts.backoff * 2u32.saturating_pow(failures.saturating_sub(1)));
            }
        }
        progress.begin(Phase::Verifying, remote.size, 0);
        let got = sha256_file(&part, progress)?;
        if got != remote.sha256 {
            let _ = std::fs::remove_file(&part);
            return Err(AiError::Integrity(format!(
                "the checksum does not match (expected {}…, got {}…); the file was deleted, please try again",
                &remote.sha256[..12],
                &got[..12]
            )));
        }
        std::fs::rename(&part, &dest).map_err(io_err)?;
        Ok(dest)
    }

    fn fetch_into(&self, remote: &RemoteFile, part: &Path, have: u64, progress: &Progress) -> Result<()> {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .write(true)
            .open(part)
            .map_err(io_err)?;
        f.seek(SeekFrom::Start(have)).map_err(io_err)?;
        f.set_len(have).map_err(io_err)?;
        let cancel = progress.cancel_flag();
        let mut sink = |b: &[u8]| -> Result<()> {
            f.write_all(b).map_err(io_err)?;
            progress.add(b.len() as u64);
            Ok(())
        };
        let r = self.source.fetch(remote, have, &mut sink, &cancel);
        let _ = f.flush();
        if cancel.load(Ordering::Relaxed) && r.is_ok() {
            return Err(AiError::Cancelled);
        }
        r
    }
}
