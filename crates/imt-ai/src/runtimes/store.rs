//! The inference runtimes on disk, in the app data folder:
//!
//! ```text
//! <data>/ai/runtimes/
//!   downloads/<asset>(.part)    archives while downloading (resumable), removed after unpacking
//!   <tag>/manifest.json         the release the builds below belong to
//!   <tag>/<id>/llama-server     an installed build (plus its libraries and runtime.json)
//!   <tag>/.staging-<id>/        a build being unpacked; renamed to <id> only when complete
//! ```
//!
//! Installing reuses the model downloader (free-space check, resume, retries, SHA-256 before use), then
//! unpacks defensively ([`super::unpack`]). A build folder exists only when it is complete, so the
//! worker (pointed at `<tag>/` with `IMT_LLAMA_DIR`) never sees half an install.

use super::catalog::{RuntimeBuild, RuntimeCatalog};
use super::unpack::{unpack, UnpackLimits};
use crate::error::{AiError, Result};
use crate::models::{DiskSpace, DownloadOptions, Downloader, ModelSource, Progress};
use serde::Serialize;
use serde_json::json;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The marker written into a complete build folder.
pub const MARKER: &str = "runtime.json";

fn server_file_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// One installed build.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledRuntime {
    pub id: String,
    pub tag: String,
    #[serde(skip)]
    pub dir: PathBuf,
    pub bytes: u64,
    /// Belongs to the release IMT currently pins (older ones still work but are no longer offered).
    pub current: bool,
}

/// Where an install is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "stage", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RuntimeStage {
    Downloading { asset: String, index: usize, count: usize },
    Unpacking { asset: String },
    Finishing,
}

pub struct RuntimeStore {
    root: PathBuf,
    busy: Mutex<HashSet<String>>,
}

fn dir_bytes(dir: &Path) -> u64 {
    std::fs::read_dir(dir)
        .map(|it| {
            it.flatten()
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}

/// An asset name is a plain archive file name.
fn is_asset_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && (name.ends_with(".zip") || name.ends_with(".tar.gz"))
        && name.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'))
}

struct BusyGuard<'a> {
    set: &'a Mutex<HashSet<String>>,
    key: String,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        self.set.lock().unwrap_or_else(|p| p.into_inner()).remove(&self.key);
    }
}

impl RuntimeStore {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        RuntimeStore {
            root: root.into(),
            busy: Mutex::new(HashSet::new()),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder holding the builds of `tag` (what the worker gets as `IMT_LLAMA_DIR`).
    pub fn builds_dir(&self, tag: &str) -> PathBuf {
        self.root.join(tag)
    }

    fn downloads(&self) -> PathBuf {
        self.root.join("downloads")
    }

    /// Every complete build on disk, newest release first. Unreadable folders are skipped.
    pub fn list(&self, current_tag: &str) -> Vec<InstalledRuntime> {
        let mut out = Vec::new();
        let Ok(tags) = std::fs::read_dir(&self.root) else {
            return out;
        };
        for tag in tags.flatten() {
            let tag_name = tag.file_name().to_string_lossy().into_owned();
            if !super::builds::is_build_id(&tag_name) || tag_name == "downloads" {
                continue;
            }
            let Ok(ids) = std::fs::read_dir(tag.path()) else { continue };
            for id in ids.flatten() {
                let name = id.file_name().to_string_lossy().into_owned();
                let dir = id.path();
                if super::builds::is_build_id(&name) && dir.join(MARKER).is_file() && dir.join(server_file_name()).is_file() {
                    out.push(InstalledRuntime {
                        bytes: dir_bytes(&dir),
                        current: tag_name == current_tag,
                        id: name,
                        tag: tag_name.clone(),
                        dir,
                    });
                }
            }
        }
        out.sort_by(|a, b| (!a.current, &b.tag, &a.id).cmp(&(!b.current, &a.tag, &b.id)));
        out
    }

    /// Removes an installed build.
    pub fn delete(&self, tag: &str, id: &str) -> Result<()> {
        if !super::builds::is_build_id(tag) || !super::builds::is_build_id(id) {
            return Err(AiError::Refused("that is not an inference build name".into()));
        }
        if self.busy.lock().unwrap_or_else(|p| p.into_inner()).contains(&format!("{tag}/{id}")) {
            return Err(AiError::Refused("that build is being installed".into()));
        }
        let dir = self.root.join(tag).join(id);
        if !dir.is_dir() {
            return Err(AiError::NotFound(format!("the {id} build")));
        }
        std::fs::remove_dir_all(&dir).map_err(|e| {
            AiError::Io(format!(
                "cannot delete {} ({e}); close IMT's AI engine (unload the model) and try again",
                dir.display()
            ))
        })
    }

