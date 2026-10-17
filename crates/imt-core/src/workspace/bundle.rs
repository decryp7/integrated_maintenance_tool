//! Writing shareable `.imtz` bundles of the workspace (see [`crate::bundle`]).

use super::Workspace;
use crate::archive::{is_gzip, is_zip, ENTRY_SEPARATOR};
use crate::bundle::{
    BundleContent, BundleEntry, BundleEstimate, BundleManifest, BundleOptions, EntryKind, EstimatePart, HashingWriter, SkippedSource,
    BUNDLE_FORMAT, BUNDLE_SCHEMA, MANIFEST_NAME, MAX_REDACTED_FILE, PROJECT_NAME,
};
use crate::export::{ClefExporter, ExportRow, Exporter};
use crate::project::ProjectFile;
use crate::redact::{Counts, Redactor};
use crate::report::Report;
use anyhow::{Context, Result};
use std::borrow::Cow;
use std::collections::BTreeMap;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use zip::write::SimpleFileOptions;

const LOGS: &str = "logs";
const EVENTS: &str = "events/events.clef";

/// Which files go where, and which are left out.
struct RawPlan {
    files: Vec<(PathBuf, String)>,
    skipped: Vec<SkippedSource>,
}

/// The zip being written, with its manifest entries.
struct Writer {
    zip: zip::ZipWriter<File>,
    entries: Vec<BundleEntry>,
}

impl Writer {
    fn start(&mut self, name: &str) -> Result<()> {
        let opts = SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .large_file(true);
        self.zip
            .start_file(name, opts)
            .with_context(|| format!("cannot add {name} to the bundle"))
    }

    /// Streams one entry through `write`, recording its size and hash.
    fn entry(&mut self, name: &str, kind: EntryKind, write: impl FnOnce(&mut dyn Write) -> Result<()>) -> Result<()> {
        self.start(name)?;
        let mut out = HashingWriter::new(&mut self.zip);
        write(&mut out)?;
        let (_, bytes, sha256) = out.finish();
        self.entries.push(BundleEntry {
            path: name.into(),
            kind,
            bytes,
            sha256,
        });
        Ok(())
    }

    fn bytes(&mut self, name: &str, kind: EntryKind, data: &[u8]) -> Result<()> {
        self.entry(name, kind, |w| Ok(w.write_all(data)?))
    }
}

/// Removes a half-written bundle unless disarmed.
struct PartFile(Option<PathBuf>);

impl Drop for PartFile {
    fn drop(&mut self) {
        if let Some(p) = self.0.take() {
            let _ = std::fs::remove_file(p);
        }
    }
}

impl Workspace {
    /// The opened files to copy, each with its path in the bundle. Files of
    /// one folder stay together (rolled logs keep their grouping).
    fn raw_plan(&self, redact: bool) -> RawPlan {
        self.raw_plan_capped(redact.then_some(MAX_REDACTED_FILE))
    }

    /// [`Workspace::raw_plan`]; `redact_limit` is the largest file redacted (none: no redaction).
    fn raw_plan_capped(&self, redact_limit: Option<u64>) -> RawPlan {
        let mut files: Vec<(PathBuf, String)> = Vec::new();
        let mut skipped = Vec::new();
        let mut folders: BTreeMap<PathBuf, usize> = BTreeMap::new();
        let mut seen = std::collections::HashSet::new();
        for s in &self.sources {
            let file = PathBuf::from(s.path.split_once(ENTRY_SEPARATOR).map_or(s.path.as_str(), |(f, _)| f));
            if !seen.insert(file.clone()) {
                continue;
            }
            let meta = std::fs::metadata(&file);
            if !meta.as_ref().is_ok_and(|m| m.is_file()) {
                skipped.push(SkippedSource {
                    path: s.path.clone(),
                    reason: "the file no longer exists".into(),
                });
                continue;
            }
            if let Some(limit) = redact_limit.filter(|&l| meta.is_ok_and(|m| m.len() > l)) {
                skipped.push(SkippedSource {
                    path: s.path.clone(),
                    reason: format!("larger than {} MiB, too large to redact", limit >> 20),
                });
                continue;
            }
            let parent = file.parent().map(Path::to_path_buf).unwrap_or_default();
            let n = folders.len();
            let folder = *folders.entry(parent).or_insert(n);
            let name = file
                .file_name()
                .map(|f| f.to_string_lossy().into_owned())
                .unwrap_or_else(|| "file".into());
            files.push((file, format!("{LOGS}/{folder}/{name}")));
        }
        RawPlan { files, skipped }
    }

