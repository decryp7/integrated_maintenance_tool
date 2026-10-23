//! Shareable `.debriefz` bundles: a zip with the project, the effective profile,
//! the logs (raw files, or the filtered events as CLEF), reports, dashboards
//! and a `manifest.json` listing every file with its size and SHA-256.
//!
//! Writing lives in the workspace (`Workspace::write_bundle`); this module
//! holds the manifest and opening: a bundle is extracted into a folder of its
//! own with the archive limits, rejecting any entry whose path would leave it
//! (zip slip), and checked against its manifest.

use crate::archive::{MAX_ARCHIVE_BYTES, MAX_ENTRY_BYTES};
use crate::query::Query;
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};
use std::path::{Component, Path, PathBuf};

/// `manifest.json` `format` of a shareable bundle (collector bundles have none).
pub const BUNDLE_FORMAT: &str = "debrief-share";
/// Current manifest schema; newer bundles are refused with a message.
pub const BUNDLE_SCHEMA: u32 = 1;
pub const MANIFEST_NAME: &str = "manifest.json";
pub const PROJECT_NAME: &str = "project.debriefproj";
/// Most files in a bundle.
pub const MAX_ENTRIES: usize = 10_000;
/// Largest text file redacted when raw files are bundled with redaction.
pub const MAX_REDACTED_FILE: u64 = 256 << 20;

/// What the bundle carries of the logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BundleContent {
    /// The opened files as they are (text files redacted when asked;
    /// binary files and archives are then left out).
    #[default]
    Raw,
    /// The events matching the query, as CLEF.
    Events,
}

/// `create_bundle` / `estimate_bundle` options.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleOptions {
    #[serde(default = "default_name")]
    pub name: String,
    #[serde(default)]
    pub content: BundleContent,
    /// The events to include (for `events`).
    #[serde(default)]
    pub query: Query,
    #[serde(default)]
    pub redact: bool,
    /// The UI state saved in the bundle's project.
    #[serde(default)]
    pub ui: serde_json::Value,
    /// Reports (JSON, checked with `Report::parse`).
    #[serde(default)]
    pub reports: Vec<serde_json::Value>,
}

fn default_name() -> String {
    "Debrief bundle".into()
}

/// What a file in the bundle is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EntryKind {
    Project,
    Profile,
    Log,
    Events,
    Report,
    Dashboards,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleEntry {
    pub path: String,
    pub kind: EntryKind,
    pub bytes: u64,
    pub sha256: String,
}

/// A source left out of the bundle, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SkippedSource {
    pub path: String,
    pub reason: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleManifest {
    pub format: String,
    pub schema: u32,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub created_at: i64,
    #[serde(default)]
    pub debrief_version: String,
    #[serde(default)]
    pub content: BundleContent,
    #[serde(default)]
    pub redacted: bool,
    /// Matches per redaction rule.
    #[serde(default)]
    pub redaction: crate::redact::Counts,
    /// Events written (for `events` bundles).
    #[serde(default)]
    pub events: Option<usize>,
    #[serde(default)]
    pub entries: Vec<BundleEntry>,
    #[serde(default)]
    pub skipped: Vec<SkippedSource>,
}

/// One part of an estimate.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct EstimatePart {
    pub name: String,
    pub bytes: u64,
}

/// What a bundle would contain (uncompressed sizes; zip compresses text well).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BundleEstimate {
    pub files: usize,
    pub bytes: u64,
    pub events: usize,
    pub parts: Vec<EstimatePart>,
    pub skipped: Vec<SkippedSource>,
}

