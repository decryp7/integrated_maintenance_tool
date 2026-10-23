//! The `.debriefz` zip being written, with a size budget.

use anyhow::{Context, Result};
use std::fs::File;
use std::io::Write;
use std::path::Path;
use zip::write::SimpleFileOptions;

pub struct Bundle {
    zip: zip::ZipWriter<File>,
    used: u64,
    budget: u64,
}

impl Bundle {
    pub fn create(path: &Path, budget: u64) -> Result<Bundle> {
        let file = File::create(path).with_context(|| format!("cannot create {}", path.display()))?;
        Ok(Bundle {
            zip: zip::ZipWriter::new(file),
            used: 0,
            budget,
        })
    }

    fn write(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        let opts = SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        self.zip
            .start_file(name, opts)
            .with_context(|| format!("cannot add {name} to the bundle"))?;
        self.zip
            .write_all(bytes)
            .with_context(|| format!("cannot write {name} to the bundle"))
    }

    /// Adds a file when it fits in the budget; returns false (and writes
    /// nothing) when it does not.
    pub fn try_add(&mut self, name: &str, bytes: &[u8]) -> Result<bool> {
        if self.used.saturating_add(bytes.len() as u64) > self.budget {
            return Ok(false);
        }
        self.write(name, bytes)?;
        self.used += bytes.len() as u64;
        Ok(true)
    }

    /// Adds metadata that does not count against the budget.
    pub fn add_meta(&mut self, name: &str, bytes: &[u8]) -> Result<()> {
        self.write(name, bytes)
    }

    pub fn used(&self) -> u64 {
        self.used
    }

    pub fn finish(self) -> Result<()> {
        let mut f = self.zip.finish().context("cannot finish the bundle")?;
        f.flush().context("cannot flush the bundle")
    }
}