    /// What a bundle with `opts` would contain.
    pub fn estimate_bundle(&self, opts: &BundleOptions, profile_toml: &str) -> Result<BundleEstimate> {
        let mut parts = vec![EstimatePart {
            name: "profile".into(),
            bytes: profile_toml.len() as u64,
        }];
        // Project, profile and dashboards, plus the logs.
        let (logs, events, skipped) = match opts.content {
            BundleContent::Raw => {
                let plan = self.raw_plan(opts.redact);
                let bytes = plan
                    .files
                    .iter()
                    .filter_map(|(p, _)| std::fs::metadata(p).ok())
                    .map(|m| m.len())
                    .sum();
                let part = EstimatePart {
                    name: "logs".into(),
                    bytes,
                };
                ((part, plan.files.len()), self.sources.iter().map(|s| s.records).sum(), plan.skipped)
            }
            BundleContent::Events => {
                let ids = self.matching(&opts.query)?;
                // Average CLEF size of up to 500 events spread over the result.
                let step = (ids.len() / 500).max(1);
                let sample: Vec<u64> = ids
                    .iter()
                    .step_by(step)
                    .map(|&id| serde_json::to_vec(&crate::export::clef_value(&self.records[id as usize])).map_or(0, |v| v.len() as u64 + 1))
                    .collect();
                let avg = sample.iter().sum::<u64>() / (sample.len().max(1) as u64);
                let part = EstimatePart {
                    name: "events".into(),
                    bytes: avg * ids.len() as u64,
                };
                ((part, 1), ids.len(), Vec::new())
            }
        };
        let files = 3 + logs.1 + opts.reports.len();
        parts.push(logs.0);
        let reports: u64 = opts.reports.iter().map(|r| r.to_string().len() as u64).sum();
        parts.push(EstimatePart {
            name: "reports".into(),
            bytes: reports,
        });
        Ok(BundleEstimate {
            files,
            bytes: parts.iter().map(|p| p.bytes).sum(),
            events,
            parts,
            skipped,
        })
    }

    /// Writes a shareable bundle to `path` (via `<path>.part`, renamed when complete).
    pub fn write_bundle(&self, opts: &BundleOptions, profile_toml: &str, path: &Path) -> Result<BundleManifest> {
        let reports = opts
            .reports
            .iter()
            .map(|r| Report::parse(r.clone()))
            .collect::<Result<Vec<_>>>()
            .context("a report cannot be bundled")?;
        let redactor = if opts.redact { Some(self.export_redactor()?) } else { None };
        let part = path.with_extension("imtz.part");
        let file = File::create(&part).with_context(|| format!("cannot create {}", part.display()))?;
        let mut guard = PartFile(Some(part.clone()));
        let mut w = Writer {
            zip: zip::ZipWriter::new(file),
            entries: Vec::new(),
        };
        let mut counts = Counts::new();
        let mut skipped = Vec::new();
        let mut events = None;
        let source = match opts.content {
            BundleContent::Raw => {
                let plan = self.raw_plan(opts.redact);
                skipped = plan.skipped;
                for (file, name) in plan.files {
                    if let Some(reason) = copy_file(&mut w, &file, &name, redactor.as_ref(), &mut counts)? {
                        skipped.push(SkippedSource {
                            path: file.to_string_lossy().into_owned(),
                            reason,
                        });
                    }
                }
                LOGS
            }
            BundleContent::Events => {
                let ids = self.matching(&opts.query)?;
                let cq = self.compile(&opts.query)?;
                w.entry(EVENTS, EntryKind::Events, |out| {
                    let mut rows = ids.iter().map(|&id| {
                        let record = &self.records[id as usize];
                        let source_name = self.source_name(record.source);
                        let group = cq.group_key(record, source_name);
                        ExportRow {
                            record: match &redactor {
                                Some(r) => r.record(record, &mut counts),
                                None => Cow::Borrowed(record),
                            },
                            source_name,
                            group,
                        }
                    });
                    events = Some(ClefExporter.export(&mut rows, out)?);
                    Ok(())
                })?;
                "events"
            }
        };
        w.bytes("profile.toml", EntryKind::Profile, profile_toml.as_bytes())?;
        let dashboards = serde_json::to_vec_pretty(&self.profile.profile.dashboard)?;
        w.bytes("dashboards.json", EntryKind::Dashboards, &dashboards)?;
        for r in &reports {
            let id: String =
                r.id.chars()
                    .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
                    .take(64)
                    .collect();
            w.bytes(&format!("reports/{id}.json"), EntryKind::Report, &serde_json::to_vec_pretty(r)?)?;
        }
        let project = ProjectFile {
            version: crate::project::PROJECT_VERSION,
            name: opts.name.clone(),
            sources: vec![source.into()],
            profile: profile_toml.into(),
            ui: opts.ui.clone(),
            reports,
            ..Default::default()
        };
        w.bytes(PROJECT_NAME, EntryKind::Project, &serde_json::to_vec_pretty(&project)?)?;
        let manifest = BundleManifest {
            format: BUNDLE_FORMAT.into(),
            schema: BUNDLE_SCHEMA,
            name: opts.name.clone(),
            created_at: chrono::Utc::now().timestamp_millis(),
            imt_version: env!("CARGO_PKG_VERSION").into(),
            content: opts.content,
            redacted: redactor.is_some(),
            redaction: counts,
            events,
            entries: w.entries.clone(),
            skipped,
        };
        w.start(MANIFEST_NAME)?;
        w.zip.write_all(&serde_json::to_vec_pretty(&manifest)?)?;
        let mut file = w.zip.finish().context("cannot finish the bundle")?;
        file.flush()?;
        drop(file);
        std::fs::rename(&part, path).with_context(|| format!("cannot write {}", path.display()))?;
        guard.0 = None;
        Ok(manifest)
    }
}

