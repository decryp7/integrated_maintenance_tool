//! Error snapshots: screenshots of the IMT window that the UI renders in the webview and hands over
//! when something goes wrong, so a problem can be diagnosed afterwards. Everything stays on this
//! machine: images are written to `<app log dir>/snapshots/` (never to a path the UI names) and leave
//! only when the user exports them with `export_diagnostics`.
//!
//! Commands: `save_error_snapshot`, `list_error_snapshots`, `read_error_snapshot`,
//! `delete_error_snapshot`, `open_snapshots_folder`, `export_diagnostics`. Limits and the on/off switch are the
//! shared `snapshots` section of the diagnostics settings (`crate::diagnostics`). Event: `imt://error` (see [`EngineError`]). The wire contract is
//! `app/src/app/diagnostics/snapshot.types.ts`.

use crate::activity::Breadcrumb;
use crate::{err, CmdResult};
use base64::engine::general_purpose::STANDARD;
use base64::Engine as _;
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};
use std::fmt;
use std::hash::{BuildHasher, Hasher};
use std::io::Write;
use std::path::{Path, PathBuf};

/// Event the shell emits when the engine contained a failure (plugin crash, quarantine, a file that crashed its importer).
pub const ERROR_EVENT: &str = "imt://error";
const NOTICE_MARKER: &str = "notice-shown";
/// Largest image accepted from the UI.
pub const MAX_IMAGE_BYTES: usize = 3 * 1024 * 1024;
const MAX_REASON: usize = 100;
const MAX_MESSAGE: usize = 1_000;
const MAX_STACK: usize = 8_000;
const MAX_VIEW: usize = 100;
/// How many trailing IMT log lines a sidecar references.
const LOG_REFS: usize = 20;
/// IMT log lines (bytes) bundled into an export.
const EXPORT_LOG_TAIL: u64 = 256 * 1024;
/// Logs larger than this are not scanned for line references.
const MAX_LOG_SCAN: u64 = 16 * 1024 * 1024;

/// What can go wrong; the app edge turns it into a message for the UI.
#[derive(Debug, PartialEq, Eq)]
pub enum SnapshotError {
    BadMime(String),
    BadEncoding,
    BadMagic,
    TooLarge(usize),
    BadId(String),
    NotFound(String),
    Disabled,
    BadDestination(String),
    Io(String),
}

impl fmt::Display for SnapshotError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadMime(m) => write!(f, "unsupported image type \"{m}\" (use image/png or image/jpeg)"),
            Self::BadEncoding => write!(f, "the image is not valid base64"),
            Self::BadMagic => write!(f, "the data is not a PNG or JPEG image"),
            Self::TooLarge(n) => write!(f, "the image is too large ({n} bytes; the limit is {MAX_IMAGE_BYTES})"),
            Self::BadId(id) => write!(f, "\"{id}\" is not a snapshot id"),
            Self::NotFound(id) => write!(f, "no snapshot \"{id}\""),
            Self::Disabled => write!(f, "error snapshots are turned off in the settings"),
            Self::BadDestination(why) => write!(f, "cannot export there: {why}"),
            Self::Io(e) => write!(f, "{e}"),
        }
    }
}

impl std::error::Error for SnapshotError {}

impl From<std::io::Error> for SnapshotError {
    fn from(e: std::io::Error) -> Self {
        Self::Io(e.to_string())
    }
}

/// Image formats the UI may send.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageFormat {
    Png,
    Jpeg,
}

impl ImageFormat {
    fn parse(mime: &str) -> Result<Self, SnapshotError> {
        match mime {
            "image/png" => Ok(Self::Png),
            "image/jpeg" => Ok(Self::Jpeg),
            other => Err(SnapshotError::BadMime(other.chars().take(40).collect())),
        }
    }
    fn mime(self) -> &'static str {
        match self {
            Self::Png => "image/png",
            Self::Jpeg => "image/jpeg",
        }
    }
    pub(crate) fn extension(self) -> &'static str {
        match self {
            Self::Png => "png",
            Self::Jpeg => "jpg",
        }
    }
    fn matches(self, bytes: &[u8]) -> bool {
        match self {
            Self::Png => bytes.starts_with(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]),
            Self::Jpeg => bytes.starts_with(&[0xFF, 0xD8, 0xFF]),
        }
    }
}

/// Decodes and validates an image from the UI: mime, size, base64 and magic bytes.
pub fn validate_image(mime: &str, base64: &str) -> Result<(ImageFormat, Vec<u8>), SnapshotError> {
    let format = ImageFormat::parse(mime)?;
    if base64.len() > MAX_IMAGE_BYTES / 3 * 4 + 4 {
        return Err(SnapshotError::TooLarge(base64.len() / 4 * 3));
    }
    let bytes = STANDARD.decode(base64.trim()).map_err(|_| SnapshotError::BadEncoding)?;
    if bytes.len() > MAX_IMAGE_BYTES {
        return Err(SnapshotError::TooLarge(bytes.len()));
    }
    if !format.matches(&bytes) {
        return Err(SnapshotError::BadMagic);
    }
    Ok((format, bytes))
}

/// `20261017T083000123-1a2b3c4d`: a UTC timestamp (sorts oldest first) and 8 random hex digits.
/// This is the only form of id the commands accept, and it is also the file stem.
pub fn is_valid_id(id: &str) -> bool {
    let b = id.as_bytes();
    b.len() == 27
        && b[..8].iter().all(u8::is_ascii_digit)
        && b[8] == b'T'
        && b[9..18].iter().all(u8::is_ascii_digit)
        && b[18] == b'-'
        && b[19..].iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))
}

fn new_id() -> String {
    let random = std::collections::hash_map::RandomState::new().build_hasher().finish();
    format!("{}-{:08x}", Utc::now().format("%Y%m%dT%H%M%S%3f"), random as u32)
}

/// What the UI says about the error (untrusted: lengths are capped when stored).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotContext {
    pub reason: String,
    #[serde(default)]
    pub message: String,
    #[serde(default)]
    pub stack: Option<String>,
    #[serde(default)]
    pub view: Option<String>,
    /// The user asked for it (works even when automatic snapshots are off).
    #[serde(default)]
    pub manual: bool,
    /// What the user did just before (filled by the shell from the activity log, never by the UI).
    #[serde(skip)]
    pub breadcrumbs: Vec<Breadcrumb>,
}

/// What the app adds on its own.
#[derive(Debug, Clone)]
pub struct Environment {
    pub app_version: String,
    pub os: String,
    pub arch: String,
}

impl Environment {
    pub fn current() -> Self {
        Self {
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            os: std::env::consts::OS.to_string(),
            arch: std::env::consts::ARCH.to_string(),
        }
    }
}

