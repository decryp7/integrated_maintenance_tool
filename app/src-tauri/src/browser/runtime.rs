//! The optional pinned Chromium (ADR 0043): when no Chromium-based browser exists (or the user prefers a known one), Debrief
//! can download Chrome for Testing at a pinned version, verify its SHA-256, unpack it, run a self-test and use it. Only
//! on the user's request (Settings ▸ Web browser). Updating installs the new version beside the old one and switches
//! only after the new one passes its self-test, so a failed update keeps the working version.

use super::hub::BrowserHub;
use super::renderer::Renderers;
use crate::windows::CurrentWindow;
use crate::CmdResult;
use chromium_driver::discover::Os;
use serde::Serialize;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime};

/// The pinned release (Chrome for Testing, stable channel at the time of the pin).
pub const PINNED_VERSION: &str = "156.0.8078.47";
const BASE: &str = "https://storage.googleapis.com/chrome-for-testing-public";
/// Largest download accepted.
const MAX_DOWNLOAD: u64 = 400 << 20;

/// (platform id of the release, SHA-256 of its zip).
const SUMS: &[(&str, &str)] = &[
    ("linux64", "e6c94490f0b6b337225aeb2d2a5034858516b7017356980ba94f5176d8774a50"),
    ("mac-arm64", "ad7f2b49a4dc679af286b1b1b386670d93ca7868079f69012d21ba0ac8293df3"),
    ("mac-x64", "5a38b8e3d621514fff0775dcae678a152ce378890188c88eef79fc97aa59fccd"),
    ("win64", "a4e0d57245e167e98a9d465af1e3774d2b4ec9f04e483d19bcd97a6f63cf46dd"),
];

/// The release platform of this machine, when there is a pinned build for it.
pub fn platform() -> Option<&'static str> {
    platform_of(std::env::consts::OS, std::env::consts::ARCH)
}

pub fn platform_of(os: &str, arch: &str) -> Option<&'static str> {
    match (os, arch) {
        ("linux", "x86_64") => Some("linux64"),
        ("macos", "aarch64") => Some("mac-arm64"),
        ("macos", "x86_64") => Some("mac-x64"),
        ("windows", "x86_64") => Some("win64"),
        _ => None,
    }
}

/// The download address and checksum for a platform.
pub fn pinned(platform: &str) -> Option<(String, &'static str)> {
    let sum = SUMS.iter().find(|(p, _)| *p == platform)?.1;
    Some((format!("{BASE}/{PINNED_VERSION}/{platform}/chrome-{platform}.zip"), sum))
}

fn root<R: Runtime>(app: &AppHandle<R>) -> Option<PathBuf> {
    crate::app_dirs::resolve(app, crate::app_dirs::AppDir::Data)
        .ok()
        .map(|d| d.join("chromium"))
}

/// The installed version recorded in `<root>/current.txt`, if its folder still exists.
fn installed_version(root: &Path) -> Option<String> {
    let v = std::fs::read_to_string(root.join("current.txt")).ok()?.trim().to_string();
    (!v.is_empty() && !v.contains(['/', '\\']) && root.join(&v).is_dir()).then_some(v)
}

fn installed_exe(root: &Path) -> Option<PathBuf> {
    let v = installed_version(root)?;
    chromium_driver::install::locate_executable(&root.join(v), Os::CURRENT).map(|b| b.path)
}

