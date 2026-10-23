//! Self-contained projects (ADR 0014 §6) and the one archive module for every `.debriefz`.
//!
//! A project is a folder `<name>.debrief/`: `project.toml` (format version, name, profile overrides, clock
//! offsets, layout, opened roots), the store's database and event segments, `snapshots/`, and caches
//! (`index/`, `tmp/`) that are never shared. Exported, it is a zip `<name>.debriefz` without the caches.
//!
//! `.debriefz` is also the extension of share bundles ([`crate::bundle`]) and collector bundles
//! (engine review finding 14). [`open_package`] tells them apart by their manifest: a root
//! `project.toml` whose `format` is [`PROJECT_FORMAT`] is a project, a `manifest.json` whose `format` is
//! `debrief-share` a share bundle, anything else a collector bundle. A project's database and segments are
//! streamed to disk under their own size limits, not the per-entry cap of log archives.

use crate::bundle::{safe_entry_path, ExtractedBundle};
use anyhow::{anyhow, bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};
use std::path::{Path, PathBuf};

/// `project.toml` `format` of a project.
pub const PROJECT_FORMAT: &str = "debrief-project";
/// Version of the project layout this build writes and reads.
pub const PROJECT_VERSION: u32 = 1;
pub const PROJECT_FILE: &str = "project.toml";
/// Folder extension of a project.
pub const PROJECT_EXTENSION: &str = "debrief";
/// Archive extension of an exported project (and of share and collector bundles).
pub const ARCHIVE_EXTENSION: &str = "debriefz";
pub const SNAPSHOTS: &str = "snapshots";
/// Caches: rebuilt when missing, never exported.
pub const CACHES: [&str; 2] = ["index", "tmp"];

/// Largest single file of an exported project (the database, a segment).
pub const MAX_PROJECT_ENTRY: u64 = 256 << 30;
/// Largest exported project, expanded.
pub const MAX_PROJECT_BYTES: u64 = 1 << 40;
/// Most files in an exported project.
pub const MAX_PROJECT_ENTRIES: usize = 100_000;
/// Largest `project.toml`.
const MAX_MANIFEST_BYTES: u64 = 16 << 20;

/// A clock offset of one source (shifts its timestamps).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceOffset {
    /// The source's path when it was opened (stable with the project).
    pub path: String,
    pub offset_ms: i64,
}

/// `project.toml`. Every key is optional (missing keys take defaults); unknown keys are kept out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ProjectManifest {
    pub format: String,
    pub version: u32,
    pub name: String,
    /// TOML merged over the profile the project was opened with (`Profile::with_override`).
    pub profile_overrides: String,
    pub clock_offsets: Vec<SourceOffset>,
    /// The UI layout (panels, columns), as the UI saved it.
    pub layout: toml::Table,
    /// Opened files and folders, for refresh.
    pub roots: Vec<String>,
    /// The parsers the project's sources were read with (ADR 0026); their TOML is embedded in the store
    /// (`parsers` table), so a shared project says what read it.
    pub parsers: Vec<crate::ports::ParserRef>,
}

impl Default for ProjectManifest {
    fn default() -> Self {
        ProjectManifest {
            format: PROJECT_FORMAT.into(),
            version: PROJECT_VERSION,
            name: String::new(),
            profile_overrides: String::new(),
            clock_offsets: Vec::new(),
            layout: toml::Table::new(),
            roots: Vec::new(),
            parsers: Vec::new(),
        }
    }
}

impl ProjectManifest {
    /// Parses `project.toml`; refuses another format or a newer version with a message.
    pub fn parse(text: &str) -> Result<ProjectManifest> {
        let m: ProjectManifest = toml::from_str(text).map_err(|e| anyhow!("project.toml: {}", e.message()))?;
        if m.format != PROJECT_FORMAT {
            bail!("project.toml is not a Debrief project (format \"{}\")", m.format);
        }
        if m.version > PROJECT_VERSION {
            bail!(
                "this project was saved by a newer Debrief (project format {}; this Debrief reads up to {PROJECT_VERSION}); update Debrief to open it",
                m.version
            );
        }
        Ok(m)
    }

    pub fn to_toml(&self) -> Result<String> {
        toml::to_string(self).context("cannot write project.toml")
    }
}

/// A project folder.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDir {
    pub dir: PathBuf,
}