/// Copies one file into the bundle, redacted when there is a redactor.
/// Returns why it was left out, if it was.
fn copy_file(w: &mut Writer, file: &Path, name: &str, redactor: Option<&Redactor>, counts: &mut Counts) -> Result<Option<String>> {
    let Some(redactor) = redactor else {
        let mut src = File::open(file).with_context(|| format!("cannot read {}", file.display()))?;
        w.entry(name, EntryKind::Log, |out| {
            std::io::copy(&mut src, out)?;
            Ok(())
        })?;
        return Ok(None);
    };
    let bytes = std::fs::read(file).with_context(|| format!("cannot read {}", file.display()))?;
    if is_gzip(&bytes) || is_zip(&bytes) {
        return Ok(Some("archives cannot be redacted (bundle the filtered events instead)".into()));
    }
    let Some(text) = crate::importers::decode(&bytes) else {
        return Ok(Some("binary files cannot be redacted (bundle the filtered events instead)".into()));
    };
    let out = redactor.apply(&text, counts);
    w.bytes(name, EntryKind::Log, out.as_bytes())?;
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Profile;

    #[test]
    fn large_and_binary_files_are_not_redacted() {
        let dir = std::env::temp_dir().join(format!("imt-wsbundle-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("a.clef"), "{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"x\"}\n").unwrap();
        let mut ws = Workspace::new(Profile::from_toml("[redact]\ndetectors=['email']").unwrap()).unwrap();
        ws.open(std::slice::from_ref(&dir)).unwrap();
        let plan = ws.raw_plan_capped(Some(1));
        assert!(plan.files.is_empty());
        assert!(plan.skipped[0].reason.contains("too large to redact"));
        assert_eq!(ws.raw_plan_capped(None).files.len(), 1);

        let bin = dir.join("blob.bin");
        std::fs::write(&bin, [0u8, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 1, 2, 3]).unwrap();
        let mut w = Writer {
            zip: zip::ZipWriter::new(File::create(dir.join("x.zip")).unwrap()),
            entries: Vec::new(),
        };
        let redactor = ws.export_redactor().unwrap();
        let why = copy_file(&mut w, &bin, "logs/0/blob.bin", Some(&redactor), &mut Counts::new()).unwrap();
        assert!(why.unwrap().contains("binary files cannot be redacted"));
        assert!(w.entries.is_empty());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