/// Points the renderers at the configured path and the installed Chromium. Call after settings or installs change.
pub fn refresh_sources<R: Runtime>(app: &AppHandle<R>, hub: &BrowserHub) {
    let Some(r) = app.try_state::<Arc<Renderers>>() else { return };
    r.set_configured(hub.settings().browser_path);
    r.set_installed(root(app).and_then(|d| installed_exe(&d)));
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RuntimeStatus {
    /// A browser can be started now.
    pub available: bool,
    /// `configured`, `installed`, `found` or `none`.
    pub source: &'static str,
    pub path: Option<String>,
    pub installed_version: Option<String>,
    pub pinned_version: &'static str,
    /// There is a pinned download for this platform.
    pub can_install: bool,
}

pub fn status<R: Runtime>(app: &AppHandle<R>) -> RuntimeStatus {
    let hub = app.try_state::<super::BrowserState>().map(|h| h.0.clone());
    let configured = hub.and_then(|h| h.settings().browser_path).filter(|p| p.is_file());
    let root = root(app);
    let installed = root.as_deref().and_then(installed_exe);
    let (source, path) = if let Some(p) = configured {
        ("configured", Some(p))
    } else if let Some(p) = installed {
        ("installed", Some(p))
    } else if let Some(b) = super::renderer::fallback_browser() {
        ("found", Some(b.path))
    } else {
        ("none", None)
    };
    RuntimeStatus {
        available: path.is_some(),
        source,
        path: path.map(|p| p.display().to_string()),
        installed_version: root.as_deref().and_then(installed_version),
        pinned_version: PINNED_VERSION,
        can_install: platform().is_some(),
    }
}

/// Downloads, verifies, unpacks and self-tests the pinned build; the previous install is removed only after the switch.
///
/// `report(phase, done, total)` is told the phase ("Downloading", "Unpacking", "Self-test"; bytes for the first two)
/// and returns false when the user cancelled: the install stops, removes what it wrote and keeps the previous version.
pub fn install_blocking(root: &Path, report: &mut dyn FnMut(&'static str, u64, u64) -> bool) -> Result<String, String> {
    let platform = platform().ok_or("there is no pinned Chromium for this system; install Edge, Chrome or Chromium instead")?;
    let (url, sum) = pinned(platform).ok_or("no pinned Chromium for this system")?;
    let old = installed_version(root);
    if old.as_deref() == Some(PINNED_VERSION) && installed_exe(root).is_some() {
        return Ok(PINNED_VERSION.into());
    }
    std::fs::create_dir_all(root).map_err(|e| format!("the Chromium folder could not be created: {e}"))?;
    let zip = root.join(format!("download-{PINNED_VERSION}.zip"));
    let dest = root.join(PINNED_VERSION);
    let result = (|| {
        // The archive is hashed while it is written: no second pass over the file.
        let got = download(&url, &zip, &mut |done, total| report("Downloading", done, total))?;
        let _ = std::fs::remove_dir_all(&dest);
        chromium_driver::install::extract_checked(&zip, &got, sum, &dest, &mut |done, total| report("Unpacking", done, total))
            .map_err(|e| e.to_string())?;
        if !report("Self-test", 0, 0) {
            return Err("the installation was cancelled".into());
        }
        let exe = chromium_driver::install::locate_executable(&dest, Os::CURRENT)
            .ok_or("the download does not contain a browser for this system")?;
        mark_runnable(&exe.path);
        chromium_driver::install::self_test(&exe.path).map_err(|e| e.to_string())?;
        std::fs::write(root.join("current.txt"), PINNED_VERSION).map_err(|e| e.to_string())
    })();
    let _ = std::fs::remove_file(&zip);
    match result {
        Ok(()) => {
            if let Some(o) = old.filter(|o| o != PINNED_VERSION) {
                let _ = std::fs::remove_dir_all(root.join(o));
            }
            Ok(PINNED_VERSION.into())
        }
        Err(e) => {
            // Roll back: the previous version (if any) stays current and untouched.
            let _ = std::fs::remove_dir_all(&dest);
            Err(format!("Chromium was not installed: {e}"))
        }
    }
}

#[cfg(unix)]
fn mark_runnable(p: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let _ = std::fs::set_permissions(p, std::fs::Permissions::from_mode(0o755));
}

#[cfg(not(unix))]
fn mark_runnable(_: &Path) {}

/// Downloads `url` into `to` and returns the SHA-256 of what was written; `report(done, total)` returning false cancels.
fn download(url: &str, to: &Path, report: &mut dyn FnMut(u64, u64) -> bool) -> Result<String, String> {
    let agent = debrief_ai::net::agent(&debrief_ai::net::HttpConfig {
        proxy: None,
        connect_timeout: std::time::Duration::from_secs(15),
        timeout: std::time::Duration::from_secs(15 * 60),
    })
    .map_err(|e| e.to_string())?;
    let resp = agent.get(url).call().map_err(|e| format!("the download failed: {e}"))?;
    if resp.status().as_u16() != 200 {
        return Err(format!("the download server answered {}", resp.status().as_u16()));
    }
    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    copy_hashed(resp.into_body().into_reader(), to, total, report)
}

/// Copies `body` to `to`, hashing while writing and reporting bytes; refuses more than [`MAX_DOWNLOAD`].
fn copy_hashed(mut body: impl Read, to: &Path, total: u64, report: &mut dyn FnMut(u64, u64) -> bool) -> Result<String, String> {
    let mut out = chromium_driver::install::HashingWriter::new(std::fs::File::create(to).map_err(|e| e.to_string())?);
    let mut buf = vec![0u8; 1 << 16];
    let mut done = 0u64;
    let mut last = 0u64;
    report(0, total);
    loop {
        let n = body.read(&mut buf).map_err(|e| format!("the download was interrupted: {e}"))?;
        if n == 0 {
            break;
        }
        std::io::Write::write_all(&mut out, &buf[..n]).map_err(|e| e.to_string())?;
        done += n as u64;
        if done > MAX_DOWNLOAD {
            return Err("the download is larger than expected and was refused".into());
        }
        if done - last >= 256 << 10 {
            last = done;
            if !report(done, total) {
                return Err("the installation was cancelled".into());
            }
        }
    }
    report(done, total);
    Ok(out.finish().1)
}

#[tauri::command]
pub async fn browser_runtime_status<R: Runtime>(app: AppHandle<R>) -> CmdResult<RuntimeStatus> {
    Ok(status(&app))
}

/// Downloads the pinned Chromium (only when the user asks). Returns the new status.
#[tauri::command]
pub async fn browser_runtime_install<R: Runtime>(app: AppHandle<R>, _win: CurrentWindow) -> CmdResult<RuntimeStatus> {
    use crate::jobs::{Done, JobCtx, JobKind, JobSpec, PhaseScale};
    let root = root(&app).ok_or("Debrief's data folder is not available")?;
    // A download job: Jobs shows each phase with its bytes, and Cancel works in every phase.
    let spec = JobSpec::new(JobKind::Download, "Install Chromium");
    let version = crate::jobs::run(&app, spec, Default::default(), move |ctx: &JobCtx| -> CmdResult<Done<String>> {
        let v = install_blocking(&root, &mut |phase, done, total| {
            let scale = if phase == "Self-test" { PhaseScale::Indeterminate } else { PhaseScale::Bytes };
            ctx.phase(phase, scale, done, total);
            !ctx.is_cancelled()
        })
        .map_err(|e| {
            if ctx.is_cancelled() {
                crate::CmdError::new(crate::ErrorCode::Cancelled, "cancelled")
            } else {
                crate::CmdError::from(e)
            }
        })?;
        Ok(Done::new(v))
    })
    .await?;
    log::info!(Version = version.as_str(); "Chromium installed");
    if let Some(h) = app.try_state::<super::BrowserState>() {
        refresh_sources(&app, &h.0);
    }
    Ok(status(&app))
}

/// Removes the downloaded Chromium (the machine's own browsers are never touched).
#[tauri::command]
pub async fn browser_runtime_remove<R: Runtime>(app: AppHandle<R>, _win: CurrentWindow) -> CmdResult<RuntimeStatus> {
    if let Some(r) = super::renderers(&app) {
        r.kill_all();
    }
    if let Some(root) = root(&app) {
        crate::error::lift(
            std::fs::remove_dir_all(&root)
                .or_else(|e| if root.exists() { Err(e) } else { Ok(()) })
                .map_err(crate::err),
        )?;
    }
    if let Some(h) = app.try_state::<super::BrowserState>() {
        refresh_sources(&app, &h.0);
    }
    Ok(status(&app))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_supported_platform_has_a_pinned_checksum_and_an_https_address() {
        for (os, arch) in [
            ("linux", "x86_64"),
            ("macos", "aarch64"),
            ("macos", "x86_64"),
            ("windows", "x86_64"),
        ] {
            let p = platform_of(os, arch).unwrap();
            let (url, sum) = pinned(p).unwrap();
            assert!(url.starts_with("https://storage.googleapis.com/chrome-for-testing-public/"));
            assert!(url.contains(PINNED_VERSION) && url.ends_with(".zip"));
            assert_eq!(sum.len(), 64);
        }
        assert!(platform_of("freebsd", "x86_64").is_none());
    }

    #[test]
    fn a_download_is_hashed_while_written_and_reports_growing_bytes() {
        let d = tempfile::tempdir().unwrap();
        let data: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
        let to = d.path().join("a.zip");
        let mut seen = Vec::new();
        let sum = copy_hashed(&data[..], &to, data.len() as u64, &mut |done, total| {
            seen.push((done, total));
            true
        })
        .unwrap();
        assert_eq!(sum, chromium_driver::install::sha256_file(&to).unwrap());
        assert_eq!(seen.first(), Some(&(0, 700_000)), "progress starts at once");
        assert_eq!(seen.last(), Some(&(700_000, 700_000)));
        assert!(seen.windows(2).all(|w| w[0].0 <= w[1].0));
    }

    #[test]
    fn cancelling_a_download_stops_it() {
        let d = tempfile::tempdir().unwrap();
        let data = vec![1u8; 2 << 20];
        let mut calls = 0;
        let e = copy_hashed(&data[..], &d.path().join("a.zip"), 0, &mut |_, _| {
            calls += 1;
            calls < 3
        })
        .unwrap_err();
        assert!(e.contains("cancelled"));
    }

    #[test]
    fn the_install_pointer_must_name_an_existing_plain_folder() {
        let d = tempfile::tempdir().unwrap();
        assert!(installed_version(d.path()).is_none());
        std::fs::write(d.path().join("current.txt"), "../x").unwrap();
        assert!(installed_version(d.path()).is_none());
        std::fs::create_dir(d.path().join("1.2.3")).unwrap();
        std::fs::write(d.path().join("current.txt"), "1.2.3\n").unwrap();
        assert_eq!(installed_version(d.path()).as_deref(), Some("1.2.3"));
    }
}
