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
//! worker (pointed at `<tag>/` with `DEBRIEF_LLAMA_DIR`) never sees half an install.

use super::catalog::{RuntimeBuild, RuntimeCatalog};
use super::unpack::{unpack, UnpackLimits};
use crate::error::{AiError, Result};
use crate::models::{DiskSpace, DownloadOptions, Downloader, ModelSource, Phase, Progress};
use serde::Serialize;
use serde_json::json;
use sha2::{Digest, Sha256};
use std::collections::HashSet;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The marker written into a complete build folder.
pub const MARKER: &str = "runtime.json";

/// The llama-server executable's file name on this platform.
pub fn server_file_name() -> &'static str {
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
    /// Belongs to the release Debrief currently pins (older ones still work but are no longer offered).
    pub current: bool,
}

/// Where an install is.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(tag = "stage", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum RuntimeStage {
    Downloading {
        asset: String,
        index: usize,
        count: usize,
    },
    Unpacking {
        asset: String,
    },
    /// The unpacked build runs its self-test with the active model before it is installed.
    Testing,
    Finishing,
}

/// Checks an unpacked build (its staging folder, holding `llama-server`) before it replaces anything: a failure
/// keeps the build out of use (ADR 0009).
pub type RuntimeCheck<'a> = &'a dyn Fn(&Path) -> Result<()>;

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

/// Copies `src` to `dest` and returns its SHA-256 (lowercase hex), reporting progress and honouring cancel.
fn copy_hashing(src: &Path, dest: &Path, progress: &Progress) -> Result<String> {
    let mut input = std::fs::File::open(src)?;
    let mut out = std::fs::File::create(dest)?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        if progress.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let n = input.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        out.write_all(&buf[..n]).map_err(|e| {
            if e.kind() == std::io::ErrorKind::StorageFull {
                AiError::Disk("the disk is full".into())
            } else {
                AiError::Io(e.to_string())
            }
        })?;
        progress.add(n as u64);
    }
    out.flush()?;
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// How long a folder whose server just exited may stay locked (Windows releases a process's DLLs a moment late).
const RELEASE_WAIT: std::time::Duration = std::time::Duration::from_secs(5);