impl ProjectDir {
    /// A new project folder `dir` (created when missing) named `name`.
    pub fn create(dir: &Path, name: &str) -> Result<ProjectDir> {
        std::fs::create_dir_all(dir.join(SNAPSHOTS)).with_context(|| format!("cannot create {}", dir.display()))?;
        let p = ProjectDir { dir: dir.to_path_buf() };
        if !p.manifest_path().is_file() {
            p.save(&ProjectManifest {
                name: name.to_string(),
                ..ProjectManifest::default()
            })?;
        }
        Ok(p)
    }

    /// An existing project folder.
    pub fn open(dir: &Path) -> Result<(ProjectDir, ProjectManifest)> {
        let p = ProjectDir { dir: dir.to_path_buf() };
        let path = p.manifest_path();
        let text = read_text(&path).with_context(|| format!("{} is not a Debrief project", dir.display()))?;
        let m = ProjectManifest::parse(&text)?;
        Ok((p, m))
    }

    pub fn manifest_path(&self) -> PathBuf {
        self.dir.join(PROJECT_FILE)
    }

    /// Writes `project.toml` (atomically: a crash leaves the old one).
    pub fn save(&self, m: &ProjectManifest) -> Result<()> {
        let tmp = self.dir.join(format!("{PROJECT_FILE}.tmp"));
        std::fs::write(&tmp, m.to_toml()?).with_context(|| format!("cannot write {}", tmp.display()))?;
        std::fs::rename(&tmp, self.manifest_path()).with_context(|| format!("cannot replace {}", self.manifest_path().display()))
    }

    /// The files an export carries: everything but caches and write-ahead logs, relative, sorted.
    pub fn shared_files(&self) -> Result<Vec<PathBuf>> {
        let mut out = Vec::new();
        walk(&self.dir, Path::new(""), &mut out)?;
        out.retain(|rel| {
            let first = rel.components().next().map(|c| c.as_os_str().to_string_lossy().into_owned());
            let name = rel.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            !first.is_some_and(|f| CACHES.contains(&f.as_str())) && !name.ends_with(".wal") && !name.ends_with(".tmp")
        });
        out.sort();
        Ok(out)
    }

    /// Writes the project as an `.debriefz` zip to `out` (via a `.part` file, moved into place when complete).
    /// The caller checkpoints the database first, so no write-ahead log is needed.
    pub fn export(&self, out: &Path) -> Result<u64> {
        self.export_with(out, &[])
    }

    /// [`export`](Self::export), taking the files of `substitutes` (relative path, file to read) from
    /// elsewhere: a store exports a copy of its open database (which other processes cannot read on
    /// Windows) in its place.
    pub fn export_with(&self, out: &Path, substitutes: &[(PathBuf, PathBuf)]) -> Result<u64> {
        // A project that does not open is not exported.
        ProjectDir::open(&self.dir)?;
        let part = out.with_extension(format!("{ARCHIVE_EXTENSION}.part"));
        let file = std::fs::File::create(&part).with_context(|| format!("cannot write {}", part.display()))?;
        let result = (|| -> Result<u64> {
            let mut zip = zip::ZipWriter::new(std::io::BufWriter::new(file));
            let mut total = 0u64;
            let mut files: Vec<(PathBuf, PathBuf)> = self
                .shared_files()?
                .into_iter()
                .filter(|rel| !substitutes.iter().any(|(s, _)| s == rel))
                .map(|rel| (self.dir.join(&rel), rel))
                .collect();
            files.extend(substitutes.iter().map(|(rel, src)| (src.clone(), rel.clone())));
            files.sort_by(|a, b| a.1.cmp(&b.1));
            for (src, rel) in files {
                let name = rel
                    .components()
                    .map(|c| c.as_os_str().to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join("/");
                // Parquet segments are compressed already.
                let method = if name.ends_with(".parquet") {
                    zip::CompressionMethod::Stored
                } else {
                    zip::CompressionMethod::Deflated
                };
                let opts = zip::write::SimpleFileOptions::default().compression_method(method).large_file(true);
                zip.start_file(name.as_str(), opts)?;
                let mut f = std::fs::File::open(&src).with_context(|| format!("cannot read {name}"))?;
                total += std::io::copy(&mut f, &mut zip).with_context(|| format!("cannot write {name}"))?;
            }
            let mut w = zip.finish()?;
            w.flush()?;
            Ok(total)
        })();
        match result {
            Ok(n) => {
                std::fs::rename(&part, out).with_context(|| format!("cannot write {}", out.display()))?;
                Ok(n)
            }
            Err(e) => {
                let _ = std::fs::remove_file(&part);
                Err(e)
            }
        }
    }
}

fn read_text(path: &Path) -> Result<String> {
    let f = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let bytes = crate::bundle::read_capped(f, MAX_MANIFEST_BYTES)?;
    String::from_utf8(bytes).map_err(|_| anyhow!("{} is not UTF-8 text", path.display()))
}

fn walk(root: &Path, rel: &Path, out: &mut Vec<PathBuf>) -> Result<()> {
    let dir = root.join(rel);
    for entry in std::fs::read_dir(&dir).with_context(|| format!("cannot list {}", dir.display()))? {
        let entry = entry?;
        let kind = entry.file_type()?;
        let rel = rel.join(entry.file_name());
        if kind.is_dir() {
            walk(root, &rel, out)?;
        } else if kind.is_file() {
            out.push(rel);
        }
        // Links are not followed (an export never leaves the project folder).
    }
    Ok(())
}

/// What an `.debriefz` (or a project folder) holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Package {
    /// A project, extracted into (or already) a folder.
    Project { dir: ProjectDir, manifest: ProjectManifest },
    /// A share bundle (`manifest.json` format `debrief-share`).
    Share(ExtractedBundle),
    /// A collector bundle (any other `.debriefz`).
    Collector(ExtractedBundle),
}

