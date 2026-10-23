//! Downloading one model file safely: free-space check, resumable `.part` file, retries, size cap,
//! SHA-256 verification before the file appears under its real name.

use super::disk::DiskSpace;
use super::segmented::{self, Coverage, HostGate, Tuning};
use super::source::{ModelSource, Phase, Progress, RemoteFile};
use crate::catalog::is_safe_file_name;
use crate::error::{AiError, Result};
use sha2::{Digest, Sha256};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone)]
pub struct DownloadOptions {
    pub max_bytes: u64,
    /// Free space that must remain after the download.
    pub margin_bytes: u64,
    pub attempts: u32,
    pub backoff: Duration,
    /// Parallel connections for a large file (1 turns segmented downloads off; at most
    /// [`segmented::MAX_CONNECTIONS`]).
    pub connections: usize,
    /// Files of at least this many bytes are downloaded in segments when the host serves byte ranges.
    pub segment_threshold: u64,
    /// Smallest range a connection is given (and the smallest worth splitting off a slow one).
    pub min_block: u64,
    /// Caps the connections open at once across downloads; `None` for no cap beyond `connections`.
    pub host_gate: Option<Arc<HostGate>>,
}

impl Default for DownloadOptions {
    fn default() -> Self {
        DownloadOptions {
            max_bytes: 20 * 1024 * 1024 * 1024,
            margin_bytes: 512 * 1024 * 1024,
            attempts: 3,
            backoff: Duration::from_secs(2),
            connections: 4,
            segment_threshold: 64 * 1024 * 1024,
            min_block: 1024 * 1024,
            host_gate: Some(HostGate::shared()),
        }
    }
}

/// SHA-256 computed over the bytes as they are written, with how many it has seen.
#[derive(Default)]
struct StreamHash {
    sha: Sha256,
    len: u64,
}

impl StreamHash {
    fn update(&mut self, b: &[u8]) {
        self.sha.update(b);
        self.len += b.len() as u64;
    }

    fn finish(self) -> String {
        hex(&self.sha.finalize())
    }
}

pub struct Downloader<'a> {
    pub source: &'a dyn ModelSource,
    pub disk: &'a dyn DiskSpace,
    pub opts: DownloadOptions,
}

pub(super) fn hex(bytes: &[u8]) -> String {
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

pub(super) fn io_err(e: std::io::Error) -> AiError {
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
        let sidecar = segmented::sidecar_path(&part);
        let part_len = std::fs::metadata(&part).map(|m| m.len()).unwrap_or(0);
        // An interrupted segmented download left its finished ranges in the sidecar.
        let saved = segmented::load_sidecar(&sidecar, remote, part_len);
        if saved.is_none() {
            let _ = std::fs::remove_file(&sidecar);
        }
        let mut have = match &saved {
            Some(c) => c.prefix(),
            None => part_len,
        };
        if have > remote.size {
            std::fs::remove_file(&part)?;
            have = 0;
        }
        let covered = saved.as_ref().map_or(have, Coverage::covered);
        if let Some(free) = self.disk.available_bytes(dir) {
            let need = (remote.size - covered).saturating_add(self.opts.margin_bytes);
            if free < need {
                return Err(AiError::Disk(format!(
                    "{:.1} GB are needed but only {:.1} GB are free on that drive",
                    need as f64 / 1e9,
                    free as f64 / 1e9
                )));
            }
        }
        let segments = self.opts.connections.min(segmented::MAX_CONNECTIONS) >= 2
            && remote.size >= self.opts.segment_threshold
            && remote.size > have
            && self.source.supports_ranges(remote);
        let got = if segments {
            let cov = saved.unwrap_or_else(|| Coverage::prefix_of(have));
            let tuning = Tuning {
                connections: self.opts.connections,
                min_block: self.opts.min_block,
                attempts: self.opts.attempts,
                backoff: self.opts.backoff,
                gate: self.opts.host_gate.clone(),
            };
            segmented::run(self.source, remote, &part, cov, &tuning, progress)?
        } else {
            // One stream. A leftover sidecar means the file was preallocated: only its first run is real data.
            if saved.is_some() {
                let _ = std::fs::remove_file(&sidecar);
            }
            self.single_stream(remote, &part, have, progress)?
        };
        if got != remote.sha256 {
            let _ = std::fs::remove_file(&part);
            let _ = std::fs::remove_file(&sidecar);
            return Err(AiError::Integrity(format!(
                "the checksum does not match (expected {}…, got {}…); the file was deleted, please try again",
                &remote.sha256[..12],
                &got[..12]
            )));
        }
        std::fs::rename(&part, &dest).map_err(io_err)?;
        let _ = std::fs::remove_file(&sidecar);
        Ok(dest)
    }

    /// Downloads the rest of `remote` (from byte `have`) over one connection at a time and returns its SHA-256.
    fn single_stream(&self, remote: &RemoteFile, part: &Path, mut have: u64, progress: &Progress) -> Result<String> {
        progress.begin(Phase::Downloading, remote.size, have);
        // A download that starts from nothing is hashed as it is written, so there is no second pass over the file
        // (a resumed one has to re-read what was already on disk, as before).
        let mut hasher = (have == 0).then(StreamHash::default);
        let mut failures = 0u32;
        while have < remote.size {
            if hasher.as_ref().is_some_and(|h| h.len != have) {
                hasher = None;
            }
            let result = self.fetch_into(remote, part, have, progress, hasher.as_mut());
            have = std::fs::metadata(part).map(|m| m.len()).unwrap_or(0);
            if have > remote.size {
                let _ = std::fs::remove_file(part);
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
        let got = match hasher.filter(|h| h.len == remote.size) {
            Some(h) => h.finish(),
            None => {
                progress.begin(Phase::Verifying, remote.size, 0);
                sha256_file(part, progress)?
            }
        };
        Ok(got)
    }

    fn fetch_into(
        &self,
        remote: &RemoteFile,
        part: &Path,
        have: u64,
        progress: &Progress,
        mut hash: Option<&mut StreamHash>,
    ) -> Result<()> {
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
            if let Some(h) = hash.as_deref_mut() {
                h.update(b);
            }
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
