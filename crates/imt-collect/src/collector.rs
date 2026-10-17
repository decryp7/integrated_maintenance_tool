//! Orchestration: runs the plan and writes the `.imtz` bundle.

use crate::bundle::Bundle;
use crate::env;
use crate::manifest::*;
use crate::plan::CollectPlan;
use crate::redact::{Counts, Redactor};
use crate::runner::{CommandError, CommandOutput, CommandRunner};
use crate::select::select_logs;
use anyhow::{Context, Result};
use chrono::{DateTime, Local};
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant, SystemTime};

/// Receives coarse progress so a UI can show it.
pub trait Progress {
    fn step(&self, name: &str, done: usize, total: usize);
}

/// Ignores progress.
pub struct NoProgress;

impl Progress for NoProgress {
    fn step(&self, _name: &str, _done: usize, _total: usize) {}
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    Windows,
    MacOs,
    Linux,
    Other,
}

impl Platform {
    pub fn current() -> Platform {
        if cfg!(windows) {
            Platform::Windows
        } else if cfg!(target_os = "macos") {
            Platform::MacOs
        } else if cfg!(target_os = "linux") {
            Platform::Linux
        } else {
            Platform::Other
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Platform::Windows => "windows",
            Platform::MacOs => "macos",
            Platform::Linux => "linux",
            Platform::Other => "other",
        }
    }
}

/// Largest file read into memory for redaction.
const MAX_FILE_BYTES: u64 = 256 << 20;

#[derive(Debug)]
pub struct CollectResult {
    pub bundle: PathBuf,
    pub manifest: Manifest,
}

pub struct Collector<'a> {
    pub plan: CollectPlan,
    pub runner: &'a dyn CommandRunner,
    pub progress: &'a dyn Progress,
    pub platform: Platform,
    pub now: DateTime<Local>,
    pub host: String,
    /// Reported as the installed IMT version in the environment snapshot.
    pub imt_version: String,
}