/// The `format` of a root `project.toml` in a zip, if it has one.
fn project_format<R: Read + Seek>(zip: &mut zip::ZipArchive<R>) -> Option<String> {
    let entry = zip.by_name(PROJECT_FILE).ok()?;
    let text = String::from_utf8(crate::bundle::read_capped(entry, MAX_MANIFEST_BYTES).ok()?).ok()?;
    let t: toml::Table = toml::from_str(&text).ok()?;
    t.get("format").and_then(|f| f.as_str()).map(str::to_string)
}

/// Opens a project folder or an `.debriefz`: a project archive is extracted into `dest_root/<name>.debrief`
/// (replacing an earlier extraction of the same archive); share and collector bundles are extracted as
/// [`crate::bundle::extract`] does.
pub fn open_package(path: &Path, dest_root: &Path) -> Result<Package> {
    if path.is_dir() {
        let (dir, manifest) = ProjectDir::open(path)?;
        return Ok(Package::Project { dir, manifest });
    }
    let file = std::fs::File::open(path).with_context(|| format!("cannot open {}", path.display()))?;
    let mut zip =
        zip::ZipArchive::new(file).with_context(|| format!("{} is not a Debrief project or bundle (not a zip file)", path.display()))?;
    match project_format(&mut zip).as_deref() {
        Some(PROJECT_FORMAT) => extract_project(path, &mut zip, dest_root),
        Some(other) => bail!("{} holds a project.toml of an unknown format \"{other}\"", path.display()),
        None => {
            drop(zip);
            let b = crate::bundle::extract(path, dest_root)?;
            Ok(if b.manifest.is_some() {
                Package::Share(b)
            } else {
                Package::Collector(b)
            })
        }
    }
}