/// SHA-256 as lower-case hex.
pub fn sha256_hex(bytes: &[u8]) -> String {
    hex(&Sha256::digest(bytes))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// A writer that counts and hashes what passes through.
pub struct HashingWriter<W: Write> {
    inner: W,
    hasher: Sha256,
    bytes: u64,
}

impl<W: Write> HashingWriter<W> {
    pub fn new(inner: W) -> Self {
        HashingWriter {
            inner,
            hasher: Sha256::new(),
            bytes: 0,
        }
    }

    /// Bytes written and their SHA-256 hex; gives the inner writer back.
    pub fn finish(self) -> (W, u64, String) {
        (self.inner, self.bytes, hex(&self.hasher.finalize()))
    }
}

impl<W: Write> Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hasher.update(&buf[..n]);
        self.bytes += n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// A bundle extracted for opening.
#[derive(Debug, Clone, PartialEq)]
pub struct ExtractedBundle {
    /// The folder it was extracted into.
    pub dir: PathBuf,
    /// The share manifest; `None` for other `.debriefz` files (collector bundles).
    pub manifest: Option<BundleManifest>,
    /// `project.debriefproj`, when the bundle has one.
    pub project: Option<PathBuf>,
}

/// The path of a zip entry inside the extraction folder, or an error when it
/// is absolute, climbs out (`..`), names a drive or is otherwise unsafe.
pub fn safe_entry_path(name: &str) -> Result<PathBuf> {
    let unsafe_path = || {
        anyhow!(
            "the bundle contains an unsafe path \"{}\"",
            name.chars().take(200).collect::<String>()
        )
    };
    if name.is_empty() || name.len() > 1024 || name.contains('\0') || name.contains('\\') || name.starts_with('/') {
        return Err(unsafe_path());
    }
    let mut out = PathBuf::new();
    for part in Path::new(name).components() {
        match part {
            Component::Normal(p) => {
                let p = p.to_str().ok_or_else(unsafe_path)?;
                if p.contains(':') || p.trim_end_matches([' ', '.']).is_empty() {
                    return Err(unsafe_path());
                }
                out.push(p);
            }
            Component::CurDir => {}
            _ => return Err(unsafe_path()),
        }
    }
    if out.as_os_str().is_empty() {
        return Err(unsafe_path());
    }
    Ok(out)
}

/// Extracts `bundle` into a new folder under `dest_root` (replacing an
/// earlier extraction of the same bundle) and checks it against its manifest.
pub fn extract(bundle: &Path, dest_root: &Path) -> Result<ExtractedBundle> {
    extract_capped(
        bundle,
        dest_root,
        Limits {
            entry: MAX_ENTRY_BYTES,
            total: MAX_ARCHIVE_BYTES,
            entries: MAX_ENTRIES,
        },
    )
}

/// Size and count limits of an extraction.
#[derive(Clone, Copy)]
struct Limits {
    entry: u64,
    total: u64,
    entries: usize,
}

/// Fails when `n` bytes exceed `max` (`what` names the file, or the whole bundle).
fn cap(what: &str, n: u64, max: u64) -> Result<()> {
    if n > max {
        bail!("{what} expands to more than {} MiB", max >> 20);
    }
    Ok(())
}

const WHOLE: &str = "the bundle";

fn extract_capped(bundle: &Path, dest_root: &Path, limits: Limits) -> Result<ExtractedBundle> {
    let file = std::fs::File::open(bundle).with_context(|| format!("cannot open {}", bundle.display()))?;
    let mut zip = zip::ZipArchive::new(file).with_context(|| format!("{} is not a Debrief bundle (not a zip file)", bundle.display()))?;
    if zip.len() > limits.entries {
        bail!("the bundle has more than {} files", limits.entries);
    }
    // Check every name and the declared sizes before writing anything.
    let mut names = Vec::with_capacity(zip.len());
    let mut declared = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).context("invalid zip entry")?;
        let name = entry.name().context("invalid zip entry name")?.to_string();
        if entry.is_dir() {
            continue;
        }
        let path = safe_entry_path(&name)?;
        cap(&name, entry.size(), limits.entry)?;
        declared = declared.saturating_add(entry.size());
        names.push((i, name, path));
    }
    cap(WHOLE, declared, limits.total)?;
    let dir = dest_root.join(folder_name(bundle));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("cannot replace {}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut total = 0u64;
    let mut hashes = std::collections::HashMap::new();
    for (i, name, rel) in names {
        let entry = zip.by_index(i).context("invalid zip entry")?;
        let target = dir.join(&rel);
        std::fs::create_dir_all(target.parent().unwrap_or(&dir))?;
        let mut out = HashingWriter::new(std::fs::File::create(&target).with_context(|| format!("cannot write {}", target.display()))?);
        // The declared size may lie: cap what is actually read.
        let n = std::io::copy(&mut entry.take(limits.entry + 1), &mut out).with_context(|| format!("cannot extract {name}"))?;
        cap(&name, n, limits.entry)?;
        total += n;
        cap(WHOLE, total, limits.total)?;
        let (_, bytes, sha) = out.finish();
        hashes.insert(name.clone(), (bytes, sha));
    }
    let manifest = read_manifest(&dir)?;
    if let Some(m) = &manifest {
        for e in &m.entries {
            match hashes.get(&e.path) {
                Some((bytes, sha)) if *bytes == e.bytes && *sha == e.sha256 => {}
                Some(_) => bail!("the bundle is damaged: {} does not match its manifest", e.path),
                None => bail!("the bundle is incomplete: {} is missing", e.path),
            }
        }
    }
    let project = Some(dir.join(PROJECT_NAME)).filter(|p| p.is_file());
    Ok(ExtractedBundle { dir, manifest, project })
}

