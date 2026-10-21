//! Reading a file the host opened (ADR 0017 §4, "host files instead of paths"): the plugin reads ranges through
//! the connection; it never learns the path and needs no file-system rights.

use crate::protocol::{HostFileRef, HostRequest, READ_CHUNK};
use anyhow::{anyhow, bail, Result};
use imt_ipc::Peer;
use std::time::Duration;

/// How long one read may take.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// A file read by ranges: a [`HostFile`] in a plugin process, a byte slice in tests.
pub trait RangeRead {
    /// Size of the file in bytes.
    fn len(&self) -> u64;
    /// Up to `len` bytes at `offset` (fewer at the end of the file).
    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>>;
    /// Whether the file has no bytes.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl RangeRead for &[u8] {
    fn len(&self) -> u64 {
        <[u8]>::len(self) as u64
    }

    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>> {
        let start = usize::try_from(offset).unwrap_or(usize::MAX).min(<[u8]>::len(self));
        let end = start.saturating_add(len as usize).min(<[u8]>::len(self));
        Ok(self[start..end].to_vec())
    }
}

impl RangeRead for HostFile<'_> {
    fn len(&self) -> u64 {
        HostFile::len(self)
    }

    fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>> {
        HostFile::read_at(self, offset, len)
    }
}

/// A host file, readable through `peer`.
pub struct HostFile<'a> {
    peer: &'a Peer,
    file: HostFileRef,
    cancel: Option<&'a (dyn Fn() -> bool + Sync)>,
}

impl<'a> HostFile<'a> {
    /// Wraps the host file `file`, which is read through `peer`.
    pub fn new(peer: &'a Peer, file: HostFileRef) -> HostFile<'a> {
        HostFile { peer, file, cancel: None }
    }

    /// Stops [`HostFile::read_all`] between chunks once `cancelled` says so.
    pub fn with_cancel(mut self, cancelled: &'a (dyn Fn() -> bool + Sync)) -> HostFile<'a> {
        self.cancel = Some(cancelled);
        self
    }

    /// Size of the file in bytes.
    pub fn len(&self) -> u64 {
        self.file.len
    }

    /// Whether the file has no bytes.
    pub fn is_empty(&self) -> bool {
        self.file.len == 0
    }

    /// Up to `len` bytes at `offset` (fewer at the end of the file).
    pub fn read_at(&self, offset: u64, len: u32) -> Result<Vec<u8>> {
        let len = len.min(READ_CHUNK);
        let req = HostRequest::ReadFile {
            id: self.file.id,
            offset,
            len,
        };
        let bytes: Vec<u8> = self
            .peer
            .request(&req, Some(READ_TIMEOUT))
            .map_err(|e| anyhow!("cannot read the file: {e}"))?;
        if bytes.len() > len as usize {
            bail!("the host sent more than was asked for");
        }
        Ok(bytes)
    }

    /// The whole file, refused when it is larger than `cap` bytes (the importer's limit).
    pub fn read_all(&self, cap: u64) -> Result<Vec<u8>> {
        if self.file.len > cap {
            bail!("the file is {} bytes; this importer reads at most {cap}", self.file.len);
        }
        let mut out = Vec::with_capacity(self.file.len as usize);
        while (out.len() as u64) < self.file.len {
            if self.cancel.is_some_and(|c| c()) {
                bail!("cancelled");
            }
            let want = (self.file.len - out.len() as u64).min(u64::from(READ_CHUNK)) as u32;
            let chunk = self.read_at(out.len() as u64, want)?;
            if chunk.is_empty() {
                bail!("the file ended early ({} of {} bytes)", out.len(), self.file.len);
            }
            out.extend_from_slice(&chunk);
        }
        Ok(out)
    }

    /// The whole file as UTF-8 text (the host sends decoded text for text importers).
    pub fn read_text(&self, cap: u64) -> Result<String> {
        String::from_utf8(self.read_all(cap)?).map_err(|_| anyhow!("the host sent text that is not UTF-8"))
    }
}
