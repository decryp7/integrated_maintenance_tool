//! Reading a file the host opened (ADR 0017 §4, "host files instead of paths"): the plugin reads ranges through
//! the connection; it never learns the path and needs no file-system rights.

use crate::protocol::{HostFileRef, HostRequest, READ_CHUNK};
use anyhow::{anyhow, bail, Result};
use imt_ipc::Peer;
use std::time::Duration;

/// How long one read may take.
const READ_TIMEOUT: Duration = Duration::from_secs(60);

/// A host file, readable through `peer`.
pub struct HostFile<'a> {
    peer: &'a Peer,
    file: HostFileRef,
}

impl<'a> HostFile<'a> {
    pub fn new(peer: &'a Peer, file: HostFileRef) -> HostFile<'a> {
        HostFile { peer, file }
    }

    pub fn len(&self) -> u64 {
        self.file.len
    }

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