/// The share manifest of an extracted bundle; `None` when there is none or
/// it is not a share manifest (a collector bundle).
fn read_manifest(dir: &Path) -> Result<Option<BundleManifest>> {
    let Ok(text) = std::fs::read_to_string(dir.join(MANIFEST_NAME)) else {
        return Ok(None);
    };
    let Ok(value) = serde_json::from_str::<serde_json::Value>(&text) else {
        return Ok(None);
    };
    if value.get("format").and_then(|f| f.as_str()) != Some(BUNDLE_FORMAT) {
        return Ok(None);
    }
    let m: BundleManifest = serde_json::from_value(value).context("the bundle's manifest is damaged")?;
    if m.schema > BUNDLE_SCHEMA {
        bail!(
            "this bundle was made by a newer Debrief (format {}); please update Debrief",
            m.schema
        );
    }
    Ok(Some(m))
}

/// `<file stem>-<hash of its path>`: stable per bundle, distinct across bundles.
fn folder_name(bundle: &Path) -> String {
    let stem: String = bundle
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .take(60)
        .collect();
    let id = sha256_hex(bundle.to_string_lossy().as_bytes());
    format!("{}-{}", if stem.is_empty() { "bundle" } else { &stem }, &id[..10])
}

/// Reads at most `cap` bytes of `r`; fails when there is more.
pub fn read_capped(r: impl Read, cap: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    r.take(cap + 1).read_to_end(&mut data)?;
    if data.len() as u64 > cap {
        bail!("more than {} MiB", cap >> 20);
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use zip::write::SimpleFileOptions;

    pub(crate) fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("debrief-bundle-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn zip_file(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in files {
            z.start_file(*name, SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn rejects_paths_that_leave_the_folder() {
        for bad in [
            "../x",
            "a/../../x",
            "/etc/passwd",
            "C:/x",
            "c:\\x",
            "a\\..\\x",
            "",
            "a/./..",
            "x:y",
            "a/ . /b",
            "a\0b",
        ] {
            assert!(safe_entry_path(bad).is_err(), "{bad:?} accepted");
        }
        assert_eq!(safe_entry_path("./logs/a.log").unwrap(), PathBuf::from("logs/a.log"));
        assert!(safe_entry_path(".").is_err());
        assert!(safe_entry_path(&"a/".repeat(600)).is_err());

        let d = temp("slip");
        let z = d.join("evil.debriefz");
        zip_file(&z, &[("ok.txt", b"x"), ("../../escape.txt", b"y")]);
        let e = extract(&z, &d.join("out")).unwrap_err();
        assert!(e.to_string().contains("unsafe path"), "{e}");
        assert!(!d.join("escape.txt").exists() && !d.join("out").exists());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn rejects_oversized_and_non_zip_bundles() {
        let d = temp("big");
        let z = d.join("big.debriefz");
        zip_file(&z, &[("a.log", &[b'a'; 2000]), ("b.log", &[b'b'; 2000])]);
        let limits = |entry, total, entries| Limits { entry, total, entries };
        assert!(extract_capped(&z, &d.join("o"), limits(1000, 10_000, 10))
            .unwrap_err()
            .to_string()
            .contains("a.log expands to more"));
        assert!(extract_capped(&z, &d.join("o"), limits(5000, 3000, 10))
            .unwrap_err()
            .to_string()
            .contains("the bundle expands"));
        assert!(extract_capped(&z, &d.join("o"), limits(5000, 10_000, 1))
            .unwrap_err()
            .to_string()
            .contains("more than 1 files"));
        assert!(cap("x", 5, 4).is_err() && cap("x", 4, 4).is_ok());
        let junk = d.join("junk.debriefz");
        std::fs::write(&junk, b"not a zip").unwrap();
        assert!(extract(&junk, &d).unwrap_err().to_string().contains("not a Debrief bundle"));
        assert!(extract(&d.join("missing.debriefz"), &d).is_err());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn checks_the_manifest_and_tolerates_other_bundles() {
        let d = temp("manifest");
        let log = b"hello";
        let manifest = |sha: &str, schema: u32| {
            serde_json::json!({
                "format": BUNDLE_FORMAT, "schema": schema, "unknownKey": 1,
                "entries": [{"path": "logs/a.log", "kind": "log", "bytes": 5, "sha256": sha}]
            })
            .to_string()
        };
        let good = d.join("good.debriefz");
        zip_file(
            &good,
            &[
                ("logs/a.log", log),
                (MANIFEST_NAME, manifest(&sha256_hex(log), 1).as_bytes()),
                ("dir/", b""),
            ],
        );
        let x = extract(&good, &d.join("out")).unwrap();
        assert_eq!(std::fs::read(x.dir.join("logs/a.log")).unwrap(), log);
        assert_eq!(x.manifest.unwrap().entries.len(), 1);
        assert!(x.project.is_none());
        // Extracting again replaces the folder.
        assert_eq!(extract(&good, &d.join("out")).unwrap().dir, x.dir);

        let bad = d.join("bad.debriefz");
        zip_file(
            &bad,
            &[("logs/a.log", b"HELLO"), (MANIFEST_NAME, manifest(&sha256_hex(log), 1).as_bytes())],
        );
        assert!(extract(&bad, &d.join("out")).unwrap_err().to_string().contains("damaged"));
        let missing = d.join("missing.debriefz");
        zip_file(&missing, &[(MANIFEST_NAME, manifest(&sha256_hex(log), 1).as_bytes())]);
        assert!(extract(&missing, &d.join("out")).unwrap_err().to_string().contains("missing"));
        let newer = d.join("newer.debriefz");
        zip_file(&newer, &[(MANIFEST_NAME, manifest("x", 99).as_bytes())]);
        assert!(extract(&newer, &d.join("out")).unwrap_err().to_string().contains("newer Debrief"));
        let broken = d.join("broken.debriefz");
        zip_file(&broken, &[(MANIFEST_NAME, br#"{"format":"debrief-share","schema":"x"}"#)]);
        assert!(extract(&broken, &d.join("out"))
            .unwrap_err()
            .to_string()
            .contains("manifest is damaged"));

        // A collector bundle: its own manifest, a project.
        let collected = d.join("collected.debriefz");
        zip_file(
            &collected,
            &[
                (MANIFEST_NAME, br#"{"schema":1,"items":[]}"#),
                (PROJECT_NAME, b"{}"),
                ("x.log", b"x"),
            ],
        );
        let x = extract(&collected, &d.join("out")).unwrap();
        assert!(x.manifest.is_none() && x.project.is_some());
        let bare = d.join("bare.debriefz");
        zip_file(&bare, &[("x.log", b"x")]);
        let x = extract(&bare, &d.join("out")).unwrap();
        assert!(x.manifest.is_none() && x.project.is_none());
        let not_json = d.join("notjson.debriefz");
        zip_file(&not_json, &[(MANIFEST_NAME, b"{")]);
        assert!(extract(&not_json, &d.join("out")).unwrap().manifest.is_none());
        std::fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn helpers() {
        assert_eq!(
            sha256_hex(b"abc"),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        let mut w = HashingWriter::new(Vec::new());
        w.write_all(b"abc").unwrap();
        w.flush().unwrap();
        let (inner, n, sha) = w.finish();
        assert_eq!((inner, n, sha), (b"abc".to_vec(), 3, sha256_hex(b"abc")));
        assert!(folder_name(Path::new("/x/My Case.debriefz")).starts_with("My_Case-"));
        assert!(folder_name(Path::new("/")).starts_with("bundle-"));
        assert_eq!(read_capped(&b"abc"[..], 3).unwrap(), b"abc");
        assert!(read_capped(&b"abcd"[..], 3).is_err());
    }
}