fn truncate(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut s: String = text.chars().take(max).collect();
        s.push('…');
        s
    }
}

/// A reference into IMT's own log (not its content).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct LogRef {
    pub line: usize,
    pub time: Option<String>,
}

/// The `.json` next to each image.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Sidecar {
    pub version: u32,
    pub id: String,
    pub created_at: String,
    pub reason: String,
    pub message: String,
    pub stack: Option<String>,
    pub view: Option<String>,
    pub manual: bool,
    pub app_version: String,
    pub os: String,
    pub arch: String,
    pub mime: String,
    pub bytes: u64,
    pub log_refs: Vec<LogRef>,
    /// The operations leading up to the error (a sidecar without them, e.g. cut short, still lists).
    #[serde(default)]
    pub breadcrumbs: Vec<Breadcrumb>,
    /// Human-readable steps to reproduce, generated from `breadcrumbs`.
    #[serde(default)]
    pub steps: Vec<String>,
}

/// One snapshot as listed to the UI.
#[derive(Debug, Clone, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SnapshotInfo {
    pub id: String,
    pub created_at: String,
    pub reason: String,
    pub message: String,
    pub stack: Option<String>,
    pub view: Option<String>,
    pub manual: bool,
    pub app_version: String,
    pub os: String,
    pub mime: String,
    pub bytes: u64,
    pub breadcrumbs: Vec<Breadcrumb>,
    pub steps: Vec<String>,
}

impl From<Sidecar> for SnapshotInfo {
    fn from(s: Sidecar) -> Self {
        Self {
            id: s.id,
            created_at: s.created_at,
            reason: s.reason,
            message: s.message,
            stack: s.stack,
            view: s.view,
            manual: s.manual,
            app_version: s.app_version,
            os: s.os,
            mime: s.mime,
            bytes: s.bytes,
            breadcrumbs: s.breadcrumbs,
            steps: s.steps,
        }
    }
}

/// Retention: the newest `max_count` snapshots, no more than `max_bytes` in total, none older than `max_age`.
#[derive(Debug, Clone, Copy)]
pub struct Limits {
    pub max_count: usize,
    pub max_bytes: u64,
    pub max_age: Option<std::time::Duration>,
}

impl Default for Limits {
    fn default() -> Self {
        Self::from(&crate::diagnostics::SnapshotLimits::default())
    }
}

impl From<&crate::diagnostics::SnapshotLimits> for Limits {
    fn from(l: &crate::diagnostics::SnapshotLimits) -> Self {
        Self {
            max_count: l.max_count as usize,
            max_bytes: u64::from(l.max_total_mb) * 1024 * 1024,
            max_age: (l.max_age_days > 0).then(|| std::time::Duration::from_secs(u64::from(l.max_age_days) * 86_400)),
        }
    }
}

/// The snapshots folder. All paths are built here from validated ids.
pub struct SnapshotStore {
    dir: PathBuf,
    log_file: Option<PathBuf>,
    limits: Limits,
}