impl<'a> Collector<'a> {
    pub fn new(plan: CollectPlan, runner: &'a dyn CommandRunner, progress: &'a dyn Progress) -> Collector<'a> {
        Collector {
            plan,
            runner,
            progress,
            platform: Platform::current(),
            now: Local::now(),
            host: env::host_name().unwrap_or_else(|| "unknown-host".into()),
            imt_version: env!("CARGO_PKG_VERSION").into(),
        }
    }

    /// The bundle file name: `<name>-<host>-<yyyyMMdd-HHmmss>.imtz`.
    pub fn bundle_file_name(&self) -> String {
        format!(
            "{}-{}-{}.imtz",
            safe_segment(&self.plan.name),
            safe_segment(&self.host),
            self.now.format("%Y%m%d-%H%M%S")
        )
    }

    fn total_steps(&self) -> usize {
        let p = &self.plan;
        usize::from(p.environment)
            + p.event_logs.len()
            + usize::from(p.msinfo)
            + p.registry.len()
            + usize::from(p.counters)
            + usize::from(p.macos_unified_log)
            + usize::from(p.linux_journal)
            + p.logs.len()
    }

    /// Runs the plan and writes the bundle into `out_dir` (created if needed).
    /// Problems with individual items are recorded in the manifest; only
    /// failures that make a bundle impossible are returned as errors.
    pub fn run(&self, out_dir: &Path) -> Result<CollectResult> {
        self.plan.validate()?;
        let redactor = Redactor::new(&self.plan.redact)?;
        std::fs::create_dir_all(out_dir).with_context(|| format!("cannot create {}", out_dir.display()))?;
        let final_path = out_dir.join(self.bundle_file_name());
        let part = final_path.with_extension("imtz.part");
        let budget = self.plan.max_bundle_mb.saturating_mul(1 << 20);
        let scratch = tempfile::tempdir().context("cannot create a scratch folder")?;
        let mut run = Run {
            c: self,
            bundle: Bundle::create(&part, budget)?,
            redactor,
            scratch: scratch.path().to_path_buf(),
            manifest: self.new_manifest(budget),
            names: HashSet::new(),
            done: 0,
            total: self.total_steps(),
            started: Instant::now(),
        };
        let finished = run.execute().and_then(|_| self.seal(run, &part, &final_path));
        if finished.is_err() {
            let _ = std::fs::remove_file(&part);
        }
        finished
    }

    /// Writes the project file and manifest, then moves the bundle into place.
    fn seal(&self, run: Run, part: &Path, final_path: &Path) -> Result<CollectResult> {
        let Run {
            mut bundle,
            mut manifest,
            started,
            ..
        } = run;
        manifest.total_bytes = bundle.used();
        manifest.duration_ms = started.elapsed().as_millis() as u64;
        bundle.add_meta("project.imtproj", self.project_json(top_folders(&manifest))?.as_bytes())?;
        bundle.add_meta("manifest.json", serde_json::to_string_pretty(&manifest)?.as_bytes())?;
        bundle.finish()?;
        std::fs::rename(part, final_path).with_context(|| format!("cannot move the bundle to {}", final_path.display()))?;
        Ok(CollectResult {
            bundle: final_path.to_path_buf(),
            manifest,
        })
    }

    fn new_manifest(&self, budget: u64) -> Manifest {
        let mut versions = BTreeMap::new();
        versions.insert("imt-collect".to_string(), env!("CARGO_PKG_VERSION").to_string());
        versions.insert("imt".to_string(), self.imt_version.clone());
        Manifest {
            schema: MANIFEST_SCHEMA,
            name: self.plan.name.clone(),
            host: self.host.clone(),
            platform: self.platform.label().into(),
            created_at: self.now.to_rfc3339(),
            tool_versions: versions,
            items: Vec::new(),
            skipped: Vec::new(),
            commands: Vec::new(),
            redaction: Counts::new(),
            errors: Vec::new(),
            steps: Vec::new(),
            total_bytes: 0,
            max_bundle_bytes: budget,
            duration_ms: 0,
            plan: self.plan.clone(),
        }
    }

    fn project_json(&self, sources: Vec<String>) -> Result<String> {
        let project = imt_core::project::ProjectFile {
            version: imt_core::project::PROJECT_VERSION,
            name: self.plan.name.clone(),
            sources,
            removed: Vec::new(),
            profile: imt_core::DEFAULT_PROFILE_TOML.to_string(),
            ui: serde_json::json!({}),
        };
        Ok(serde_json::to_string_pretty(&project)?)
    }
}

fn top_folders(m: &Manifest) -> Vec<String> {
    ["logs", "eventlogs", "system"]
        .into_iter()
        .filter(|dir| m.items.iter().any(|i| i.path.starts_with(&format!("{dir}/"))))
        .map(String::from)
        .collect()
}

/// A name that is safe as one path segment inside the bundle (and cannot form
/// the "!/" archive-entry separator).
pub fn safe_segment(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, '.' | '_' | '-' | ' ' | '+' | '(' | ')' | '=' | '@' | ',') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let t = cleaned.trim().trim_matches('.');
    if t.is_empty() {
        "_".into()
    } else {
        t.to_string()
    }
}

/// A file ready to be written to the bundle.
struct Stored<'p> {
    kind: ItemKind,
    path: &'p str,
    origin: &'p str,
    original: u64,
    out: Vec<u8>,
    redacted: bool,
    counts: Counts,
    note: Option<String>,
}

struct Run<'a, 'b> {
    c: &'b Collector<'a>,
    bundle: Bundle,
    redactor: Redactor,
    scratch: PathBuf,
    manifest: Manifest,
    names: HashSet<String>,
    done: usize,
    total: usize,
    started: Instant,
}

