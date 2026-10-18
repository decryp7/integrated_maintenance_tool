//! `updates.json` in the config folder: when IMT last checked (so the daily automatic check survives
//! restarts) and an install that failed while quitting (shown at the next start). Versioned; a
//! missing, oversized or corrupt file means "nothing recorded".

use super::error::UpdateError;
use serde::{Deserialize, Serialize};
use std::path::Path;

const FILE: &str = "updates.json";
const VERSION: u32 = 1;
const MAX_BYTES: u64 = 64 * 1024;

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(default)]
    pub version: u32,
    /// RFC 3339.
    #[serde(default)]
    pub last_checked: Option<String>,
    #[serde(default)]
    pub failed_install: Option<FailedInstall>,
}

/// An install that failed when IMT quit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FailedInstall {
    pub version: String,
    pub disk_full: bool,
    pub detail: String,
}

impl FailedInstall {
    pub fn of(version: &str, e: &UpdateError) -> FailedInstall {
        FailedInstall {
            version: version.into(),
            disk_full: *e == UpdateError::DiskFull,
            detail: e.detail().unwrap_or_else(|| e.message()),
        }
    }

    pub fn error(&self) -> UpdateError {
        if self.disk_full {
            UpdateError::DiskFull
        } else {
            UpdateError::InstallFailed {
                detail: format!("IMT {}: {}", self.version, self.detail),
            }
        }
    }
}

pub fn load(dir: Option<&Path>) -> Record {
    let Some(path) = dir.map(|d| d.join(FILE)) else {
        return Record::default();
    };
    let read = || -> Option<Record> {
        let meta = std::fs::metadata(&path).ok()?;
        if !meta.is_file() || meta.len() > MAX_BYTES {
            return None;
        }
        serde_json::from_str(&std::fs::read_to_string(&path).ok()?).ok()
    };
    match read() {
        Some(r) if r.version <= VERSION => r,
        _ => Record::default(),
    }
}

/// Writes the record (temp file plus rename). Failures are logged: losing it only means an extra check.
pub fn save(dir: Option<&Path>, record: &Record) {
    let Some(dir) = dir else { return };
    let path = dir.join(FILE);
    let tmp = dir.join(format!(".{FILE}.{}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let text = serde_json::to_string_pretty(&Record {
            version: VERSION,
            ..record.clone()
        })?;
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &path)
    })();
    if let Err(e) = result {
        let _ = std::fs::remove_file(&tmp);
        log::warn!("could not save {}: {e}", path.display());
    }
}