impl SnapshotStore {
    pub fn new(dir: PathBuf, log_file: Option<PathBuf>, limits: Limits) -> Self {
        Self { dir, log_file, limits }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Writes the image and its sidecar, prunes old snapshots and returns the new id.
    pub fn save(&self, format: ImageFormat, image: &[u8], ctx: &SnapshotContext, env: &Environment) -> Result<String, SnapshotError> {
        std::fs::create_dir_all(&self.dir)?;
        let id = new_id();
        let sidecar = Sidecar {
            version: 1,
            id: id.clone(),
            created_at: Utc::now().to_rfc3339_opts(SecondsFormat::Millis, true),
            reason: truncate(&ctx.reason, MAX_REASON),
            message: truncate(&ctx.message, MAX_MESSAGE),
            stack: ctx.stack.as_deref().map(|s| truncate(s, MAX_STACK)),
            view: ctx.view.as_deref().map(|s| truncate(s, MAX_VIEW)),
            manual: ctx.manual,
            app_version: env.app_version.clone(),
            os: env.os.clone(),
            arch: env.arch.clone(),
            mime: format.mime().to_string(),
            bytes: image.len() as u64,
            log_refs: self.log_refs(),
            steps: snapshot_steps(&ctx.breadcrumbs, &ctx.message),
            breadcrumbs: last_breadcrumbs(&ctx.breadcrumbs),
        };
        let image_path = self.image_path(&id, format);
        std::fs::write(&image_path, image)?;
        let json = serde_json::to_vec_pretty(&sidecar).map_err(|e| SnapshotError::Io(e.to_string()))?;
        if let Err(e) = std::fs::write(self.sidecar_path(&id), json) {
            let _ = std::fs::remove_file(&image_path);
            return Err(e.into());
        }
        self.prune();
        Ok(id)
    }

    /// Every snapshot that has a readable sidecar and its image, oldest first.
    pub fn list(&self) -> Vec<SnapshotInfo> {
        let Ok(entries) = std::fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        let mut ids: Vec<String> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let stem = name.strip_suffix(".json")?;
                is_valid_id(stem).then(|| stem.to_string())
            })
            .collect();
        ids.sort();
        ids.iter().filter_map(|id| self.read_sidecar(id)).map(SnapshotInfo::from).collect()
    }

    /// Mime type and bytes of one image. The id must be in the listing.
    pub fn read_image(&self, id: &str) -> Result<(ImageFormat, Vec<u8>), SnapshotError> {
        let (sidecar, format) = self.find(id)?;
        let bytes = std::fs::read(self.image_path(&sidecar.id, format))?;
        Ok((format, bytes))
    }

    /// Deletes one snapshot. The id must be in the listing.
    pub fn delete(&self, id: &str) -> Result<(), SnapshotError> {
        let (sidecar, format) = self.find(id)?;
        std::fs::remove_file(self.image_path(&sidecar.id, format))?;
        let _ = std::fs::remove_file(self.sidecar_path(&sidecar.id));
        Ok(())
    }

    /// Zips the chosen snapshots (image and sidecar) and the recent IMT log into `dest`.
    /// Returns the number of files written.
    pub fn export(&self, ids: &[String], dest: &Path) -> Result<usize, SnapshotError> {
        let chosen: Vec<(Sidecar, ImageFormat)> = ids.iter().map(|id| self.find(id)).collect::<Result<_, _>>()?;
        let dest = checked_destination(dest)?;
        let result = self.write_zip(&chosen, &dest);
        if result.is_err() {
            let _ = std::fs::remove_file(&dest);
        }
        result
    }

    fn write_zip(&self, chosen: &[(Sidecar, ImageFormat)], dest: &Path) -> Result<usize, SnapshotError> {
        let zerr = |e: zip::result::ZipError| SnapshotError::Io(e.to_string());
        let file = std::fs::File::create(dest)?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        let mut count = 0;
        for (sidecar, format) in chosen {
            let stem = &sidecar.id;
            zip.start_file(format!("snapshots/{stem}.{}", format.extension()), opts)
                .map_err(zerr)?;
            zip.write_all(&std::fs::read(self.image_path(stem, *format))?)?;
            zip.start_file(format!("snapshots/{stem}.json"), opts).map_err(zerr)?;
            zip.write_all(&std::fs::read(self.sidecar_path(stem))?)?;
            count += 2;
        }
        let steps = steps_document(chosen.iter().map(|(s, _)| s));
        if !steps.is_empty() {
            zip.start_file("steps-to-reproduce.txt", opts).map_err(zerr)?;
            zip.write_all(steps.as_bytes())?;
            count += 1;
        }
        if let Some(tail) = self.log_file.as_deref().and_then(|p| tail_of(p, EXPORT_LOG_TAIL)) {
            // Named `imt.log` in the archive whatever today's file is called.
            zip.start_file(format!("logs/{}.log", crate::diagnostics::LOG_NAME), opts)
                .map_err(zerr)?;
            zip.write_all(&tail)?;
            count += 1;
        }
        zip.finish().map_err(zerr)?;
        Ok(count)
    }

    /// Applies the retention limits now.
    pub fn prune(&self) {
        self.prune_at(Utc::now());
    }

    /// Deletes, oldest first, whatever exceeds the age, count or size limit. The newest snapshot is kept
    /// unless it is itself older than the age limit.
    pub fn prune_at(&self, now: chrono::DateTime<Utc>) {
        let all = self.list();
        let too_old = |info: &SnapshotInfo| {
            self.limits.max_age.is_some_and(|max| {
                chrono::DateTime::parse_from_rfc3339(&info.created_at)
                    .ok()
                    .and_then(|t| (now - t.with_timezone(&Utc)).to_std().ok())
                    .is_some_and(|age| age > max)
            })
        };
        let mut total: u64 = 0;
        let mut keep = 0usize;
        for info in all.iter().rev() {
            let size = info.bytes + self.sidecar_len(&info.id);
            if too_old(info) || (keep >= 1 && (keep >= self.limits.max_count || total + size > self.limits.max_bytes)) {
                break;
            }
            total += size;
            keep += 1;
        }
        for old in &all[..all.len() - keep] {
            if let Err(e) = self.delete(&old.id) {
                log::warn!("could not prune error snapshot {}: {e}", old.id);
            }
        }
    }

    /// True once, the first time a snapshot is saved on this machine (the UI shows the "saved locally" notice).
    fn take_first_time(&self) -> bool {
        let marker = self.dir.join(NOTICE_MARKER);
        let first = !marker.exists();
        if first {
            let _ = std::fs::write(marker, "");
        }
        first
    }

    fn find(&self, id: &str) -> Result<(Sidecar, ImageFormat), SnapshotError> {
        if !is_valid_id(id) {
            return Err(SnapshotError::BadId(id.chars().take(40).collect()));
        }
        let sidecar = self.read_sidecar(id).ok_or_else(|| SnapshotError::NotFound(id.to_string()))?;
        let format = ImageFormat::parse(&sidecar.mime).map_err(|_| SnapshotError::NotFound(id.to_string()))?;
        if !self.image_path(id, format).is_file() {
            return Err(SnapshotError::NotFound(id.to_string()));
        }
        Ok((sidecar, format))
    }

    fn read_sidecar(&self, id: &str) -> Option<Sidecar> {
        let text = std::fs::read_to_string(self.sidecar_path(id)).ok()?;
        let sidecar: Sidecar = serde_json::from_str(&text).ok()?;
        // The file name is authoritative: a sidecar claiming another id would let a crafted file alias an image.
        (sidecar.id == id && ImageFormat::parse(&sidecar.mime).is_ok()).then_some(sidecar)
    }

    fn sidecar_len(&self, id: &str) -> u64 {
        std::fs::metadata(self.sidecar_path(id)).map(|m| m.len()).unwrap_or(0)
    }

    fn image_path(&self, id: &str, format: ImageFormat) -> PathBuf {
        self.dir.join(format!("{id}.{}", format.extension()))
    }

    fn sidecar_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.json"))
    }

    fn log_refs(&self) -> Vec<LogRef> {
        self.log_file.as_deref().map(|p| last_log_refs(p, LOG_REFS)).unwrap_or_default()
    }
}

/// The last [`crate::activity::BREADCRUMBS_PER_ERROR`] breadcrumbs.
fn last_breadcrumbs(all: &[Breadcrumb]) -> Vec<Breadcrumb> {
    all[all.len().saturating_sub(crate::activity::BREADCRUMBS_PER_ERROR)..].to_vec()
}

/// Steps to reproduce ending in the error; none when nothing was recorded before it.
fn snapshot_steps(crumbs: &[Breadcrumb], message: &str) -> Vec<String> {
    if crumbs.is_empty() {
        return Vec::new();
    }
    crate::activity::steps(
        &last_breadcrumbs(crumbs),
        Some(&format!("Error: {}", truncate(message, MAX_MESSAGE))),
    )
}

/// The steps to reproduce of each snapshot that has any, one section per snapshot.
pub fn steps_document<'a>(sidecars: impl Iterator<Item = &'a Sidecar>) -> String {
    let mut out = String::new();
    for s in sidecars.filter(|s| !s.steps.is_empty()) {
        out.push_str(&format!("Snapshot {} ({}, {}): {}\n", s.id, s.created_at, s.reason, s.message));
        for step in &s.steps {
            out.push_str(step);
            out.push('\n');
        }
        out.push('\n');
    }
    out
}

/// The export destination must be a `.zip` in an existing folder.
pub(crate) fn checked_destination(dest: &Path) -> Result<PathBuf, SnapshotError> {
    let is_zip = dest.extension().is_some_and(|e| e.eq_ignore_ascii_case("zip"));
    if !is_zip {
        return Err(SnapshotError::BadDestination("the file name must end in .zip".into()));
    }
    let name = dest
        .file_name()
        .ok_or_else(|| SnapshotError::BadDestination("no file name".into()))?;
    let parent = match dest.parent() {
        Some(p) if !p.as_os_str().is_empty() => p,
        _ => Path::new("."),
    };
    let parent = parent
        .canonicalize()
        .map_err(|_| SnapshotError::BadDestination("the folder does not exist".into()))?;
    Ok(parent.join(name))
}