impl Run<'_, '_> {
    fn execute(&mut self) -> Result<()> {
        if self.c.plan.environment {
            self.step("environment", |r| r.environment())?;
        }
        let win = self.c.platform == Platform::Windows;
        let timeout = Duration::from_secs(self.c.plan.command_timeout_secs);

        for name in self.c.plan.event_logs.clone() {
            self.step(&format!("event log {name}"), |r| {
                if !win {
                    r.skip(ItemKind::EventLog, &name, "Windows event logs are only available on Windows");
                    return Ok(());
                }
                let file = format!("{}.evtx", safe_segment(&name));
                let ms = u64::from(r.c.plan.event_log_days) * 86_400_000;
                let query = format!("/q:*[System[TimeCreated[timediff(@SystemTime) <= {ms}]]]");
                let out = r.scratch.join(&file);
                let args = vec!["epl".into(), name.clone(), out.display().to_string(), query, "/ow:true".into()];
                r.tool_file(ItemKind::EventLog, "wevtutil", args, &out, &format!("eventlogs/{file}"), timeout)
            })?;
        }
        if self.c.plan.msinfo {
            self.step("msinfo", |r| {
                if !win {
                    r.skip(ItemKind::Msinfo, "msinfo32", "msinfo32 is only available on Windows");
                    return Ok(());
                }
                let out = r.scratch.join("msinfo.nfo");
                let args = vec!["/nfo".into(), out.display().to_string()];
                r.tool_file(ItemKind::Msinfo, "msinfo32", args, &out, "system/msinfo.nfo", timeout)
            })?;
        }
        for key in self.c.plan.registry.clone() {
            self.step(&format!("registry {key}"), |r| {
                if !win {
                    r.skip(ItemKind::Registry, &key, "registry export is only available on Windows");
                    return Ok(());
                }
                let file = format!("registry-{}.reg", safe_segment(&key.replace('\\', "-")));
                let out = r.scratch.join(&file);
                let args = vec!["export".into(), key.clone(), out.display().to_string(), "/y".into()];
                r.tool_file(ItemKind::Registry, "reg", args, &out, &format!("system/{file}"), timeout)
            })?;
        }
        if self.c.plan.counters {
            self.step("counters", |r| {
                if !win {
                    r.skip(ItemKind::Counters, "typeperf", "performance counters are only available on Windows");
                    return Ok(());
                }
                let out = r.scratch.join("counters.csv");
                let mut args: Vec<String> = vec!["-sc".into(), "5".into()];
                args.extend(r.c.plan.counter_paths.iter().cloned());
                args.extend(["-o".into(), out.display().to_string(), "-y".into()]);
                r.tool_file(ItemKind::Counters, "typeperf", args, &out, "system/counters.csv", timeout)
            })?;
        }
        let hours = self.c.plan.recent_log_hours;
        if self.c.plan.macos_unified_log {
            self.step("unified log", |r| {
                if r.c.platform != Platform::MacOs {
                    r.skip(ItemKind::UnifiedLog, "log show", "the unified log is only available on macOS");
                    return Ok(());
                }
                let args = vec!["show".into(), "--last".into(), format!("{hours}h"), "--style".into(), "json".into()];
                r.tool_stdout(ItemKind::UnifiedLog, "log", args, "system/unified-log.json", timeout)
            })?;
        }
        if self.c.plan.linux_journal {
            self.step("journal", |r| {
                if r.c.platform != Platform::Linux {
                    r.skip(ItemKind::Journal, "journalctl", "the systemd journal is only available on Linux");
                    return Ok(());
                }
                let args = vec!["-o".into(), "json".into(), "--since".into(), format!("-{hours}h")];
                r.tool_stdout(ItemKind::Journal, "journalctl", args, "system/journal.json", timeout)
            })?;
        }
        self.logs()
    }

    /// Times a step and reports progress.
    fn step(&mut self, name: &str, f: impl FnOnce(&mut Self) -> Result<()>) -> Result<()> {
        let t = Instant::now();
        let r = f(self);
        self.finish_step(name, t);
        r
    }

    fn finish_step(&mut self, name: &str, started: Instant) {
        self.done += 1;
        self.manifest.steps.push(StepTiming {
            name: name.into(),
            ms: started.elapsed().as_millis() as u64,
        });
        self.c.progress.step(name, self.done, self.total);
    }