fn extract_project<R: Read + Seek>(path: &Path, zip: &mut zip::ZipArchive<R>, dest_root: &Path) -> Result<Package> {
    if zip.len() > MAX_PROJECT_ENTRIES {
        bail!("the project has more than {MAX_PROJECT_ENTRIES} files");
    }
    // Check every name and the declared sizes before writing anything.
    let mut names = Vec::with_capacity(zip.len());
    let mut declared = 0u64;
    for i in 0..zip.len() {
        let entry = zip.by_index(i).context("invalid zip entry")?;
        if entry.is_dir() {
            continue;
        }
        let name = entry.name().context("invalid zip entry name")?.to_string();
        let rel = safe_entry_path(&name)?;
        if rel.components().next().is_some_and(|c| CACHES.iter().any(|x| c.as_os_str() == *x)) {
            continue;
        }
        cap(&name, entry.size(), MAX_PROJECT_ENTRY)?;
        declared = declared.saturating_add(entry.size());
        names.push((i, name, rel));
    }
    cap("the project", declared, MAX_PROJECT_BYTES)?;
    let stem = path.file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
    let safe: String = stem
        .chars()
        .map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' })
        .take(60)
        .collect();
    let dir = dest_root.join(format!("{}.{PROJECT_EXTENSION}", if safe.is_empty() { "project" } else { &safe }));
    if dir.exists() {
        std::fs::remove_dir_all(&dir).with_context(|| format!("cannot replace {}", dir.display()))?;
    }
    std::fs::create_dir_all(&dir).with_context(|| format!("cannot create {}", dir.display()))?;
    let mut total = 0u64;
    for (i, name, rel) in names {
        let entry = zip.by_index(i).context("invalid zip entry")?;
        let target = dir.join(&rel);
        std::fs::create_dir_all(target.parent().unwrap_or(&dir))?;
        let mut out = std::fs::File::create(&target).with_context(|| format!("cannot write {}", target.display()))?;
        // Streamed to disk; the declared size may lie, so what is read is capped too.
        let n = std::io::copy(&mut entry.take(MAX_PROJECT_ENTRY + 1), &mut out).with_context(|| format!("cannot extract {name}"))?;
        cap(&name, n, MAX_PROJECT_ENTRY)?;
        total += n;
        cap("the project", total, MAX_PROJECT_BYTES)?;
    }
    let (dir, manifest) = ProjectDir::open(&dir)?;
    std::fs::create_dir_all(dir.dir.join(SNAPSHOTS))?;
    Ok(Package::Project { dir, manifest })
}

fn cap(what: &str, n: u64, max: u64) -> Result<()> {
    if n > max {
        bail!("{what} expands to more than {} GiB", max >> 30);
    }
    Ok(())
}

