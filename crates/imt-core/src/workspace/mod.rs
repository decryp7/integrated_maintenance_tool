//! The set of opened files and everything the UI asks about them.

mod chart;
mod context;
mod export;
mod facet;
mod page;
mod preview;
mod timeline;

pub use crate::api::*;

use crate::chart::ChartDef;
use crate::importers::{decode, ImportCtx, Importers};
use crate::model::{Level, Record};
use crate::plugin::PluginRegistry;
use crate::profile::{CompiledProfile, Profile};
use crate::query::{CompiledQuery, Query};
use crate::time::utc_date_of;
use anyhow::{anyhow, Result};
use rayon::prelude::*;
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

const MAX_DISTINCT: usize = 10_000;

pub struct Workspace {
    profile: Arc<CompiledProfile>,
    registry: PluginRegistry,
    roots: Vec<PathBuf>,
    /// Files the user removed; folder scans skip them until opened explicitly.
    removed: HashSet<PathBuf>,
    sources: Vec<SourceInfo>,
    /// Source names indexed by source id (ids are never reused).
    names: Vec<String>,
    unrecognized: Vec<UnrecognizedFile>,
    records: Vec<Record>,
    fields: Vec<FieldInfo>,
    default_group_by: Vec<String>,
    marker_kinds: Vec<String>,
    load_ms: u64,
    cache: Mutex<Option<(String, Arc<Vec<u32>>)>>,
}

impl Workspace {
    /// An empty workspace with the built-in importers and exporters.
    pub fn new(profile: Profile) -> Result<Workspace> {
        Self::with_registry(profile, PluginRegistry::new())
    }

    /// An empty workspace that reads files with the given importers (and the
    /// built-in exporters).
    pub fn with_importers(profile: Profile, importers: Importers) -> Result<Workspace> {
        let mut registry = PluginRegistry::new();
        registry.importers = importers;
        Self::with_registry(profile, registry)
    }

    /// An empty workspace extended by the plugins installed in `registry`.
    pub fn with_registry(profile: Profile, registry: PluginRegistry) -> Result<Workspace> {
        let profile = Arc::new(CompiledProfile::compile_with(profile, &registry.transforms)?);
        let marker_kinds = profile.markers.iter().map(|m| m.kind.clone()).collect();
        Ok(Workspace {
            profile,
            registry,
            roots: Vec::new(),
            removed: HashSet::new(),
            sources: Vec::new(),
            names: Vec::new(),
            unrecognized: Vec::new(),
            records: Vec::new(),
            fields: Vec::new(),
            default_group_by: vec!["@source".into()],
            marker_kinds,
            load_ms: 0,
            cache: Mutex::new(None),
        })
    }

    pub fn profile(&self) -> &Profile {
        &self.profile.profile
    }

    pub fn records(&self) -> &[Record] {
        &self.records
    }

    /// Opens files and folders (recursively), skipping files already loaded.
    pub fn open(&mut self, paths: &[PathBuf]) -> Result<()> {
        let start = Instant::now();
        let mut files = Vec::new();
        for p in paths {
            let p = std::fs::canonicalize(p).map_err(|e| anyhow!("{}: {e}", p.display()))?;
            if !self.roots.contains(&p) {
                self.roots.push(p.clone());
            }
            self.removed.remove(&p);
            self.collect_files(&p, &mut files);
        }
        // Archive entries are recorded as "<archive>!/<entry>"; the archive itself counts as known.
        let known: HashSet<&str> = self
            .sources
            .iter()
            .map(|s| s.path.as_str())
            .chain(self.unrecognized.iter().map(|u| u.path.as_str()))
            .map(|p| p.split(crate::archive::ENTRY_SEPARATOR).next().unwrap_or(p))
            .collect();
        files.retain(|f| !known.contains(f.to_string_lossy().as_ref()));
        files.sort();
        files.dedup();
        self.import_files(files);
        self.rebuild();
        self.load_ms = start.elapsed().as_millis() as u64;
        if !self.registry.hooks.is_empty() {
            let summary = self.summary();
            for h in &self.registry.hooks {
                h.on_loaded(&summary);
            }
        }
        Ok(())
    }

