//! The FFmpeg runtimes on disk, in the app data folder (ADR 0033 §1):
//!
//! ```text
//! <data>/video/ffmpeg/
//!   state.json               which release is active, the one before it (rollback), a user-provided path
//!   downloads/<asset>(.part) archives while downloading (resumable), removed after a successful install
//!   <tag>/ffmpeg, ffprobe…   an installed release (plus runtime.json)
//!   .staging-<tag>/          a release being unpacked and self-tested; renamed to <tag> only when it passed
//! ```
//!
//! A release is used only after its self-test passed; an update keeps the current release until the new one passed
//! and keeps it afterwards as the rollback target. `state.json` is versioned; a missing or damaged one falls back to
//! what is on disk.

use super::catalog::{is_archive_name, is_tag, FfmpegAsset, FfmpegCatalog};
use super::selftest::SelfTestReport;
use crate::error::{Result, VideoError};
use crate::run::{exe, Tools};
use imt_ai::models::{DiskSpace, DownloadOptions, Downloader, ModelSource, Progress};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The marker of a complete release folder.
pub const MARKER: &str = "runtime.json";
const STATE_FILE: &str = "state.json";
const STATE_VERSION: u32 = 1;

/// The self-test a runtime must pass (the real one is [`super::selftest::self_test`]; tests pass a fake).
pub type Check<'a> = &'a dyn Fn(&Tools) -> Result<SelfTestReport>;

/// What is saved between runs.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
struct State {
    version: u32,
    active: Option<String>,
    previous: Option<String>,
    /// A user-provided ffmpeg (a file or its folder) and whether it is the one in use.
    custom: Option<String>,
    use_custom: bool,
    /// What the last self-test of each found (by tag, or `custom`).
    versions: std::collections::BTreeMap<String, String>,
}

/// What [`FfmpegStore::install`] is doing.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum InstallStage {
    Downloading,
    Unpacking,
    Testing,
    Finishing,
}

/// Which runtime analyses videos.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub enum RuntimeMode {
    /// Nothing installed or chosen.
    None,
    /// A release IMT downloaded.
    Managed,
    /// The user's own ffmpeg.
    Custom,
}

/// One installed release.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct InstalledRelease {
    pub tag: String,
    /// FFmpeg's version as its self-test reported it.
    pub version: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub bytes: u64,
}

/// What Settings ▸ Video shows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    pub mode: RuntimeMode,
    pub active: Option<InstalledRelease>,
    /// The release an update replaced ("Roll back").
    pub previous: Option<InstalledRelease>,
    /// The pinned release for this platform (none on macOS for now).
    pub pinned_tag: Option<String>,
    pub pinned_release: Option<String>,
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub pinned_bytes: Option<u64>,
    /// The pinned release is not the active one.
    pub update_available: bool,
    /// The user-provided ffmpeg, when one is set.
    pub custom_path: Option<String>,
    pub custom_version: Option<String>,
    /// Why the runtime in use cannot run (a missing binary), with what to do.
    pub problem: Option<String>,
}

pub struct FfmpegStore {
    root: PathBuf,
    busy: Mutex<bool>,
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

struct Busy<'a>(&'a Mutex<bool>);

impl Drop for Busy<'_> {
    fn drop(&mut self) {
        *self.0.lock().unwrap_or_else(|p| p.into_inner()) = false;
    }
}