    fn skip(&mut self, kind: ItemKind, what: &str, reason: &str) {
        self.manifest.skipped.push(Skipped {
            kind,
            what: what.into(),
            reason: reason.into(),
        });
    }

    fn environment(&mut self) -> Result<()> {
        let v = env::snapshot(&self.c.plan.env_allowlist, std::env::vars(), &self.c.imt_version);
        let mut counts = Counts::new();
        let v = self.redactor.apply_json(&v, &mut counts);
        let bytes = serde_json::to_vec_pretty(&v)?;
        let redacted = !self.redactor.is_empty();
        let note = (!redacted).then(|| "redaction disabled".to_string());
        let n = bytes.len() as u64;
        self.store(Stored {
            kind: ItemKind::Environment,
            path: "system/environment.json",
            origin: "this machine",
            original: n,
            out: bytes,
            redacted,
            counts,
            note,
        })?;
        Ok(())
    }

    /// Runs a tool and records the invocation; None when it did not run to completion.
    fn exec(&mut self, program: &str, args: &[String], timeout: Duration) -> Option<CommandOutput> {
        let t = Instant::now();
        let res = self.c.runner.run(program, args, timeout);
        let ms = t.elapsed().as_millis() as u64;
        let (status, exit_code, stderr, out) = match res {
            Ok(o) => (
                if o.success() { CommandStatus::Ok } else { CommandStatus::Failed },
                o.exit_code,
                o.stderr.clone(),
                Some(o),
            ),
            Err(e) => {
                let status = match e {
                    CommandError::NotFound(_) => CommandStatus::NotFound,
                    CommandError::TimedOut(..) => CommandStatus::TimedOut,
                    CommandError::Io(..) => CommandStatus::Failed,
                };
                (status, None, e.to_string(), None)
            }
        };
        if status != CommandStatus::Ok {
            let detail = if stderr.is_empty() {
                format!("{status:?}")
            } else {
                stderr.chars().take(500).collect()
            };
            self.manifest.errors.push(format!("{program} failed: {detail}"));
        }
        self.manifest.commands.push(CommandRecord {
            program: program.into(),
            args: args.to_vec(),
            status,
            exit_code,
            stderr: stderr.chars().take(2000).collect(),
            ms,
        });
        out
    }

    /// Tool that writes `file`; the file is added if it exists afterwards.
    fn tool_file(
        &mut self,
        kind: ItemKind,
        program: &str,
        args: Vec<String>,
        file: &Path,
        bundle_path: &str,
        timeout: Duration,
    ) -> Result<()> {
        let origin = format!("{program} {}", args.join(" "));
        self.exec(program, &args, timeout);
        match std::fs::read(file) {
            Ok(bytes) => {
                self.add(kind, bundle_path, &origin, bytes)?;
            }
            Err(_) => self.skip(kind, program, "the tool produced no output file (see errors)"),
        }
        Ok(())
    }

    /// Tool whose stdout is the result.
    fn tool_stdout(&mut self, kind: ItemKind, program: &str, args: Vec<String>, bundle_path: &str, timeout: Duration) -> Result<()> {
        let origin = format!("{program} {}", args.join(" "));
        match self.exec(program, &args, timeout) {
            Some(o) if o.success() => {
                self.add(kind, bundle_path, &origin, o.stdout)?;
            }
            _ => self.skip(kind, program, "the tool failed (see errors)"),
        }
        Ok(())
    }