    pub fn reload(&mut self) -> Result<()> {
        let roots = std::mem::take(&mut self.roots);
        let removed = std::mem::take(&mut self.removed);
        self.reset();
        self.removed = removed;
        self.open(&roots.into_iter().filter(|r| r.exists()).collect::<Vec<_>>())
    }

    /// Closes everything and notifies hooks.
    pub fn clear(&mut self) {
        self.reset();
        for h in &self.registry.hooks {
            h.on_cleared();
        }
    }

    fn reset(&mut self) {
        self.roots.clear();
        self.removed.clear();
        self.sources.clear();
        self.unrecognized.clear();
        self.records.clear();
        self.rebuild();
    }

    pub fn remove_source(&mut self, id: u32) {
        if let Some(src) = self.sources.iter().find(|s| s.id == id) {
            let path = PathBuf::from(&src.path);
            self.roots.retain(|r| *r != path);
            self.removed.insert(path);
        }
        self.sources.retain(|s| s.id != id);
        self.records.retain(|r| r.source != id);
        self.rebuild();
    }

    /// Replaces the profile and re-imports everything.
    pub fn set_profile(&mut self, profile: Profile) -> Result<()> {
        let compiled = CompiledProfile::compile_with(profile, &self.registry.transforms)?;
        for h in &self.registry.hooks {
            h.on_profile_changed(&compiled.profile);
        }
        self.marker_kinds = compiled.markers.iter().map(|m| m.kind.clone()).collect();
        self.profile = Arc::new(compiled);
        self.reload()
    }

    /// The opened paths and removed files.
    pub fn state(&self) -> WorkspaceState {
        let s = |p: &PathBuf| p.to_string_lossy().into_owned();
        let mut removed: Vec<String> = self.removed.iter().map(s).collect();
        removed.sort();
        WorkspaceState {
            roots: self.roots.iter().map(s).collect(),
            removed,
        }
    }

    /// Replaces everything with `state` under `profile`. Paths that no longer
    /// exist are skipped and returned.
    pub fn restore(&mut self, profile: Profile, state: &WorkspaceState) -> Result<Vec<String>> {
        let compiled = CompiledProfile::compile_with(profile, &self.registry.transforms)?;
        self.marker_kinds = compiled.markers.iter().map(|m| m.kind.clone()).collect();
        self.profile = Arc::new(compiled);
        self.reset();
        self.removed = state.removed.iter().map(PathBuf::from).collect();
        let (present, missing): (Vec<&String>, Vec<&String>) = state
            .roots
            .iter()
            .partition(|p| Path::new(p.split(crate::archive::ENTRY_SEPARATOR).next().unwrap_or(p)).exists());
        self.open(&present.into_iter().map(PathBuf::from).collect::<Vec<_>>())?;
        Ok(missing.into_iter().cloned().collect())
    }

    /// Checks that a profile compiles against the installed plugins.
    pub fn validate_profile(&self, profile: Profile) -> Result<()> {
        CompiledProfile::compile_with(profile, &self.registry.transforms).map(|_| ())
    }

    /// Plugins installed in this workspace.
    pub fn plugins(&self) -> &[crate::plugin::PluginInfo] {
        self.registry.plugins()
    }

    fn collect_files(&self, path: &Path, out: &mut Vec<PathBuf>) {
        if path.is_file() {
            out.push(path.to_path_buf());
            return;
        }
        let mut stack = vec![path.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let Ok(entries) = std::fs::read_dir(&dir) else { continue };
            for entry in entries.flatten() {
                let p = entry.path();
                let rel = p.strip_prefix(path).unwrap_or(&p);
                if self.profile.exclude.is_match(rel) || self.removed.contains(&p) {
                    continue;
                }
                match entry.file_type() {
                    Ok(t) if t.is_dir() => stack.push(p),
                    Ok(t) if t.is_file() && self.profile.include.is_match(rel) => out.push(p),
                    _ => {}
                }
            }
        }
    }