/// The last `max_bytes` of a file, starting at a line boundary.
pub(crate) fn tail_of(path: &Path, max_bytes: u64) -> Option<Vec<u8>> {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = std::fs::File::open(path).ok()?;
    let len = file.metadata().ok()?.len();
    let start = len.saturating_sub(max_bytes);
    // Start one byte early so a cut that falls exactly on a line start keeps that line.
    file.seek(SeekFrom::Start(start.saturating_sub(1))).ok()?;
    let mut buf = Vec::new();
    file.take(max_bytes + 1).read_to_end(&mut buf).ok()?;
    if start > 0 {
        if let Some(nl) = buf.iter().position(|&b| b == b'\n') {
            buf.drain(..=nl);
        }
    }
    Some(buf)
}

/// Line numbers (and CLEF timestamps) of the last `n` lines of IMT's log. Contents are not kept.
fn last_log_refs(path: &Path, n: usize) -> Vec<LogRef> {
    if std::fs::metadata(path).map(|m| m.len() > MAX_LOG_SCAN).unwrap_or(true) {
        return Vec::new();
    }
    let Ok(bytes) = std::fs::read(path) else { return Vec::new() };
    let text = String::from_utf8_lossy(&bytes);
    let lines: Vec<&str> = text.lines().collect();
    let first = lines.len().saturating_sub(n);
    lines[first..]
        .iter()
        .enumerate()
        .map(|(i, l)| LogRef {
            line: first + i + 1,
            time: serde_json::from_str::<serde_json::Value>(l)
                .ok()
                .and_then(|v| v.get("@t").and_then(|t| t.as_str().map(String::from))),
        })
        .collect()
}

/// Payload of `imt://error`.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct EngineError {
    pub kind: EngineErrorKind,
    pub message: String,
    /// The plugin concerned, when there is one.
    pub plugin: Option<String>,
}

#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum EngineErrorKind {
    PluginCrash,
    PluginQuarantined,
    /// A plugin stopped running: a required dependency became unavailable, or activation failed.
    PluginInactive,
    ImportCrash,
}

/// Tells the window whose workspace contained a failure (and its pop-out panels), so it can take an error
/// snapshot. Never fails.
pub fn emit_engine_error<R: tauri::Runtime>(app: &tauri::AppHandle<R>, window: &str, error: &EngineError) {
    // Logged here too (with the window's breadcrumbs, attached by the activity logger to errors), so a
    // contained failure is in the log even when no UI is listening.
    let kind = format!("{:?}", error.kind);
    let plugin = error.plugin.as_deref().unwrap_or("");
    let message = crate::activity::privacy::redact(&error.message);
    match error.kind {
        EngineErrorKind::PluginInactive => log::warn!(Kind = kind.as_str(), Plugin = plugin, Window = window; "{message}"),
        _ => log::error!(Kind = kind.as_str(), Plugin = plugin, Window = window; "{message}"),
    }
    crate::windows::emit_to_window(app, window, ERROR_EVENT, error);
    if let (EngineErrorKind::PluginQuarantined, Some(id)) = (&error.kind, &error.plugin) {
        use crate::notification_centre::centre::{NewNotice, NoticeAction, NoticeLink, NoticeScope, NoticeSeverity, NoticeSource};
        let notice = NewNotice::new(
            NoticeSource::Plugin,
            NoticeSeverity::Error,
            "A plugin was switched off",
            NoticeScope::Project,
        )
        .body(error.message.clone())
        .primary(NoticeAction::ResetPlugin { plugin_id: id.clone() })
        .dedupe(format!("plugin:{id}"))
        .link(NoticeLink {
            plugin_id: Some(id.clone()),
            ..NoticeLink::default()
        });
        crate::notification_centre::post(app, Some(window), notice, Some(crate::notify::NotificationKind::Attention));
    }
}

pub(crate) fn config_dir<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> Option<PathBuf> {
    crate::app_dirs::config(app).ok()
}

pub(crate) fn store<R: tauri::Runtime>(app: &tauri::AppHandle<R>) -> CmdResult<SnapshotStore> {
    let dir = crate::app_dirs::log(app).map_err(err)?;
    let limits = Limits::from(&crate::diagnostics::load(config_dir(app).as_deref()).snapshots);
    Ok(SnapshotStore::new(
        dir.join("snapshots"),
        Some(crate::logging::active_log_file(&dir)),
        limits,
    ))
}

/// Applies the retention limits at start-up (call from `setup`). Never fails.
pub fn prune_on_startup<R: tauri::Runtime>(app: &tauri::AppHandle<R>) {
    if let Ok(store) = store(app) {
        store.prune();
    }
}

/// Automatic snapshots follow the one switch in the diagnostics settings
/// (`diagnostics.json` → `snapshots.enabled`); a snapshot the user asks for is always taken.
fn allowed(settings: &crate::diagnostics::SnapshotLimits, context: &SnapshotContext) -> Result<(), SnapshotError> {
    if settings.enabled || context.manual {
        Ok(())
    } else {
        Err(SnapshotError::Disabled)
    }
}

/// Result of `save_error_snapshot`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveResult {
    pub id: String,
    /// First snapshot ever saved: the UI shows the "saved locally" notice.
    pub first_time: bool,
}

#[tauri::command]
pub fn save_error_snapshot<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    webview: tauri::Webview<R>,
    image: String,
    mime: String,
    context: SnapshotContext,
) -> CmdResult<SaveResult> {
    save_snapshot(&app, webview.label(), image, mime, context)
}

/// `save_error_snapshot` for the window `label`: the snapshot records that window's latest activity.
pub(crate) fn save_snapshot<R: tauri::Runtime>(
    app: &tauri::AppHandle<R>,
    label: &str,
    image: String,
    mime: String,
    mut context: SnapshotContext,
) -> CmdResult<SaveResult> {
    let app = app.clone();
    context.breadcrumbs = crate::activity::breadcrumbs(Some(label), crate::activity::BREADCRUMBS_PER_ERROR);
    allowed(&crate::diagnostics::load(config_dir(&app).as_deref()).snapshots, &context).map_err(|e| e.to_string())?;
    let (format, bytes) = validate_image(&mime, &image).map_err(|e| e.to_string())?;
    let store = store(&app)?;
    let first_time = !store.dir().join(NOTICE_MARKER).exists();
    let id = store
        .save(format, &bytes, &context, &Environment::current())
        .map_err(|e| e.to_string())?;
    if first_time {
        store.take_first_time();
    }
    publish_snapshot_taken(&app, &id);
    // Reason and size only: no message text, no image data.
    log::info!(SnapshotId = id.as_str(), Reason = context.reason.as_str(), Bytes = bytes.len() as u64; "Error snapshot saved");
    Ok(SaveResult { id, first_time })
}