    fn logs(&mut self) -> Result<()> {
        let now: SystemTime = self.c.now.into();
        let mut files = Vec::new();
        let mut labels: HashSet<String> = HashSet::new();
        let c = self.c;
        for spec in &c.plan.logs {
            let t = Instant::now();
            let (selected, rejected) = select_logs(spec, now);
            for r in rejected {
                self.skip(ItemKind::Log, &r.path, &r.reason);
            }
            let p = Path::new(&spec.path);
            let dir = if p.is_file() { p.parent() } else { Some(p) };
            let base = safe_segment(
                &dir.and_then(|d| d.file_name())
                    .map(|b| b.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "root".into()),
            );
            let mut label = base.clone();
            let mut n = 2;
            while !labels.insert(label.clone()) {
                label = format!("{base}-{n}");
                n += 1;
            }
            files.extend(selected.into_iter().map(|s| (label.clone(), s)));
            self.finish_step(&format!("logs {}", spec.path), t);
        }
        // Newest first across all sources, so the oldest are dropped when the bundle is full.
        files.sort_by(|a, b| b.1.modified.cmp(&a.1.modified).then_with(|| a.1.path.cmp(&b.1.path)));
        let mut full = false;
        for (label, f) in files {
            let origin = f.path.display().to_string();
            if full {
                self.skip(
                    ItemKind::Log,
                    &origin,
                    "bundle size limit (maxBundleMb) reached; oldest logs are dropped first",
                );
                continue;
            }
            if f.size > MAX_FILE_BYTES {
                self.skip(ItemKind::Log, &origin, "file is too large to copy and redact");
                continue;
            }
            let rel: Vec<String> = f.rel.split('/').map(safe_segment).collect();
            let bundle_path = format!("logs/{label}/{}", rel.join("/"));
            match std::fs::read(&f.path) {
                Ok(bytes) => full = !self.add(ItemKind::Log, &bundle_path, &origin, bytes)?,
                Err(e) => {
                    self.manifest.errors.push(format!("{origin}: {e}"));
                    self.skip(ItemKind::Log, &origin, "cannot be read");
                }
            }
        }
        Ok(())
    }

    /// Redacts text (binary data is copied as is), then stores it. Returns
    /// false when the bundle size limit stopped it.
    fn add(&mut self, kind: ItemKind, path: &str, origin: &str, bytes: Vec<u8>) -> Result<bool> {
        let original = bytes.len() as u64;
        let mut counts = Counts::new();
        let mut note = None;
        let (out, redacted) = if self.redactor.is_empty() {
            note = Some("redaction disabled".to_string());
            (bytes, false)
        } else if let Some(text) = imt_core::importers::decode(&bytes) {
            (self.redactor.apply(&text, &mut counts).into_bytes(), true)
        } else {
            note = Some("binary file, not redacted".to_string());
            (bytes, false)
        };
        self.store(Stored {
            kind,
            path,
            origin,
            original,
            out,
            redacted,
            counts,
            note,
        })
    }

    /// Writes an already-redacted file and records it in the manifest.
    fn store(&mut self, s: Stored) -> Result<bool> {
        let path = self.unique(s.path);
        if !self.bundle.try_add(&path, &s.out)? {
            self.names.remove(&path);
            self.skip(s.kind, s.origin, "bundle size limit (maxBundleMb) reached");
            return Ok(false);
        }
        let redactions: usize = s.counts.values().sum();
        for (k, v) in s.counts {
            *self.manifest.redaction.entry(k).or_insert(0) += v;
        }
        self.manifest.items.push(Item {
            path,
            kind: s.kind,
            origin: s.origin.into(),
            original_bytes: s.original,
            bytes: s.out.len() as u64,
            redacted: s.redacted,
            redactions,
            note: s.note,
        });
        Ok(true)
    }

    /// Reserves a unique entry name (`a.log`, `a-2.log`, ...).
    fn unique(&mut self, path: &str) -> String {
        if self.names.insert(path.to_string()) {
            return path.to_string();
        }
        let (stem, ext) = path
            .rsplit_once('.')
            .map(|(a, b)| (a.to_string(), format!(".{b}")))
            .unwrap_or((path.to_string(), String::new()));
        (2..)
            .map(|n| format!("{stem}-{n}{ext}"))
            .find(|cand| self.names.insert(cand.clone()))
            .unwrap_or_else(|| path.to_string())
    }
}