    fn import_files(&mut self, files: Vec<PathBuf>) {
        let profile = self.profile.clone();
        let importers = &self.registry.importers;
        let named: Vec<(PathBuf, String)> = files
            .into_iter()
            .map(|p| {
                let name = display_name(&p, &self.roots);
                (p, name)
            })
            .collect();
        let removed = &self.removed;
        let results: Vec<(String, String, ImportResult)> = named
            .into_par_iter()
            .flat_map_iter(|(path, name)| import_file(importers, &profile, &path, &name, removed))
            .collect();

        for (path_s, name, result) in results {
            match result {
                Ok((importer, imported, step_stats)) => {
                    let dropped = step_stats.first().map_or(0, |s| s.input) - step_stats.last().map_or(0, |s| s.output);
                    let id = self.names.len() as u32;
                    self.names.push(name.clone());
                    let (mut from, mut to) = (None::<i64>, None::<i64>);
                    for r in &imported.records {
                        if let Some(ts) = r.ts {
                            from = Some(from.map_or(ts, |f| f.min(ts)));
                            to = Some(to.map_or(ts, |t| t.max(ts)));
                        }
                    }
                    self.sources.push(SourceInfo {
                        id,
                        path: path_s,
                        name,
                        importer,
                        records: imported.records.len(),
                        skipped: imported.skipped,
                        dropped,
                        step_stats,
                        error: None,
                        from,
                        to,
                    });
                    self.records.extend(imported.records.into_iter().map(|mut r| {
                        r.source = id;
                        r
                    }));
                }
                Err(reason) => self.unrecognized.push(UnrecognizedFile {
                    path: path_s,
                    name,
                    reason,
                }),
            }
        }
        self.sources.sort_by(|a, b| a.name.cmp(&b.name));
        self.unrecognized.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Re-sorts, re-numbers and re-derives everything after the record set changed.
    fn rebuild(&mut self) {
        self.records.par_sort_by_key(|r| (r.ts.unwrap_or(i64::MAX), r.source, r.line));
        self.records.par_iter_mut().enumerate().for_each(|(i, r)| r.id = i as u32);
        self.compute_fields();
        self.default_group_by = self.pick_group_by();
        *self.cache.lock().unwrap() = None;
    }

    fn compute_fields(&mut self) {
        struct Stat {
            count: usize,
            numeric: bool,
            values: HashSet<String>,
        }
        let mut stats: HashMap<String, Stat> = HashMap::new();
        for r in &self.records {
            for (k, v) in &r.fields {
                let s = stats.entry(k.clone()).or_insert_with(|| Stat {
                    count: 0,
                    numeric: true,
                    values: HashSet::new(),
                });
                s.count += 1;
                if s.numeric && crate::model::value_number(v).is_none() {
                    s.numeric = false;
                }
                if s.values.len() < MAX_DISTINCT {
                    s.values.insert(crate::model::value_text(v).into_owned());
                }
            }
        }
        let derived: HashSet<&str> = self
            .profile
            .extractors
            .iter()
            .flat_map(|e| e.fields.iter().map(|(n, _)| n.as_str()))
            .collect();
        let mut fields: Vec<FieldInfo> = stats
            .into_iter()
            .map(|(name, s)| FieldInfo {
                derived: derived.contains(name.as_str()),
                distinct: s.values.len(),
                count: s.count,
                numeric: s.numeric,
                name,
            })
            .collect();
        fields.sort_by(|a, b| b.count.cmp(&a.count).then_with(|| a.name.cmp(&b.name)));
        self.fields = fields;
    }

    fn pick_group_by(&self) -> Vec<String> {
        let cfg = &self.profile.profile.instance;
        if !cfg.key.is_empty() {
            return cfg.key.clone();
        }
        let total = self.records.len().max(1);
        let present: Vec<&FieldInfo> = cfg
            .candidates
            .iter()
            .filter_map(|c| self.fields.iter().find(|f| &f.name == c))
            .filter(|f| f.count * 2 >= total)
            .collect();
        // Constant fields (same value everywhere) do not tell instances apart.
        let varying: Vec<String> = present.iter().filter(|f| f.distinct > 1).map(|f| f.name.clone()).collect();
        if !varying.is_empty() {
            return varying;
        }
        if self.sources.len() <= 1 {
            if let Some(f) = present.first() {
                return vec![f.name.clone()];
            }
        }
        vec!["@source".into()]
    }

    fn source_name(&self, id: u32) -> &str {
        self.names.get(id as usize).map(String::as_str).unwrap_or("")
    }

    fn compile(&self, q: &Query) -> Result<CompiledQuery> {
        CompiledQuery::new(q, &self.default_group_by, &self.marker_kinds)
    }

    fn marker_names(&self, mask: u32) -> Vec<&str> {
        (0..self.marker_kinds.len())
            .filter(|i| mask & (1 << i) != 0)
            .map(|i| self.marker_kinds[i].as_str())
            .collect()
    }

    /// Ids (ascending) of every record matching the query, cached for paging.
    pub fn matching(&self, q: &Query) -> Result<Arc<Vec<u32>>> {
        let key = serde_json::to_string(q)?;
        if let Some((k, ids)) = self.cache.lock().unwrap().as_ref() {
            if *k == key {
                return Ok(ids.clone());
            }
        }
        let cq = self.compile(q)?;
        let ids: Vec<u32> = self
            .records
            .par_iter()
            .filter(|r| cq.matches(r, self.source_name(r.source)))
            .map(|r| r.id)
            .collect();
        let ids = Arc::new(ids);
        *self.cache.lock().unwrap() = Some((key, ids.clone()));
        Ok(ids)
    }

    pub fn summary(&self) -> WorkspaceSummary {
        let mut levels: BTreeMap<Level, usize> = Level::ALL.iter().map(|l| (*l, 0)).collect();
        let (mut from, mut to) = (None::<i64>, None::<i64>);
        for r in &self.records {
            *levels.get_mut(&r.level).unwrap() += 1;
            if let Some(ts) = r.ts {
                from = Some(from.map_or(ts, |f| f.min(ts)));
                to = Some(to.map_or(ts, |t| t.max(ts)));
            }
        }
        WorkspaceSummary {
            sources: self.sources.clone(),
            unrecognized: self.unrecognized.clone(),
            total: self.records.len(),
            from,
            to,
            levels,
            fields: self.fields.clone(),
            default_group_by: self.default_group_by.clone(),
            marker_kinds: self
                .profile
                .markers
                .iter()
                .map(|m| MarkerKind {
                    kind: m.kind.clone(),
                    label: m.label.clone(),
                    lifecycle: m.lifecycle,
                    color: m.color.clone(),
                })
                .collect(),
            charts: self.chart_defs(),
            profile_name: self.profile.profile.name.clone(),
            load_ms: self.load_ms,
            dropped: self.sources.iter().map(|s| s.dropped).sum(),
            pipeline: self.pipeline_stats(),
            plugins: self.registry.plugins().to_vec(),
            exporters: self
                .registry
                .exporters
                .iter()
                .map(|e| ExporterInfo {
                    id: e.id().into(),
                    name: e.name().into(),
                    extension: e.extension().into(),
                })
                .collect(),
        }
    }

    fn pipeline_stats(&self) -> Vec<PipelineStepInfo> {
        self.profile
            .profile
            .pipeline
            .iter()
            .enumerate()
            .map(|(index, cfg)| {
                let (input, output) = self
                    .sources
                    .iter()
                    .filter_map(|s| s.step_stats.get(index))
                    .fold((0, 0), |(i, o), s| (i + s.input, o + s.output));
                PipelineStepInfo {
                    index,
                    step: cfg.step.clone(),
                    input,
                    output,
                }
            })
            .collect()
    }

    fn chart_defs(&self) -> Vec<ChartDef> {
        self.profile
            .profile
            .chart
            .iter()
            .enumerate()
            .map(|(i, c)| ChartDef {
                id: c.id.clone().unwrap_or_else(|| format!("chart-{}", i + 1)),
                title: c.title.clone(),
                y: c.y.clone(),
                bucket: c.bucket.clone(),
                split_by: c.split_by.clone(),
                kind: c.kind.clone(),
                unit: c.unit.clone(),
            })
            .collect()
    }
}

/// Importer id, records and the number the pipeline dropped, or why the file
/// could not be read.
type ImportResult = std::result::Result<(String, crate::importers::Imported, Vec<crate::pipeline::StepStats>), String>;

/// Reads one file and runs the profile's ingest pipeline on its records.
/// Reads one file from disk, expands it if it is compressed, and imports
/// every resulting file. Returns (source path, display name, result) per file.
fn import_file(
    importers: &Importers,
    profile: &CompiledProfile,
    path: &Path,
    name: &str,
    removed: &HashSet<PathBuf>,
) -> Vec<(String, String, ImportResult)> {
    let path_s = path.to_string_lossy().into_owned();
    let bytes = match std::fs::read(path) {
        Ok(b) => b,
        Err(e) => return vec![(path_s, name.to_string(), Err(e.to_string()))],
    };
    let file_name = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    let compressed = crate::archive::is_zip(&bytes);
    let entries = match crate::archive::expand(&file_name, bytes) {
        Ok(e) => e,
        Err(e) => return vec![(path_s, name.to_string(), Err(format!("{e:#}")))],
    };
    let file_date = std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
        .and_then(|d| utc_date_of(d.as_millis() as i64));
    entries
        .into_iter()
        .filter_map(|entry| {
            // Zip members are separate sources; a gzip file stays one source.
            let (source_path, source_name) = match (&entry.name, compressed) {
                (Some(inner), true) => (
                    format!("{path_s}{}{inner}", crate::archive::ENTRY_SEPARATOR),
                    format!("{name}/{inner}"),
                ),
                _ => (path_s.clone(), name.to_string()),
            };
            if removed.contains(Path::new(&source_path)) {
                return None;
            }
            let result = import_bytes(importers, profile, path, &entry.bytes, &source_name, file_date);
            Some((source_path, source_name, result))
        })
        .collect()
}

fn import_bytes(
    importers: &Importers,
    profile: &CompiledProfile,
    path: &Path,
    bytes: &[u8],
    source_name: &str,
    file_date: Option<chrono::NaiveDate>,
) -> ImportResult {
    if bytes.is_empty() {
        return Err("empty file".into());
    }
    let content = decode(bytes).ok_or("binary file")?;
    let ctx = ImportCtx { profile, path, file_date };
    let head: String = content.chars().take(64 * 1024).collect();
    let importer = importers.select(&ctx, &head).ok_or("no importer recognises this format")?;
    let mut imported = importer.import(&ctx, &content).map_err(|e| format!("{e:#}"))?;
    if imported.records.is_empty() {
        return Err(format!("{} importer found no events", importer.id()));
    }
    let (records, stats) = profile.ingest(std::mem::take(&mut imported.records), source_name);
    imported.records = records;
    Ok((importer.id().to_string(), imported, stats))
}

/// Path relative to the opened folder it came from, else `<parent dir>/<file name>`.
fn display_name(path: &Path, roots: &[PathBuf]) -> String {
    roots
        .iter()
        .filter(|r| r.is_dir())
        .filter_map(|r| path.strip_prefix(r).ok())
        .min_by_key(|p| p.components().count())
        .map(|p| p.to_string_lossy().replace('\\', "/"))
        .unwrap_or_else(|| explicit_name(path))
}

fn explicit_name(path: &Path) -> String {
    let file = path.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
    match path.parent().and_then(Path::file_name) {
        Some(dir) => format!("{}/{file}", dir.to_string_lossy()),
        None => file,
    }
}