/// The tools of a user-provided path: an `ffmpeg` executable (with `ffprobe` next to it) or their folder. Its folder
/// and, on Unix, the installation prefix above `bin/` may be read (their libraries).
pub fn custom_tools(path: &Path) -> Result<Tools> {
    // Homebrew's `ffmpeg` is a link into `Cellar/` whose libraries are in sibling folders of the same prefix: the
    // prefix of the path as the user gave it (`/opt/homebrew`) must be readable too.
    let given_prefix = (if path.is_dir() { Some(path) } else { path.parent() })
        .filter(|d| d.file_name().is_some_and(|n| n == "bin"))
        .and_then(Path::parent)
        .and_then(|p| p.canonicalize().ok());
    let path = path
        .canonicalize()
        .map_err(|_| VideoError::NotFound(format!("{}", path.display())))?;
    let dir = if path.is_dir() {
        path.clone()
    } else {
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().to_ascii_lowercase())
            .unwrap_or_default();
        if name != exe("ffmpeg") {
            return Err(VideoError::Invalid(format!(
                "choose the {} program or the folder it is in",
                exe("ffmpeg")
            )));
        }
        path.parent().map(Path::to_path_buf).unwrap_or_default()
    };
    let mut tools = Tools::in_dir(&dir);
    tools.lib_dir = None;
    if cfg!(unix) && dir.file_name().is_some_and(|n| n == "bin") {
        if let Some(prefix) = dir.parent() {
            tools.read.push(prefix.to_path_buf());
        }
    }
    if let Some(p) = given_prefix.filter(|p| !tools.read.contains(p)) {
        tools.read.push(p);
    }
    tools
        .present()
        .map_err(|_| VideoError::NotFound(format!("{} and {} in {}", exe("ffmpeg"), exe("ffprobe"), dir.display())))?;
    Ok(tools)
}