/// Tells every window's plugins that a snapshot was saved (`imt_core::events::SnapshotTaken`).
fn publish_snapshot_taken<R: tauri::Runtime>(app: &tauri::AppHandle<R>, id: &str) {
    use tauri::Manager;
    if let Some(windows) = app.try_state::<crate::windows::Windows>() {
        for w in windows.all() {
            w.publish(imt_core::events::SnapshotTaken { id: id.to_string() });
        }
    }
}

#[tauri::command]
pub fn list_error_snapshots<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> CmdResult<Vec<SnapshotInfo>> {
    let mut all = store(&app)?.list();
    all.reverse(); // newest first
    Ok(all)
}

/// The image as a `data:` URL (the CSP allows data images, not files or blobs).
#[tauri::command]
pub fn read_error_snapshot<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: String) -> CmdResult<String> {
    let (format, bytes) = store(&app)?.read_image(&id).map_err(|e| e.to_string())?;
    Ok(format!("data:{};base64,{}", format.mime(), STANDARD.encode(bytes)))
}

#[tauri::command]
pub fn delete_error_snapshot<R: tauri::Runtime>(app: tauri::AppHandle<R>, id: String) -> CmdResult<()> {
    store(&app)?.delete(&id).map_err(|e| e.to_string())?;
    log::info!(SnapshotId = id.as_str(); "Error snapshot deleted");
    Ok(())
}

/// Shows the snapshots folder in the OS file manager; returns its path.
#[tauri::command]
pub fn open_snapshots_folder<R: tauri::Runtime>(app: tauri::AppHandle<R>) -> CmdResult<String> {
    let store = store(&app)?;
    std::fs::create_dir_all(store.dir()).map_err(err)?;
    let open = crate::opener::open_folder(crate::opener::Os::CURRENT, store.dir());
    crate::opener::start(&open).map_err(|e| format!("could not open the folder ({e}); it is at {}", store.dir().display()))?;
    Ok(store.dir().to_string_lossy().into_owned())
}