/// Renames `from` to `to`, retrying while the files are still locked.
fn rename_retrying(from: &Path, to: &Path) -> std::io::Result<()> {
    let deadline = std::time::Instant::now() + RELEASE_WAIT;
    loop {
        match std::fs::rename(from, to) {
            Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied && std::time::Instant::now() < deadline => {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            r => return r,
        }
    }
}

/// The release whose builds the worker uses: the pinned one when any of its builds is installed, else the newest
/// older release that has builds (an update keeps it until the pinned build is installed and verified), else
/// the pinned one (nothing installed yet: the worker says so).
pub fn usable_tag(installed: &[InstalledRuntime], pinned: &str) -> String {
    if installed.iter().any(|r| r.tag == pinned) {
        return pinned.to_string();
    }
    let limit = super::builds::tag_number(pinned).unwrap_or(u64::MAX);
    installed
        .iter()
        .filter_map(|r| super::builds::tag_number(&r.tag).filter(|n| *n < limit).map(|n| (n, &r.tag)))
        .max_by_key(|(n, _)| *n)
        .map_or_else(|| pinned.to_string(), |(_, t)| t.clone())
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

    /// The folder holding the builds of `tag` (what the worker gets as `DEBRIEF_LLAMA_DIR`).
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
                "cannot delete {} ({e}); close Debrief's AI engine (unload the model) and try again",
                dir.display()
            ))
        })
    }

    /// The folder an install looks in for its archives first (where [`Self::import_archives`] puts them).
    pub fn downloads_dir(&self) -> PathBuf {
        self.downloads()
    }

    /// Takes the archives of `build` the user downloaded themselves (when Debrief's own download is blocked, e.g.
    /// by a TLS-inspecting proxy): each must be one of the build's assets by name, with the pinned size and
    /// SHA-256, and all of them must be given. They are copied into the downloads folder, so the next
    /// [`Self::install`] unpacks them without downloading. Reads every file once: call off the UI thread;
    /// `progress` reports the check (phase `Verifying`) and cancels it.
    pub fn import_archives(&self, build: &RuntimeBuild, files: &[PathBuf], progress: &Progress) -> Result<()> {
        let names = || build.assets.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(" and ");
        let mut chosen: Vec<(&super::catalog::RuntimeAsset, &PathBuf)> = Vec::new();
        for f in files {
            let name = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let asset = build
                .assets
                .iter()
                .find(|a| a.name.eq_ignore_ascii_case(&name))
                .ok_or_else(|| AiError::Invalid(format!("{name} is not a file of this build; it needs {}", names())))?;
            if chosen.iter().any(|(a, _)| a.name == asset.name) {
                return Err(AiError::Invalid(format!("{} was chosen twice", asset.name)));
            }
            chosen.push((asset, f));
        }
        if chosen.len() != build.assets.len() {
            return Err(AiError::Invalid(format!("this build needs {}: choose all of them", names())));
        }
        let dir = self.downloads();
        std::fs::create_dir_all(&dir)?;
        for (asset, src) in chosen {
            let size = std::fs::metadata(src)?.len();
            if size != asset.size {
                return Err(AiError::Integrity(format!(
                    "{} should be {} bytes but is {size}: the download is incomplete or it is another file; download it again",
                    asset.name, asset.size
                )));
            }
            progress.begin(Phase::Verifying, size, 0);
            let part = dir.join(format!("{}.import", asset.name));
            let got = copy_hashing(src, &part, progress).inspect_err(|_| {
                let _ = std::fs::remove_file(&part);
            })?;
            if got != asset.sha256 {
                let _ = std::fs::remove_file(&part);
                return Err(AiError::Integrity(format!(
                    "{} is not the original: its SHA-256 is {got}, expected {}; the file is damaged or was changed, download it again",
                    asset.name, asset.sha256
                )));
            }
            let dest = dir.join(&asset.name);
            let _ = std::fs::remove_file(dir.join(format!("{}.part", asset.name)));
            std::fs::rename(&part, &dest)?;
        }
        Ok(())
    }

    /// Downloads, verifies and unpacks `build` of `catalog`, runs `check` on the unpacked build when given, and only
    /// then replaces an older copy of the same build (so a build that fails its check never becomes the one in use;
    /// its verified archives stay, so trying again does not download them again). Cancelling keeps partial downloads
    /// so the next install resumes.
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
        check: Option<RuntimeCheck<'_>>,
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
            if let Some(check) = check {
                on_stage(RuntimeStage::Testing);
                check(&staging)?;
                if progress.is_cancelled() {
                    return Err(AiError::Cancelled);
                }
            }
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
            rename_retrying(&staging, &dest)?;
            Ok(())
        })();
        if let Err(e) = result {
            // The self-tested server may hold its libraries for a moment after it exited (Windows).
            let deadline = std::time::Instant::now() + RELEASE_WAIT;
            while std::fs::remove_dir_all(&staging).is_err() && staging.exists() && std::time::Instant::now() < deadline {
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
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

    #[test]
    fn the_worker_keeps_an_older_release_until_the_pinned_one_is_installed() {
        let rt = |tag: &str, id: &str| InstalledRuntime {
            id: id.into(),
            tag: tag.into(),
            dir: PathBuf::new(),
            bytes: 1,
            current: tag == "b200",
        };
        assert_eq!(usable_tag(&[], "b200"), "b200");
        assert_eq!(usable_tag(&[rt("b100", "cpu"), rt("b150", "vulkan")], "b200"), "b150");
        assert_eq!(usable_tag(&[rt("b100", "cpu"), rt("b200", "cpu")], "b200"), "b200");
        assert_eq!(usable_tag(&[rt("b300", "cpu")], "b200"), "b200", "never a newer, unpinned release");
    }

    fn asset(name: &str, data: &[u8]) -> super::super::catalog::RuntimeAsset {
        super::super::catalog::RuntimeAsset {
            name: name.into(),
            url: format!("https://github.com/ggml-org/llama.cpp/releases/download/b1/{name}"),
            size: data.len() as u64,
            sha256: Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect(),
        }
    }

    #[test]
    fn archives_downloaded_by_hand_are_checked_before_they_are_used() {
        let dir = tempfile::tempdir().unwrap();
        let s = RuntimeStore::new(dir.path().join("rt"));
        let (a, b) = (b"server archive".to_vec(), b"cuda runtime".to_vec());
        let build = RuntimeBuild {
            id: "cuda-12".into(),
            assets: vec![asset("llama-b1-cuda.zip", &a), asset("cudart-b1.zip", &b)],
        };
        let put = |name: &str, data: &[u8]| {
            let p = dir.path().join("dl").join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(&p, data).unwrap();
            p
        };
        let (pa, pb) = (put("llama-b1-cuda.zip", &a), put("CUDART-b1.zip", &b));
        let p = Progress::new();
        let err = |files: &[PathBuf]| s.import_archives(&build, files, &Progress::new()).unwrap_err().to_string();
        assert!(err(std::slice::from_ref(&pa)).contains("choose all of them"));
        assert!(err(&[pa.clone(), pa.clone()]).contains("twice"));
        assert!(err(&[pa.clone(), put("other.zip", b"x")]).contains("not a file of this build"));
        assert!(err(&[pa.clone(), put("x/cudart-b1.zip", b"short")]).contains("incomplete"));
        let mut changed = b.clone();
        changed[0] ^= 1;
        let e = err(&[pa.clone(), put("y/cudart-b1.zip", &changed)]);
        assert!(e.contains("SHA-256") && e.contains(&build.assets[1].sha256), "{e}");
        assert!(!s.downloads_dir().join("cudart-b1.zip").exists());
        assert!(!s.downloads_dir().join("cudart-b1.zip.import").exists(), "no leftovers");
        let cancelled = Progress::new();
        cancelled.cancel();
        assert_eq!(
            s.import_archives(&build, &[pa.clone(), pb.clone()], &cancelled),
            Err(AiError::Cancelled)
        );
        s.import_archives(&build, &[pb, pa], &p).unwrap();
        assert_eq!(std::fs::read(s.downloads_dir().join("llama-b1-cuda.zip")).unwrap(), a);
        assert_eq!(std::fs::read(s.downloads_dir().join("cudart-b1.zip")).unwrap(), b);
        assert_eq!(p.snapshot().phase, Phase::Verifying);
    }
}