impl FfmpegStore {
    pub fn new(root: impl Into<PathBuf>) -> FfmpegStore {
        FfmpegStore {
            root: root.into(),
            busy: Mutex::new(false),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    fn downloads(&self) -> PathBuf {
        self.root.join("downloads")
    }

    fn release_dir(&self, tag: &str) -> PathBuf {
        self.root.join(tag)
    }

    fn complete(&self, tag: &str) -> bool {
        is_tag(tag) && self.release_dir(tag).join(MARKER).is_file()
    }

    /// The installed releases (complete folders), by tag.
    pub fn installed(&self) -> Vec<String> {
        let mut out: Vec<String> = std::fs::read_dir(&self.root)
            .map(|it| {
                it.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|t| t != "downloads" && self.complete(t))
                    .collect()
            })
            .unwrap_or_default();
        out.sort();
        out
    }

    fn state(&self) -> State {
        let text = std::fs::read_to_string(self.root.join(STATE_FILE)).unwrap_or_default();
        let mut s: State = serde_json::from_str(&text)
            .ok()
            .filter(|s: &State| s.version == STATE_VERSION)
            .unwrap_or_default();
        s.version = STATE_VERSION;
        // What is on disk wins over a stale or damaged state: the active release must exist, else the newest one.
        if !s.active.as_deref().is_some_and(|t| self.complete(t)) {
            s.active = self.installed().pop();
        }
        if s.previous
            .as_deref()
            .is_some_and(|p| !self.complete(p) || Some(p) == s.active.as_deref())
        {
            s.previous = None;
        }
        s
    }

    fn save(&self, s: &State) -> Result<()> {
        std::fs::create_dir_all(&self.root)?;
        let tmp = self.root.join(format!("{STATE_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(s).map_err(|e| VideoError::Io(e.to_string()))?)?;
        std::fs::rename(&tmp, self.root.join(STATE_FILE))?;
        Ok(())
    }

    fn release(&self, s: &State, tag: &str) -> InstalledRelease {
        InstalledRelease {
            tag: tag.to_string(),
            version: s.versions.get(tag).cloned(),
            bytes: dir_bytes(&self.release_dir(tag)),
        }
    }

    /// What Settings shows, for `catalog` on `platform`.
    pub fn status(&self, catalog: &FfmpegCatalog, platform: &str) -> RuntimeStatus {
        let s = self.state();
        let pinned = catalog.asset(platform);
        let mode = if s.use_custom && s.custom.is_some() {
            RuntimeMode::Custom
        } else if s.active.is_some() {
            RuntimeMode::Managed
        } else {
            RuntimeMode::None
        };
        let problem = match self.tools() {
            Err(VideoError::NoRuntime(m)) if mode != RuntimeMode::None => Some(m),
            Err(e) if mode != RuntimeMode::None => Some(e.to_string()),
            _ => None,
        };
        RuntimeStatus {
            mode,
            active: s.active.as_deref().map(|t| self.release(&s, t)),
            previous: s.previous.as_deref().map(|t| self.release(&s, t)),
            pinned_tag: pinned.map(|_| catalog.tag.clone()),
            pinned_release: pinned.map(|_| catalog.release.clone()),
            pinned_bytes: pinned.map(|a| a.size),
            update_available: pinned.is_some() && s.active.as_deref().is_some_and(|t| t != catalog.tag),
            custom_path: s.custom.clone(),
            custom_version: s.versions.get("custom").cloned(),
            problem,
        }
    }

    /// The tools to analyse with: the user's ffmpeg when chosen, else the active release. A missing program is a
    /// [`VideoError::NoRuntime`] that says what to do.
    pub fn tools(&self) -> Result<Tools> {
        let s = self.state();
        if s.use_custom {
            if let Some(p) = &s.custom {
                return custom_tools(Path::new(p)).map_err(|e| {
                    VideoError::NoRuntime(format!(
                        "the FFmpeg chosen in Settings ▸ Video cannot be used ({e}): choose it again or install IMT's"
                    ))
                });
            }
        }
        let tag = s
            .active
            .ok_or_else(|| VideoError::NoRuntime("FFmpeg is not installed: install it in Settings ▸ Video to analyse videos".into()))?;
        let tools = Tools::in_dir(&self.release_dir(&tag));
        tools.present()?;
        Ok(tools)
    }

    fn begin(&self) -> Result<Busy<'_>> {
        let mut b = self.busy.lock().unwrap_or_else(|p| p.into_inner());
        if *b {
            return Err(VideoError::Invalid("FFmpeg is already being installed".into()));
        }
        *b = true;
        Ok(Busy(&self.busy))
    }

    /// Downloads, verifies, unpacks and self-tests the pinned release for `platform`, then makes it the active one
    /// (the active one before becomes the rollback target; older ones are removed). A failure at any step leaves the
    /// runtime in use as it was; verified archives stay for another try.
    #[allow(clippy::too_many_arguments)]
    pub fn install(
        &self,
        catalog: &FfmpegCatalog,
        platform: &str,
        source: &dyn ModelSource,
        disk: &dyn DiskSpace,
        progress: &Progress,
        on_stage: &mut dyn FnMut(InstallStage),
        check: Check<'_>,
    ) -> Result<InstalledRelease> {
        let _busy = self.begin()?;
        let asset: &FfmpegAsset = catalog.asset(platform).ok_or_else(|| {
            VideoError::NoRuntime(format!(
                "IMT has no FFmpeg build for {platform}: choose your own ffmpeg in Settings ▸ Video"
            ))
        })?;
        let tag = catalog.tag.as_str();
        if !is_tag(tag) {
            return Err(VideoError::Invalid("the FFmpeg release name is not valid".into()));
        }
        on_stage(InstallStage::Downloading);
        let remote = source.describe(tag, &asset.name)?;
        if remote.sha256 != asset.sha256 || remote.size != asset.size {
            return Err(VideoError::Integrity(format!("{} does not match the pinned catalog", asset.name)));
        }
        let dl = Downloader {
            source,
            disk,
            opts: DownloadOptions {
                max_bytes: asset.size.max(1),
                margin_bytes: 256 << 20,
                ..DownloadOptions::default()
            },
        };
        let archive = dl.download_checked(&remote, &self.downloads(), progress, is_archive_name)?;
        let staging = self.root.join(format!(".staging-{tag}"));
        if staging.exists() {
            std::fs::remove_dir_all(&staging)?;
        }
        let result = (|| -> Result<SelfTestReport> {
            on_stage(InstallStage::Unpacking);
            super::unpack::unpack(&archive, &staging, Default::default())?;
            if progress.is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            on_stage(InstallStage::Testing);
            let report = check(&Tools::in_dir(&staging))?;
            if progress.is_cancelled() {
                return Err(VideoError::Cancelled);
            }
            on_stage(InstallStage::Finishing);
            let marker =
                serde_json::json!({"version": 1, "tag": tag, "release": catalog.release, "asset": asset.name, "sha256": asset.sha256});
            std::fs::write(staging.join(MARKER), marker.to_string())?;
            let dest = self.release_dir(tag);
            if dest.exists() {
                std::fs::remove_dir_all(&dest)?;
            }
            std::fs::rename(&staging, &dest)?;
            Ok(report)
        })();
        let report = match result {
            Ok(r) => r,
            Err(e) => {
                let _ = std::fs::remove_dir_all(&staging);
                return Err(e);
            }
        };
        let _ = std::fs::remove_file(&archive);
        let mut s = self.state();
        let before = s.active.clone().filter(|t| t != tag);
        s.previous = before.or(s.previous.filter(|p| p != tag));
        s.active = Some(tag.to_string());
        s.use_custom = false;
        if let Some(v) = &report.version {
            s.versions.insert(tag.to_string(), v.clone());
        }
        // Only the active release and the rollback target are kept.
        for t in self.installed() {
            if Some(&t) != s.active.as_ref() && Some(&t) != s.previous.as_ref() {
                let _ = std::fs::remove_dir_all(self.release_dir(&t));
                s.versions.remove(&t);
            }
        }
        self.save(&s)?;
        Ok(self.release(&s, tag))
    }

    /// Goes back to the release before the last update (it is self-tested again first).
    pub fn rollback(&self, check: Check<'_>) -> Result<InstalledRelease> {
        let _busy = self.begin()?;
        let mut s = self.state();
        let prev = s
            .previous
            .clone()
            .ok_or_else(|| VideoError::NotFound("an earlier FFmpeg release to go back to".into()))?;
        check(&Tools::in_dir(&self.release_dir(&prev)))?;
        s.previous = s.active.take();
        s.active = Some(prev.clone());
        s.use_custom = false;
        self.save(&s)?;
        Ok(self.release(&s, &prev))
    }

    /// Deletes an installed release (not while it is the only one in use without a replacement: then it is simply
    /// removed and analysing videos needs an install again).
    pub fn delete(&self, tag: &str) -> Result<()> {
        let _busy = self.begin()?;
        if !self.complete(tag) {
            return Err(VideoError::NotFound(format!("the FFmpeg release {tag}")));
        }
        std::fs::remove_dir_all(self.release_dir(tag)).map_err(|e| {
            VideoError::Io(format!(
                "cannot delete the FFmpeg release ({e}): wait for running analyses and try again"
            ))
        })?;
        let mut s = self.state();
        s.versions.remove(tag);
        self.save(&s)
    }

    /// Uses the user's ffmpeg at `path` (a file or its folder) once it passed the self-test; the path is saved only
    /// then.
    pub fn set_custom(&self, path: &Path, check: Check<'_>) -> Result<SelfTestReport> {
        let tools = custom_tools(path)?;
        let report = check(&tools)?;
        let _busy = self.begin()?;
        let mut s = self.state();
        s.custom = Some(path.canonicalize()?.to_string_lossy().into_owned());
        s.use_custom = true;
        match &report.version {
            Some(v) => s.versions.insert("custom".into(), v.clone()),
            None => s.versions.remove("custom"),
        };
        self.save(&s)?;
        Ok(report)
    }

    /// Stops using the user's ffmpeg (IMT's release is used again, when there is one).
    pub fn clear_custom(&self) -> Result<()> {
        let _busy = self.begin()?;
        let mut s = self.state();
        s.custom = None;
        s.use_custom = false;
        s.versions.remove("custom");
        self.save(&s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_ai::models::RemoteFile;
    use sha2::{Digest, Sha256};
    use std::collections::BTreeMap;
    use std::sync::atomic::AtomicBool;

    /// Serves archives from memory.
    struct Mem(BTreeMap<String, Vec<u8>>);

    impl ModelSource for Mem {
        fn describe(&self, repo: &str, file: &str) -> imt_ai::Result<RemoteFile> {
            let data = self.0.get(file).ok_or_else(|| imt_ai::AiError::NotFound(file.into()))?;
            Ok(RemoteFile {
                repo: repo.into(),
                file: file.into(),
                size: data.len() as u64,
                sha256: Sha256::digest(data).iter().map(|b| format!("{b:02x}")).collect(),
                license: None,
                gated: false,
            })
        }

        fn fetch(
            &self,
            r: &RemoteFile,
            offset: u64,
            sink: &mut dyn FnMut(&[u8]) -> imt_ai::Result<()>,
            _c: &AtomicBool,
        ) -> imt_ai::Result<()> {
            sink(&self.0[&r.file][offset as usize..])
        }
    }

    struct Roomy;

    impl DiskSpace for Roomy {
        fn available_bytes(&self, _dir: &Path) -> Option<u64> {
            None
        }
    }

    fn archive(tag: &str, content: &[u8]) -> (FfmpegCatalog, Mem) {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("a.zip");
        let (ff, fp) = (format!("b/bin/{}", exe("ffmpeg")), format!("b/bin/{}", exe("ffprobe")));
        crate::runtime::unpack::tests::zip_of(&p, &[(&ff, content), (&fp, b"probe")]);
        let bytes = std::fs::read(&p).unwrap();
        let name = format!("ffmpeg-{tag}-win64-lgpl-shared.zip");
        let asset = FfmpegAsset {
            name: name.clone(),
            url: format!("https://github.com/BtbN/FFmpeg-Builds/releases/download/{tag}/{name}"),
            size: bytes.len() as u64,
            sha256: Sha256::digest(&bytes).iter().map(|b| format!("{b:02x}")).collect(),
        };
        let catalog = FfmpegCatalog {
            version: 1,
            source: "test".into(),
            tag: tag.into(),
            release: format!("n-{tag}"),
            license: "LGPL-2.1-or-later".into(),
            platforms: [("test-x64".to_string(), asset)].into_iter().collect(),
        };
        (catalog, Mem([(name, bytes)].into_iter().collect()))
    }

    fn passes(_: &Tools) -> Result<SelfTestReport> {
        Ok(SelfTestReport {
            version: Some("n9".into()),
            gpl: false,
        })
    }

    fn install(store: &FfmpegStore, c: &FfmpegCatalog, m: &dyn ModelSource, check: Check<'_>) -> Result<InstalledRelease> {
        let mut stages = Vec::new();
        let r = store.install(c, "test-x64", m, &Roomy, &Progress::new(), &mut |s| stages.push(s), check);
        if r.is_ok() {
            assert_eq!(
                stages,
                [
                    InstallStage::Downloading,
                    InstallStage::Unpacking,
                    InstallStage::Testing,
                    InstallStage::Finishing
                ]
            );
        }
        r
    }

    #[test]
    fn nothing_installed_says_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let store = FfmpegStore::new(dir.path());
        let e = store.tools().unwrap_err();
        assert!(matches!(e, VideoError::NoRuntime(_)) && e.to_string().contains("install it in Settings"));
        let (c, _) = archive("t1", b"ff");
        let st = store.status(&c, "test-x64");
        assert_eq!((st.mode, st.active, st.problem), (RuntimeMode::None, None, None));
        assert_eq!(st.pinned_tag.as_deref(), Some("t1"));
        assert!(!st.update_available);
        assert!(store.status(&c, "macos-arm64").pinned_tag.is_none());
        let e = store
            .install(
                &c,
                "macos-arm64",
                &Mem(BTreeMap::new()),
                &Roomy,
                &Progress::new(),
                &mut |_| {},
                &passes,
            )
            .unwrap_err();
        assert!(e.to_string().contains("choose your own ffmpeg"), "{e}");
    }

    #[test]
    fn install_update_rollback_and_delete() {
        let dir = tempfile::tempdir().unwrap();
        let store = FfmpegStore::new(dir.path());
        let (c1, m1) = archive("t1", b"one");
        let r = install(&store, &c1, &m1, &passes).unwrap();
        assert_eq!((r.tag.as_str(), r.version.as_deref()), ("t1", Some("n9")));
        let tools = store.tools().unwrap();
        assert_eq!(std::fs::read(&tools.ffmpeg).unwrap(), b"one");
        assert!(
            !dir.path().join("downloads").join(&c1.platforms["test-x64"].name).exists(),
            "the archive is removed"
        );

        // An update: the new release is active, the old one the rollback target.
        let (c2, m2) = archive("t2", b"two");
        let st = store.status(&c2, "test-x64");
        assert!(st.update_available);
        install(&store, &c2, &m2, &passes).unwrap();
        let st = store.status(&c2, "test-x64");
        assert_eq!(
            (st.mode, st.active.unwrap().tag, st.previous.unwrap().tag),
            (RuntimeMode::Managed, "t2".into(), "t1".into())
        );
        assert!(!st.update_available);
        assert_eq!(std::fs::read(store.tools().unwrap().ffmpeg).unwrap(), b"two");

        // A third release drops the oldest.
        let (c3, m3) = archive("t3", b"three");
        install(&store, &c3, &m3, &passes).unwrap();
        assert_eq!(store.installed(), ["t2", "t3"]);

        // Rollback is self-tested too.
        let failing = |_: &Tools| -> Result<SelfTestReport> { Err(VideoError::Tool("broken".into())) };
        assert!(store.rollback(&failing).is_err());
        assert_eq!(store.status(&c3, "test-x64").active.unwrap().tag, "t3");
        store.rollback(&passes).unwrap();
        let st = store.status(&c3, "test-x64");
        assert_eq!((st.active.unwrap().tag, st.previous.unwrap().tag), ("t2".into(), "t3".into()));
        assert!(st.update_available);

        store.delete("t3").unwrap();
        assert!(store.delete("t3").is_err());
        assert!(store.delete("../t2").is_err());
        assert!(store.rollback(&passes).is_err(), "nothing left to go back to");
        store.delete("t2").unwrap();
        assert!(matches!(store.tools(), Err(VideoError::NoRuntime(_))));
    }

    #[test]
    fn a_failed_self_test_keeps_the_release_in_use() {
        let dir = tempfile::tempdir().unwrap();
        let store = FfmpegStore::new(dir.path());
        let (c1, m1) = archive("t1", b"one");
        install(&store, &c1, &m1, &passes).unwrap();
        let (c2, m2) = archive("t2", b"two");
        let fails = |_: &Tools| -> Result<SelfTestReport> {
            Err(VideoError::SelfTest {
                step: "probing the test clip".into(),
                message: "no frames".into(),
            })
        };
        let e = install(&store, &c2, &m2, &fails).unwrap_err();
        assert!(e.to_string().contains("self-test failed while probing"), "{e}");
        assert_eq!(
            std::fs::read(store.tools().unwrap().ffmpeg).unwrap(),
            b"one",
            "still the old release"
        );
        assert_eq!(store.installed(), ["t1"]);
        assert!(!dir.path().join(".staging-t2").exists());
        assert!(
            dir.path().join("downloads").join(&c2.platforms["test-x64"].name).exists(),
            "kept for another try"
        );
        // Another try uses the verified archive.
        install(&store, &c2, &m2, &passes).unwrap();
        assert_eq!(store.status(&c2, "test-x64").active.unwrap().tag, "t2");
    }

    #[test]
    fn a_checksum_mismatch_is_never_installed() {
        let dir = tempfile::tempdir().unwrap();
        let store = FfmpegStore::new(dir.path());
        let (mut c, m) = archive("t1", b"one");
        c.platforms.get_mut("test-x64").unwrap().sha256 = "0".repeat(64);
        let e = install(&store, &c, &m, &passes).unwrap_err();
        assert!(
            matches!(e, VideoError::Integrity(_)) && e.to_string().contains("does not match the pinned catalog"),
            "{e}"
        );
        assert!(store.installed().is_empty());
        // A served file that differs from what its host claims is caught by the download's own check.
        struct Liar(Mem);
        impl ModelSource for Liar {
            fn describe(&self, repo: &str, file: &str) -> imt_ai::Result<RemoteFile> {
                self.0.describe(repo, file)
            }
            fn fetch(
                &self,
                _r: &RemoteFile,
                _o: u64,
                sink: &mut dyn FnMut(&[u8]) -> imt_ai::Result<()>,
                _c: &AtomicBool,
            ) -> imt_ai::Result<()> {
                sink(&vec![0u8; _r.size as usize])
            }
        }
        let (c, m) = archive("t1", b"one");
        let e = install(&store, &c, &Liar(m), &passes).unwrap_err();
        assert!(matches!(e, VideoError::Integrity(_)), "{e}");
        assert!(store.installed().is_empty());
    }

    #[test]
    fn a_missing_binary_is_reported_with_what_to_do() {
        let dir = tempfile::tempdir().unwrap();
        let store = FfmpegStore::new(dir.path());
        let (c, m) = archive("t1", b"one");
        install(&store, &c, &m, &passes).unwrap();
        std::fs::remove_file(dir.path().join("t1").join(exe("ffprobe"))).unwrap();
        let e = store.tools().unwrap_err();
        assert!(e.to_string().contains("install the FFmpeg runtime again"), "{e}");
        assert!(store.status(&c, "test-x64").problem.unwrap().contains("missing"));
    }

    #[test]
    fn a_damaged_state_falls_back_to_what_is_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let store = FfmpegStore::new(dir.path());
        let (c, m) = archive("t1", b"one");
        install(&store, &c, &m, &passes).unwrap();
        std::fs::write(dir.path().join(STATE_FILE), b"{ damaged").unwrap();
        assert_eq!(store.status(&c, "test-x64").active.unwrap().tag, "t1");
        std::fs::write(dir.path().join(STATE_FILE), br#"{"version": 9, "active": "zz"}"#).unwrap();
        assert!(store.tools().is_ok());
    }

    #[test]
    fn a_user_provided_ffmpeg_is_used_after_its_self_test() {
        let dir = tempfile::tempdir().unwrap();
        let store = FfmpegStore::new(dir.path().join("rt"));
        let own = dir.path().join("own").join("bin");
        std::fs::create_dir_all(&own).unwrap();
        assert!(matches!(custom_tools(&own), Err(VideoError::NotFound(_))));
        std::fs::write(own.join(exe("ffmpeg")), b"mine").unwrap();
        std::fs::write(own.join("other.exe"), b"x").unwrap();
        assert!(custom_tools(&own.join("other.exe")).is_err());
        assert!(custom_tools(&own.join(exe("ffmpeg"))).is_err(), "ffprobe is missing");
        std::fs::write(own.join(exe("ffprobe")), b"mine").unwrap();
        let failing = |_: &Tools| -> Result<SelfTestReport> { Err(VideoError::Tool("old".into())) };
        assert!(store.set_custom(&own.join(exe("ffmpeg")), &failing).is_err());
        assert!(store.tools().is_err(), "not saved when it fails");
        let r = store.set_custom(&own.join(exe("ffmpeg")), &passes).unwrap();
        assert_eq!(r.version.as_deref(), Some("n9"));
        assert_eq!(std::fs::read(store.tools().unwrap().ffmpeg).unwrap(), b"mine");
        let (c, _) = archive("t1", b"x");
        let st = store.status(&c, "test-x64");
        assert_eq!((st.mode, st.custom_version.as_deref()), (RuntimeMode::Custom, Some("n9")));
        std::fs::remove_file(own.join(exe("ffprobe"))).unwrap();
        let e = store.tools().unwrap_err();
        assert!(e.to_string().contains("choose it again"), "{e}");
        assert!(store.status(&c, "test-x64").problem.is_some());
        store.clear_custom().unwrap();
        assert_eq!(store.status(&c, "test-x64").mode, RuntimeMode::None);
        assert!(custom_tools(&dir.path().join("missing")).is_err());
    }
}