/// Zips the chosen snapshots and the recent IMT log to `path` (chosen in the save dialog) as a background
/// export job; returns the job at once ("Show in folder" when the zip is there).
#[tauri::command]
pub fn export_diagnostics<R: tauri::Runtime>(
    app: tauri::AppHandle<R>,
    grants: crate::paths::WindowGrants,
    path: String,
    ids: Vec<String>,
    overwrite: Option<bool>,
) -> CmdResult<u64> {
    let target = grants.write(&path, &["zip"])?.path().to_path_buf();
    imt_core::overwrite::check_overwrite(Path::new(&path), overwrite.unwrap_or(false))?;
    let store = store(&app)?;
    let path = target.to_string_lossy().into_owned();
    let spec = crate::jobs::export_spec("Export diagnostics to", None, &target);
    crate::jobs::start_file(&app, spec, &target, move |_| {
        let count = store.export(&ids, Path::new(&path)).map_err(|e| e.to_string())?;
        log::info!(Files = count as u64; "Diagnostics exported");
        Ok(format!("{count} files zipped."))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        static N: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let d = std::env::temp_dir().join(format!("imt-snap-{tag}-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    const PNG: [u8; 12] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 1, 2, 3, 4];

    fn png(extra: usize) -> Vec<u8> {
        let mut v = PNG.to_vec();
        v.extend(std::iter::repeat_n(7u8, extra));
        v
    }

    fn ctx(reason: &str) -> SnapshotContext {
        SnapshotContext {
            reason: reason.into(),
            message: "boom".into(),
            stack: Some("at x".into()),
            view: Some("grid".into()),
            manual: false,
            breadcrumbs: Vec::new(),
        }
    }

    fn env() -> Environment {
        Environment {
            app_version: "1.2.3".into(),
            os: "test-os".into(),
            arch: "x".into(),
        }
    }

    fn store_in(d: &Path, limits: Limits) -> SnapshotStore {
        SnapshotStore::new(d.join("snapshots"), Some(d.join("imt.log")), limits)
    }

    #[test]
    fn validates_mime_encoding_magic_and_size() {
        let ok = STANDARD.encode(PNG);
        assert_eq!(validate_image("image/png", &ok).unwrap().0, ImageFormat::Png);
        assert_eq!(validate_image("image/png", &format!(" {ok}\n")).unwrap().1, PNG);
        let jpeg = STANDARD.encode([0xFF, 0xD8, 0xFF, 0xE0, 0]);
        assert_eq!(validate_image("image/jpeg", &jpeg).unwrap().0, ImageFormat::Jpeg);

        assert!(matches!(validate_image("image/gif", &ok), Err(SnapshotError::BadMime(_))));
        assert!(matches!(validate_image("text/html", &ok), Err(SnapshotError::BadMime(_))));
        assert_eq!(validate_image("image/png", "!!!not base64!!!"), Err(SnapshotError::BadEncoding));
        // A JPEG labelled PNG, and the other way round, and plain text.
        assert_eq!(validate_image("image/png", &jpeg), Err(SnapshotError::BadMagic));
        assert_eq!(validate_image("image/jpeg", &ok), Err(SnapshotError::BadMagic));
        assert_eq!(
            validate_image("image/png", &STANDARD.encode("<html>")),
            Err(SnapshotError::BadMagic)
        );
        assert_eq!(validate_image("image/png", ""), Err(SnapshotError::BadMagic));

        // Exactly at the limit passes, one byte over fails (before and after decoding).
        assert!(validate_image("image/png", &STANDARD.encode(png(MAX_IMAGE_BYTES - PNG.len()))).is_ok());
        let over = STANDARD.encode(png(MAX_IMAGE_BYTES - PNG.len() + 1));
        assert!(matches!(validate_image("image/png", &over), Err(SnapshotError::TooLarge(_))));
        let huge = "A".repeat(MAX_IMAGE_BYTES * 2);
        assert!(matches!(validate_image("image/png", &huge), Err(SnapshotError::TooLarge(_))));
        assert!(SnapshotError::TooLarge(5).to_string().contains("too large"));
    }

    #[test]
    fn ids_are_generated_and_checked_strictly() {
        let id = new_id();
        assert!(is_valid_id(&id), "{id}");
        assert_ne!(id, new_id());
        for bad in [
            "",
            "../../etc/passwd",
            "20261017T083000123-1a2b3c4d/../x",
            "20261017T083000123-1A2B3C4D", // upper case hex
            "20261017T083000123-1a2b3c4",
            "20261017T083000123-1a2b3c4dd",
            "20261017X083000123-1a2b3c4d",
            "..\\..\\windows\\x.png",
            "20261017T083000123-1a2b3c4d.png",
            "２0261017T083000123-1a2b3c4d",
        ] {
            assert!(!is_valid_id(bad), "{bad}");
        }
    }

    #[test]
    fn saves_lists_reads_and_deletes() {
        let d = temp("crud");
        std::fs::write(
            d.join("imt.log"),
            "{\"@t\":\"2026-10-17T08:00:00Z\",\"@m\":\"secret text\"}\nplain line\n{\"@t\":\"2026-10-17T08:00:02Z\"}\n",
        )
        .unwrap();
        let store = store_in(&d, Limits::default());
        assert!(store.list().is_empty());
        let a = store.save(ImageFormat::Png, &png(10), &ctx("panel"), &env()).unwrap();
        let b = store
            .save(ImageFormat::Jpeg, &[0xFF, 0xD8, 0xFF, 1], &ctx("manual"), &env())
            .unwrap();
        assert!(is_valid_id(&a) && is_valid_id(&b));

        let files: Vec<String> = std::fs::read_dir(store.dir())
            .unwrap()
            .flatten()
            .map(|e| e.file_name().into_string().unwrap())
            .collect();
        assert!(files.contains(&format!("{a}.png")) && files.contains(&format!("{a}.json")));
        assert!(files.contains(&format!("{b}.jpg")) && files.contains(&format!("{b}.json")));

        let all = store.list();
        assert_eq!(all.len(), 2);
        let first = all.iter().find(|i| i.id == a).unwrap();
        assert_eq!(
            (first.reason.as_str(), first.message.as_str(), first.mime.as_str()),
            ("panel", "boom", "image/png")
        );
        assert_eq!(
            (first.app_version.as_str(), first.os.as_str(), first.bytes),
            ("1.2.3", "test-os", 22)
        );

        // The sidecar references the log lines by number and time, never by content.
        let sidecar: Sidecar = serde_json::from_str(&std::fs::read_to_string(store.sidecar_path(&a)).unwrap()).unwrap();
        assert_eq!(
            sidecar.log_refs,
            vec![
                LogRef {
                    line: 1,
                    time: Some("2026-10-17T08:00:00Z".into())
                },
                LogRef { line: 2, time: None },
                LogRef {
                    line: 3,
                    time: Some("2026-10-17T08:00:02Z".into())
                },
            ]
        );
        assert!(!std::fs::read_to_string(store.sidecar_path(&a)).unwrap().contains("secret text"));

        let (format, bytes) = store.read_image(&a).unwrap();
        assert_eq!((format, bytes), (ImageFormat::Png, png(10)));
        store.delete(&a).unwrap();
        assert_eq!(store.list().len(), 1);
        assert_eq!(store.delete(&a), Err(SnapshotError::NotFound(a.clone())));
        assert_eq!(store.read_image(&a).unwrap_err(), SnapshotError::NotFound(a));
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn traversal_and_unlisted_ids_are_rejected() {
        let d = temp("trav");
        let store = store_in(&d, Limits::default());
        store.save(ImageFormat::Png, &png(1), &ctx("x"), &env()).unwrap();
        std::fs::write(d.join("victim.png"), png(1)).unwrap();
        for bad in ["../victim", "../../victim.png", "/etc/passwd", "C:\\x", ""] {
            assert!(matches!(store.delete(bad), Err(SnapshotError::BadId(_))), "{bad}");
            assert!(matches!(store.read_image(bad), Err(SnapshotError::BadId(_))), "{bad}");
            assert!(
                matches!(store.export(&[bad.to_string()], &d.join("o.zip")), Err(SnapshotError::BadId(_))),
                "{bad}"
            );
        }
        // Well-formed but not in the listing.
        let ghost = "20200101T000000000-deadbeef".to_string();
        assert_eq!(store.delete(&ghost), Err(SnapshotError::NotFound(ghost.clone())));
        // A sidecar that names another id, or an unsupported mime, is not listed.
        std::fs::write(store.sidecar_path(&ghost), "{\"id\":\"20200101T000000000-00000000\"}").unwrap();
        assert!(store.list().iter().all(|i| i.id != ghost));
        assert!(d.join("victim.png").exists());
        assert!(!d.join("o.zip").exists());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn prunes_by_count_keeping_the_newest() {
        let d = temp("count");
        let store = store_in(
            &d,
            Limits {
                max_count: 3,
                max_bytes: u64::MAX,
                max_age: None,
            },
        );
        let mut ids = Vec::new();
        for i in 0..6 {
            ids.push(store.save(ImageFormat::Png, &png(i), &ctx("x"), &env()).unwrap());
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let kept: Vec<String> = store.list().into_iter().map(|i| i.id).collect();
        assert_eq!(kept, ids[3..].to_vec());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn prunes_by_size_but_always_keeps_the_newest() {
        let d = temp("size");
        let store = store_in(
            &d,
            Limits {
                max_count: 50,
                max_bytes: 3_000,
                max_age: None,
            },
        );
        let mut ids = Vec::new();
        for _ in 0..4 {
            ids.push(store.save(ImageFormat::Png, &png(1_000), &ctx("x"), &env()).unwrap());
            std::thread::sleep(std::time::Duration::from_millis(3));
        }
        let kept: Vec<String> = store.list().into_iter().map(|i| i.id).collect();
        assert!(kept.len() < 4 && !kept.is_empty());
        assert_eq!(kept.last(), ids.last());
        let total: u64 = std::fs::read_dir(store.dir())
            .unwrap()
            .flatten()
            .map(|e| e.metadata().unwrap().len())
            .sum();
        assert!(total <= 3_000, "{total}");

        // One image above the budget is still kept.
        let tiny = store_in(
            &temp("tiny"),
            Limits {
                max_count: 50,
                max_bytes: 10,
                max_age: None,
            },
        );
        let id = tiny.save(ImageFormat::Png, &png(500), &ctx("x"), &env()).unwrap();
        assert_eq!(tiny.list().len(), 1);
        assert_eq!(tiny.list()[0].id, id);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn caps_stored_text() {
        let d = temp("caps");
        let store = store_in(&d, Limits::default());
        let long = SnapshotContext {
            reason: "r".repeat(500),
            message: "m".repeat(5_000),
            stack: Some("s".repeat(50_000)),
            view: Some("v".repeat(500)),
            manual: true,
            ..Default::default()
        };
        store.save(ImageFormat::Png, &png(1), &long, &env()).unwrap();
        let info = &store.list()[0];
        assert_eq!(info.reason.chars().count(), MAX_REASON + 1);
        assert_eq!(info.message.chars().count(), MAX_MESSAGE + 1);
        assert_eq!(info.stack.as_ref().unwrap().chars().count(), MAX_STACK + 1);
        assert!(info.manual);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn exports_a_zip_with_images_sidecars_and_the_log() {
        let d = temp("zip");
        std::fs::write(d.join("imt.log"), "{\"@t\":\"2026-10-17T08:00:00Z\"}\nsecond\n").unwrap();
        let store = store_in(&d, Limits::default());
        let a = store.save(ImageFormat::Png, &png(5), &ctx("a"), &env()).unwrap();
        let b = store.save(ImageFormat::Jpeg, &[0xFF, 0xD8, 0xFF, 9], &ctx("b"), &env()).unwrap();
        let _unchosen = store.save(ImageFormat::Png, &png(6), &ctx("c"), &env()).unwrap();

        let out = d.join("out.zip");
        let n = store.export(&[a.clone(), b.clone()], &out).unwrap();
        assert_eq!(n, 5);
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let mut names: Vec<String> = zip.file_names().map(|n| n.unwrap().into_owned()).collect();
        names.sort();
        let mut want = vec![
            format!("snapshots/{a}.png"),
            format!("snapshots/{a}.json"),
            format!("snapshots/{b}.jpg"),
            format!("snapshots/{b}.json"),
            "logs/imt.log".to_string(),
        ];
        want.sort();
        assert_eq!(names, want);
        let mut img = Vec::new();
        std::io::Read::read_to_end(&mut zip.by_name(&format!("snapshots/{a}.png")).unwrap(), &mut img).unwrap();
        assert_eq!(img, png(5));
        let mut log = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("logs/imt.log").unwrap(), &mut log).unwrap();
        assert!(log.contains("second"));

        // Without a log file the zip simply has no log entry.
        let no_log = SnapshotStore::new(d.join("snapshots"), None, Limits::default());
        assert_eq!(no_log.export(&[a], &d.join("two.zip")).unwrap(), 2);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn export_destination_is_checked_and_failures_leave_nothing() {
        let d = temp("dest");
        let store = store_in(&d, Limits::default());
        let id = store.save(ImageFormat::Png, &png(1), &ctx("x"), &env()).unwrap();
        for bad in [
            d.join("out.txt"),
            d.join("out"),
            d.join("missing-dir").join("out.zip"),
            PathBuf::from(""),
        ] {
            assert!(
                matches!(store.export(std::slice::from_ref(&id), &bad), Err(SnapshotError::BadDestination(_))),
                "{bad:?}"
            );
        }
        assert_eq!(
            store.export(&["20200101T000000000-deadbeef".into()], &d.join("o.zip")),
            Err(SnapshotError::NotFound("20200101T000000000-deadbeef".into()))
        );
        assert!(!d.join("o.zip").exists());
        // The image vanished between listing and zipping: no partial archive is left behind.
        let gone = store.save(ImageFormat::Png, &png(2), &ctx("y"), &env()).unwrap();
        let chosen = vec![store.find(&gone).unwrap()];
        std::fs::remove_file(store.image_path(&gone, ImageFormat::Png)).unwrap();
        assert!(store.write_zip(&chosen, &d.join("p.zip")).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn log_helpers_cope_with_missing_and_large_files() {
        let d = temp("log");
        assert!(last_log_refs(&d.join("nope.log"), 5).is_empty());
        assert!(tail_of(&d.join("nope.log"), 10).is_none());
        std::fs::write(d.join("a.log"), "one\ntwo\nthree\nfour\n").unwrap();
        assert_eq!(tail_of(&d.join("a.log"), 1000).unwrap(), b"one\ntwo\nthree\nfour\n");
        // Starts at a line boundary when cut.
        assert_eq!(tail_of(&d.join("a.log"), 11).unwrap(), b"three\nfour\n");
        let refs = last_log_refs(&d.join("a.log"), 2);
        assert_eq!(refs.iter().map(|r| r.line).collect::<Vec<_>>(), vec![3, 4]);
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn engine_errors_serialise_for_the_ui() {
        let e = EngineError {
            kind: EngineErrorKind::PluginQuarantined,
            message: "x".into(),
            plugin: Some("p".into()),
        };
        assert_eq!(
            serde_json::to_value(&e).unwrap(),
            serde_json::json!({ "kind": "plugin-quarantined", "message": "x", "plugin": "p" })
        );
        assert_eq!(serde_json::to_value(EngineErrorKind::ImportCrash).unwrap(), "import-crash");
        assert_eq!(serde_json::to_value(EngineErrorKind::PluginCrash).unwrap(), "plugin-crash");
        assert_eq!(ERROR_EVENT, "imt://error");
    }

    #[test]
    fn prunes_by_age_with_the_shared_limits() {
        let d = temp("age");
        let limits = Limits::from(&crate::diagnostics::SnapshotLimits {
            enabled: true,
            max_count: 500,
            max_total_mb: 2000,
            max_age_days: 2,
        });
        assert_eq!(limits.max_age, Some(std::time::Duration::from_secs(2 * 86_400)));
        let store = store_in(&d, limits);
        let a = store.save(ImageFormat::Png, &png(1), &ctx("x"), &env()).unwrap();
        let b = store.save(ImageFormat::Png, &png(2), &ctx("y"), &env()).unwrap();
        store.prune_at(Utc::now() + chrono::Duration::days(1));
        assert_eq!(store.list().len(), 2);
        // Three days later both are past the limit, even the newest.
        store.prune_at(Utc::now() + chrono::Duration::days(3));
        assert!(store.list().is_empty(), "{a} {b}");
        // 0 means no age limit.
        let forever = Limits::from(&crate::diagnostics::SnapshotLimits {
            max_age_days: 0,
            ..Default::default()
        });
        assert_eq!(forever.max_age, None);
        let store = store_in(&d, forever);
        store.save(ImageFormat::Png, &png(1), &ctx("x"), &env()).unwrap();
        store.prune_at(Utc::now() + chrono::Duration::days(3650));
        assert_eq!(store.list().len(), 1);
        // Limits convert from the shared settings: count and megabytes.
        let l = Limits::from(&crate::diagnostics::SnapshotLimits {
            max_count: 7,
            max_total_mb: 12,
            ..Default::default()
        });
        assert_eq!((l.max_count, l.max_bytes), (7, 12 * 1024 * 1024));
        assert_eq!(Limits::default().max_count, 50);
        std::fs::remove_dir_all(d).unwrap();
    }

    fn crumb(op: &str) -> Breadcrumb {
        Breadcrumb {
            time: "2026-10-17T08:00:00Z".into(),
            operation: op.into(),
            trigger: "menu".into(),
            operation_id: Some("op-1".into()),
            outcome: None,
            error_code: None,
            duration_ms: None,
            view: None,
            window: "main".into(),
            params: [("Title".to_string(), "Export CSV".to_string())].into(),
        }
    }

    #[test]
    fn snapshots_keep_breadcrumbs_and_steps_and_export_them() {
        let d = temp("crumbs");
        let store = store_in(&d, Limits::default());
        let mut c = ctx("panel-failed");
        c.breadcrumbs = (0..40).map(|i| crumb(&format!("op{i}"))).collect();
        c.breadcrumbs.push(crumb("export.csv"));
        let id = store.save(ImageFormat::Png, &png(1), &c, &env()).unwrap();
        let info = store.list().into_iter().find(|i| i.id == id).unwrap();
        assert_eq!(info.breadcrumbs.len(), crate::activity::BREADCRUMBS_PER_ERROR);
        assert_eq!(info.breadcrumbs.last().unwrap().operation, "export.csv");
        assert_eq!(info.steps.last().unwrap(), &format!("{}. Error: boom", info.steps.len()));
        assert!(
            info.steps.iter().any(|s| s.contains("Clicked 'Export CSV' from the menu")),
            "{:?}",
            info.steps
        );
        // Without breadcrumbs there are no steps.
        let plain = store.save(ImageFormat::Png, &png(2), &ctx("x"), &env()).unwrap();
        assert!(store.list().into_iter().find(|i| i.id == plain).unwrap().steps.is_empty());

        let out = d.join("crumbs.zip");
        assert_eq!(store.export(&[id.clone(), plain], &out).unwrap(), 5);
        let mut zip = zip::ZipArchive::new(std::fs::File::open(&out).unwrap()).unwrap();
        let mut steps = String::new();
        std::io::Read::read_to_string(&mut zip.by_name("steps-to-reproduce.txt").unwrap(), &mut steps).unwrap();
        assert!(
            steps.starts_with(&format!("Snapshot {id}")) && steps.contains("Error: boom"),
            "{steps}"
        );
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn a_sidecar_without_breadcrumbs_or_steps_still_lists() {
        let d = temp("partial");
        let store = store_in(&d, Limits::default());
        let id = store.save(ImageFormat::Png, &png(1), &ctx("x"), &env()).unwrap();
        let path = store.sidecar_path(&id);
        let mut v: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&path).unwrap()).unwrap();
        let o = v.as_object_mut().unwrap();
        o.remove("breadcrumbs");
        o.remove("steps");
        std::fs::write(&path, v.to_string()).unwrap();
        let info = store.list().into_iter().find(|i| i.id == id).unwrap();
        assert!(info.breadcrumbs.is_empty() && info.steps.is_empty());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn notice_is_reported_once() {
        let d = temp("notice");
        let store = store_in(&d, Limits::default());
        std::fs::create_dir_all(store.dir()).unwrap();
        assert!(store.take_first_time());
        assert!(!store.take_first_time());
        // The marker is not mistaken for a snapshot.
        assert!(store.list().is_empty());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn saved_snapshots_are_announced_to_every_windows_plugins() {
        let app = crate::test_support::test_app(None);
        let main = crate::test_support::main_window(&app);
        let (tx, rx) = std::sync::mpsc::channel();
        main.events
            .clone()
            .unwrap()
            .subscribe::<imt_core::events::SnapshotTaken>("test.snapshots", move |e| {
                let _ = tx.send(e.id.clone());
            })
            .unwrap();
        publish_snapshot_taken(app.handle(), "snap-42");
        assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap(), "snap-42");
        // Without window state (e.g. very early at startup) nothing happens.
        publish_snapshot_taken(tauri::test::mock_app().handle(), "x");
    }

    #[test]
    fn commands_run_against_a_mock_app() {
        let app = crate::test_support::test_app(None);
        let handle = app.handle().clone();
        let image = STANDARD.encode(png(20));
        let saved = save_snapshot(&handle, "main", image.clone(), "image/png".into(), ctx("cmd")).unwrap();
        let listed = list_error_snapshots(handle.clone()).unwrap();
        assert!(listed.iter().any(|i| i.id == saved.id));
        assert!(read_error_snapshot(handle.clone(), saved.id.clone())
            .unwrap()
            .starts_with("data:image/png;base64,"));
        let bad = save_snapshot(&handle, "main", "AAAA".into(), "image/png".into(), ctx("bad"));
        assert!(bad.is_err());
        let out = std::env::temp_dir().join(format!("imt-snap-cmd-{}.zip", std::process::id()));
        let grants = || crate::paths::WindowGrants::of(&handle, "main");
        let refused = export_diagnostics(handle.clone(), grants(), out.to_string_lossy().into(), vec![], None);
        assert!(refused.unwrap_err().contains("only writes where you choose"));
        crate::paths::grant(
            &handle,
            "main",
            &out,
            crate::paths::Access::Write,
            crate::paths::GrantSource::Dialog,
        )
        .unwrap();
        let job = export_diagnostics(
            handle.clone(),
            grants(),
            out.to_string_lossy().into(),
            vec![saved.id.clone()],
            Some(true),
        )
        .unwrap();
        let line = crate::test_support::job_done(&app, job);
        let n: usize = line.split(' ').next().unwrap().parse().unwrap();
        assert!(n >= 2, "{line}");
        let refused = export_diagnostics(handle.clone(), grants(), out.to_string_lossy().into(), vec![saved.id.clone()], None);
        assert_eq!(
            refused.unwrap_err(),
            crate::error::file_exists(format!("{} already exists; confirm to replace it", out.display()))
        );
        let _ = std::fs::remove_file(out);
        prune_on_startup(&handle);
        delete_error_snapshot(handle.clone(), saved.id).unwrap();
        assert!(delete_error_snapshot(handle.clone(), "../x".into()).is_err());
        emit_engine_error(
            &handle,
            "main",
            &EngineError {
                kind: EngineErrorKind::PluginCrash,
                message: "m".into(),
                plugin: None,
            },
        );
    }

    #[test]
    fn snapshots_attach_todays_log_file() {
        let name = crate::logging::active_log_file(std::path::Path::new("logs"));
        let name = name.file_name().unwrap().to_string_lossy().into_owned();
        assert!(
            name.starts_with(&format!("{}.", crate::diagnostics::LOG_NAME)) && name.ends_with(".log"),
            "{name}"
        );
    }

    #[test]
    fn only_the_diagnostics_switch_turns_automatic_snapshots_off() {
        let d = temp("switch");
        let mut settings = crate::diagnostics::load(Some(&d)).snapshots;
        assert!(settings.enabled, "on by default");
        let mut c = ctx("panel-failed");
        assert!(allowed(&settings, &c).is_ok());
        settings.enabled = false;
        assert_eq!(
            allowed(&settings, &c).unwrap_err().to_string(),
            "error snapshots are turned off in the settings"
        );
        c.manual = true;
        assert!(allowed(&settings, &c).is_ok(), "a snapshot the user asks for is always taken");
    }
}