/// Clock offsets of a manifest by source path.
pub fn offsets_by_path(m: &ProjectManifest) -> BTreeMap<&str, i64> {
    m.clock_offsets.iter().map(|o| (o.path.as_str(), o.offset_ms)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("debrief-package-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn manifests_round_trip_and_refuse_what_they_cannot_read() {
        let m = ProjectManifest {
            name: "incident".into(),
            profile_overrides: "[instance]\nkey = [\"Host\"]\n".into(),
            clock_offsets: vec![SourceOffset {
                path: "/logs/a.log".into(),
                offset_ms: -1500,
            }],
            roots: vec!["/logs".into()],
            ..ProjectManifest::default()
        };
        let back = ProjectManifest::parse(&m.to_toml().unwrap()).unwrap();
        assert_eq!(back, m);
        assert_eq!(offsets_by_path(&back)["/logs/a.log"], -1500);
        // Missing keys take defaults.
        let minimal = ProjectManifest::parse("format = \"debrief-project\"").unwrap();
        assert_eq!(minimal.version, PROJECT_VERSION);
        let newer = ProjectManifest::parse("format = \"debrief-project\"\nversion = 99")
            .unwrap_err()
            .to_string();
        assert!(newer.contains("newer Debrief"), "{newer}");
        assert!(ProjectManifest::parse("format = \"other\"").is_err());
        assert!(ProjectManifest::parse("version = [").is_err());
    }

    #[test]
    fn projects_export_without_caches_and_open_again() {
        let root = temp("export");
        let dir = root.join("incident.debrief");
        let p = ProjectDir::create(&dir, "incident").unwrap();
        std::fs::write(dir.join("data.duckdb"), vec![7u8; 100_000]).unwrap();
        std::fs::write(dir.join("data.duckdb.wal"), b"wal").unwrap();
        std::fs::create_dir_all(dir.join("segments")).unwrap();
        std::fs::write(dir.join("segments/0001.parquet"), b"PAR1").unwrap();
        std::fs::create_dir_all(dir.join("index")).unwrap();
        std::fs::write(dir.join("index/meta.json"), b"{}").unwrap();
        std::fs::write(dir.join("snapshots/s1.json"), b"{}").unwrap();
        let files = p.shared_files().unwrap();
        let names: Vec<String> = files.iter().map(|f| f.to_string_lossy().replace('\\', "/")).collect();
        assert_eq!(
            names,
            vec!["data.duckdb", "project.toml", "segments/0001.parquet", "snapshots/s1.json"]
        );
        let out = root.join("incident.debriefz");
        let bytes = p.export(&out).unwrap();
        assert!(bytes > 100_000);
        assert!(!root.join("incident.debriefz.part").exists());
        let opened = open_package(&out, &root.join("open")).unwrap();
        let (opened, manifest) = project(opened);
        assert_eq!(manifest.name, "incident");
        assert_eq!(std::fs::read(opened.dir.join("data.duckdb")).unwrap().len(), 100_000);
        assert!(!opened.dir.join("index").exists() && opened.dir.join("snapshots").is_dir());
        // A folder opens as it is.
        assert!(matches!(open_package(&dir, &root).unwrap(), Package::Project { .. }));
        let _ = std::fs::remove_dir_all(&root);
    }

    #[rustfmt::skip]
    fn project(p: Package) -> (ProjectDir, ProjectManifest) {
        let Package::Project { dir, manifest } = p else { panic!("not a project") };
        (dir, manifest)
    }

    fn zip_with(path: &Path, files: &[(&str, &[u8])]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (name, data) in files {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    #[test]
    fn debriefz_files_are_told_apart_by_their_manifest() {
        let root = temp("kinds");
        let collector = root.join("collected.debriefz");
        zip_with(&collector, &[("manifest.json", br#"{"host":"web01"}"#), ("logs/a.log", b"x\n")]);
        assert!(matches!(open_package(&collector, &root.join("o")).unwrap(), Package::Collector(_)));
        let share = root.join("shared.debriefz");
        let log = b"hello";
        let manifest = serde_json::json!({
            "format": crate::bundle::BUNDLE_FORMAT, "schema": 1,
            "entries": [{"path": "logs/a.log", "kind": "log", "bytes": 5, "sha256": crate::bundle::sha256_hex(log)}]
        })
        .to_string();
        zip_with(&share, &[("logs/a.log", log), ("manifest.json", manifest.as_bytes())]);
        let shared = open_package(&share, &root.join("o")).unwrap();
        assert!(
            matches!(&shared, Package::Share(b) if b.dir.join("logs/a.log").is_file()),
            "{shared:?}"
        );
        let odd = root.join("odd.debriefz");
        zip_with(&odd, &[("project.toml", b"format = \"something-else\"")]);
        let err = open_package(&odd, &root.join("o")).unwrap_err().to_string();
        assert!(err.contains("unknown format"), "{err}");
        let unsafe_zip = root.join("unsafe.debriefz");
        zip_with(&unsafe_zip, &[("project.toml", b"format = \"debrief-project\""), ("../evil", b"x")]);
        assert!(open_package(&unsafe_zip, &root.join("o")).is_err());
        let not_zip = root.join("plain.debriefz");
        std::fs::write(&not_zip, b"hello").unwrap();
        assert!(open_package(&not_zip, &root.join("o")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn creating_a_project_keeps_an_existing_manifest_and_substitutes_replace_files() {
        let root = temp("create");
        let dir = root.join("p.debrief");
        let first = ProjectDir::create(&dir, "first").unwrap();
        // Creating again never renames or resets what is there.
        let again = ProjectDir::create(&dir, "second").unwrap();
        assert_eq!(first, again);
        assert_eq!(ProjectDir::open(&dir).unwrap().1.name, "first");
        // A file read from elsewhere (a copy of an open database) goes into the archive in the place of the original.
        std::fs::write(dir.join("data.duckdb"), b"original").unwrap();
        let copy = root.join("copy.duckdb");
        std::fs::write(&copy, b"the copy").unwrap();
        let out = root.join("p.debriefz");
        let bytes = first.export_with(&out, &[(PathBuf::from("data.duckdb"), copy)]).unwrap();
        assert!(bytes >= "the copy".len() as u64);
        let opened = open_package(&out, &root.join("open")).unwrap();
        let (opened, _) = project(opened);
        assert_eq!(std::fs::read(opened.dir.join("data.duckdb")).unwrap(), b"the copy");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[cfg(unix)]
    #[test]
    fn a_failed_export_leaves_no_partial_archive() {
        let root = temp("failed-export");
        let dir = root.join("p.debrief");
        let p = ProjectDir::create(&dir, "p").unwrap();
        // A file that cannot be read fails the export midway.
        std::fs::write(dir.join("broken.dat"), b"x").unwrap();
        std::fs::set_permissions(dir.join("broken.dat"), std::os::unix::fs::PermissionsExt::from_mode(0o000)).unwrap();
        if std::fs::File::open(dir.join("broken.dat")).is_ok() {
            // Running as root (a CI container): permissions are not enforced, so the file cannot be made unreadable.
            return;
        }
        let out = root.join("p.debriefz");
        let err = p.export(&out).unwrap_err().to_string();
        assert!(err.contains("cannot read broken.dat"), "{err}");
        assert!(
            !out.exists() && !root.join("p.debriefz.part").exists(),
            "nothing half-written is left"
        );
        std::fs::set_permissions(dir.join("broken.dat"), std::os::unix::fs::PermissionsExt::from_mode(0o600)).unwrap();
        // A project that does not open is not exported at all.
        let missing = ProjectDir {
            dir: root.join("missing.debrief"),
        };
        assert!(missing.export(&root.join("m.debriefz")).is_err());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn opening_an_archive_again_replaces_the_earlier_extraction_and_skips_caches() {
        let root = temp("reopen");
        let archive = root.join("my project (copy).debriefz");
        zip_with(
            &archive,
            &[
                ("project.toml", b"format = \"debrief-project\"\nname = \"mine\""),
                ("data.duckdb", b"db"),
                ("index/meta.json", b"{}"),
                ("tmp/scratch", b"x"),
            ],
        );
        let dest = root.join("open");
        let (dir, manifest) = project(open_package(&archive, &dest).unwrap());
        assert_eq!(manifest.name, "mine");
        // The folder is named after the archive, with unsafe characters replaced.
        assert_eq!(
            dir.dir.file_name().unwrap().to_string_lossy(),
            format!("my_project__copy_.{PROJECT_EXTENSION}")
        );
        assert!(
            !dir.dir.join("index").exists() && !dir.dir.join("tmp").exists(),
            "caches are not extracted"
        );
        // Something added since is gone after the next open: it is the archive's content again.
        std::fs::write(dir.dir.join("added-later.txt"), b"x").unwrap();
        let (again, _) = project(open_package(&archive, &dest).unwrap());
        assert_eq!(again.dir, dir.dir);
        assert!(!again.dir.join("added-later.txt").exists());
        assert_eq!(std::fs::read(again.dir.join("data.duckdb")).unwrap(), b"db");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn size_limits_say_how_big_is_too_big() {
        assert!(cap("a", MAX_PROJECT_ENTRY, MAX_PROJECT_ENTRY).is_ok());
        let e = cap("data.duckdb", MAX_PROJECT_ENTRY + 1, MAX_PROJECT_ENTRY)
            .unwrap_err()
            .to_string();
        assert_eq!(e, "data.duckdb expands to more than 256 GiB");
        assert!(cap("the project", MAX_PROJECT_BYTES + 1, MAX_PROJECT_BYTES)
            .unwrap_err()
            .to_string()
            .contains("1024 GiB"));
    }

    #[test]
    fn an_archive_with_too_many_files_is_refused_before_anything_is_written() {
        let root = temp("many");
        let archive = root.join("many.debriefz");
        let mut z = zip::ZipWriter::new(std::io::BufWriter::new(std::fs::File::create(&archive).unwrap()));
        let stored = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
        z.start_file("project.toml", stored).unwrap();
        z.write_all(b"format = \"debrief-project\"").unwrap();
        for i in 0..MAX_PROJECT_ENTRIES {
            z.start_file(format!("f/{i}"), stored).unwrap();
        }
        z.finish().unwrap();
        let dest = root.join("open");
        let err = open_package(&archive, &dest).unwrap_err().to_string();
        assert!(err.contains("more than 100000 files"), "{err}");
        assert!(!dest.exists(), "nothing was extracted");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn directory_entries_in_a_project_archive_are_skipped_and_files_extract() {
        let root = temp("diren");
        let archive = root.join("dirs.debriefz");
        let mut z = zip::ZipWriter::new(std::fs::File::create(&archive).unwrap());
        z.add_directory("notes/", zip::write::SimpleFileOptions::default()).unwrap();
        z.start_file("project.toml", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"format = \"debrief-project\"\nname = \"withdirs\"").unwrap();
        z.start_file("notes/a.txt", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"hi").unwrap();
        z.finish().unwrap();
        let (dir, manifest) = project(open_package(&archive, &root.join("o")).unwrap());
        assert_eq!(manifest.name, "withdirs");
        assert_eq!(std::fs::read(dir.dir.join("notes/a.txt")).unwrap(), b"hi");
        let _ = std::fs::remove_dir_all(&root);
    }
}