    /// Downloads, verifies and unpacks `build` of `catalog`, replacing an older copy of the same build.
    /// Cancelling keeps partial downloads so the next install resumes.
    #[allow(clippy::too_many_arguments)]
    pub fn install(
        &self,
        catalog: &RuntimeCatalog,
        build: &RuntimeBuild,
        source: &dyn ModelSource,
        disk: &dyn DiskSpace,
        opts: DownloadOptions,
        progress: &Progress,
        on_stage: &mut dyn FnMut(RuntimeStage),
    ) -> Result<InstalledRuntime> {
        let tag = catalog.tag.as_str();
        if !super::builds::is_build_id(tag) || !super::builds::is_build_id(&build.id) {
            return Err(AiError::Refused("that is not an inference build name".into()));
        }
        let key = format!("{tag}/{}", build.id);
        if !self.busy.lock().unwrap_or_else(|p| p.into_inner()).insert(key.clone()) {
            return Err(AiError::Refused(format!("the {} build is already being installed", build.id)));
        }
        let _guard = BusyGuard { set: &self.busy, key };
        std::fs::create_dir_all(self.downloads())?;
        // Archives plus their unpacked contents (about three times the download) must fit.
        if let Some(free) = disk.available_bytes(&self.root) {
            let need = build.download_bytes().saturating_mul(3).saturating_add(opts.margin_bytes);
            if free < need {
                return Err(AiError::Disk(format!(
                    "{:.1} GB are needed but only {:.1} GB are free on that drive",
                    need as f64 / 1e9,
                    free as f64 / 1e9
                )));
            }
        }
        let max = opts.max_bytes.max(build.assets.iter().map(|a| a.size).max().unwrap_or(0));
        let dl = Downloader {
            source,
            disk,
            opts: DownloadOptions { max_bytes: max, ..opts },
        };
        let mut archives = Vec::new();
        for (i, a) in build.assets.iter().enumerate() {
            on_stage(RuntimeStage::Downloading {
                asset: a.name.clone(),
                index: i + 1,
                count: build.assets.len(),
            });
            let remote = source.describe(tag, &a.name)?;
            if remote.sha256 != a.sha256 || remote.size != a.size {
                return Err(AiError::Integrity(format!("{} does not match the pinned catalog", a.name)));
            }
            archives.push(dl.download_checked(&remote, &self.downloads(), progress, is_asset_name)?);
        }
        let tag_dir = self.builds_dir(tag);
        let staging = tag_dir.join(format!(".staging-{}", build.id));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        std::fs::create_dir_all(&staging)?;
        let result = (|| -> Result<()> {
            for archive in &archives {
                if progress.is_cancelled() {
                    return Err(AiError::Cancelled);
                }
                on_stage(RuntimeStage::Unpacking {
                    asset: archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default(),
                });
                unpack(archive, &staging, &UnpackLimits::default())?;
            }
            on_stage(RuntimeStage::Finishing);
            if !staging.join(server_file_name()).is_file() {
                return Err(AiError::Integrity(format!(
                    "the {} download does not contain llama-server",
                    build.id
                )));
            }
            super::macos::after_verified_unpack(&staging)?;
            let marker = json!({
                "version": 1,
                "tag": tag,
                "id": build.id,
                "assets": build.assets.iter().map(|a| json!({"name": a.name, "sha256": a.sha256})).collect::<Vec<_>>(),
            });
            std::fs::write(staging.join(MARKER), marker.to_string())?;
            std::fs::write(
                tag_dir.join("manifest.json"),
                json!({"tag": tag, "release": catalog.release, "source": catalog.source}).to_string(),
            )?;
            let dest = tag_dir.join(&build.id);
            if dest.exists() {
                std::fs::remove_dir_all(&dest)?;
            }
            std::fs::rename(&staging, &dest)?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = std::fs::remove_dir_all(&staging);
            return Err(e);
        }
        // The archives are no longer needed once unpacked.
        for archive in &archives {
            let _ = std::fs::remove_file(archive);
        }
        let dir = tag_dir.join(&build.id);
        Ok(InstalledRuntime {
            id: build.id.clone(),
            tag: tag.to_string(),
            bytes: dir_bytes(&dir),
            current: true,
            dir,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asset_names() {
        assert!(is_asset_name("llama-b1-bin-win-cpu-x64.zip") && is_asset_name("llama-b1-bin-ubuntu-x64.tar.gz"));
        for bad in ["", ".x.zip", "x.exe", "a/b.zip", "a..b.zip".replace("..", "/").as_str()] {
            assert!(!is_asset_name(bad), "{bad}");
        }
    }

    #[test]
    fn listing_ignores_incomplete_and_foreign_folders() {
        let dir = tempfile::tempdir().unwrap();
        let s = RuntimeStore::new(dir.path());
        assert!(s.list("b2").is_empty());
        for (tag, id, complete) in [
            ("b2", "cpu", true),
            ("b1", "vulkan", true),
            ("b2", ".staging-cuda-13", true),
            ("b2", "rocm", false),
        ] {
            let d = dir.path().join(tag).join(id);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(server_file_name()), b"x").unwrap();
            if complete {
                std::fs::write(d.join(MARKER), b"{}").unwrap();
            }
        }
        std::fs::create_dir_all(dir.path().join("downloads")).unwrap();
        let l = s.list("b2");
        let got: Vec<(&str, &str, bool)> = l.iter().map(|r| (r.tag.as_str(), r.id.as_str(), r.current)).collect();
        assert_eq!(got, [("b2", "cpu", true), ("b1", "vulkan", false)]);
        assert!(l[0].bytes > 0);
        assert_eq!(s.builds_dir("b2"), dir.path().join("b2"));
        s.delete("b1", "vulkan").unwrap();
        assert_eq!(s.list("b2").len(), 1);
        assert!(matches!(s.delete("b1", "vulkan"), Err(AiError::NotFound(_))));
        assert!(matches!(s.delete("..", "cpu"), Err(AiError::Refused(_))));
    }
}
