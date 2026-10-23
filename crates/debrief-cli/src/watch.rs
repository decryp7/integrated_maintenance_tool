//! `debrief watch DIR`: imports files as they settle in a folder (ADR 0048) and says when a known issue appears. The
//! decisions are `debrief_core::watch::WatchState`'s; this module reads the folder, imports through the store ports and
//! prints one line (or JSON object) per event.

use crate::run::{read_knowledge, store_message};
use crate::BIN_NAME;
use debrief_core::known_issues::{match_known_issues, KnowledgeText};
use debrief_core::ports::Store;
use debrief_core::profile::CompiledProfile;
use debrief_core::watch::{scan_folder, FileStamp, Skipped, WatchConfig, WatchState};
use debrief_core::Profile;
use serde_json::json;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

/// What to watch and how.
pub struct WatchOptions {
    pub dir: PathBuf,
    pub knowledge: Vec<PathBuf>,
    pub interval_ms: u64,
    pub debounce_ms: u64,
    /// Settle what is in the folder now, then stop.
    pub once: bool,
    pub json: bool,
}

/// A flag Ctrl+C and SIGTERM set.
pub fn stop_flag() -> Arc<AtomicBool> {
    static FLAG: std::sync::OnceLock<Arc<AtomicBool>> = std::sync::OnceLock::new();
    FLAG.get_or_init(|| {
        let flag = Arc::new(AtomicBool::new(false));
        let f = flag.clone();
        // A second registration in one process (tests) fails harmlessly.
        let _ = ctrlc::set_handler(move || f.store(true, Ordering::Relaxed));
        flag
    })
    .clone()
}

fn emit(out: &mut dyn Write, json_mode: bool, value: serde_json::Value, line: String) -> Result<(), String> {
    let r = if json_mode {
        writeln!(out, "{value}")
    } else {
        writeln!(out, "{line}")
    };
    r.and_then(|()| out.flush()).map_err(|e| format!("could not write the output: {e}"))
}

/// Runs until `stop` (or, with `once`, until nothing is waiting). Returns the number of known issues announced.
pub fn watch(store: &dyn Store, opts: &WatchOptions, stop: &AtomicBool, out: &mut dyn Write, err: &mut dyn Write) -> Result<usize, String> {
    if !opts.dir.is_dir() {
        return Err(format!("{} is not a folder", opts.dir.display()));
    }
    let root = std::fs::canonicalize(&opts.dir).map_err(|e| format!("cannot read {}: {e}", opts.dir.display()))?;
    let compiled = CompiledProfile::compile(Profile::default_profile()).map_err(|e| format!("{e:#}"))?;
    let texts = read_knowledge(&opts.knowledge)?;
    let mut state = WatchState::new(WatchConfig {
        debounce_ms: opts.debounce_ms,
        ..WatchConfig::default()
    });
    let started = Instant::now();
    let mut announced = 0usize;
    let mut warned_cut = false;
    loop {
        let now = started.elapsed().as_millis() as u64;
        let (scan, cut) = scan_folder(&root, &|rel: &Path| {
            compiled.include.is_match(rel) && !compiled.exclude.is_match(rel)
        });
        if cut && !warned_cut {
            warned_cut = true;
            let _ = writeln!(
                err,
                "{BIN_NAME}: the folder has more files than one scan reads; the newest may be missed"
            );
        }
        let ready = state.observe(&scan, now);
        for (path, why) in &ready.skipped {
            let text = match why {
                Skipped::TooLarge { size } => format!("{} is {} bytes, too large to import automatically", path.display(), size),
                Skipped::GaveUp { reason } => format!("gave up on {}: {reason}", path.display()),
            };
            emit(
                out,
                opts.json,
                json!({"event": "skipped", "path": path.display().to_string(), "reason": text}),
                format!("skipped {text}"),
            )?;
        }
        let mut imported_any = false;
        let stamps: std::collections::HashMap<&Path, &FileStamp> = scan.iter().map(|f| (f.path.as_path(), f)).collect();
        for (path, refresh) in ready.new.iter().map(|p| (p, false)).chain(ready.changed.iter().map(|p| (p, true))) {
            let Some(stamp) = stamps.get(path.as_path()) else { continue };
            let one = std::slice::from_ref(path);
            let result = if refresh { store.refresh(one) } else { store.open(one) }
                .and_then(|s| s.wait())
                .map_err(store_message);
            let failure = match result {
                Ok(report) => match report.unrecognized.first() {
                    Some(u) => Some(u.reason.clone()),
                    None => {
                        state.imported(stamp);
                        imported_any = true;
                        emit(
                            out,
                            opts.json,
                            json!({"event": "imported", "path": path.display().to_string(), "events": report.events}),
                            format!("imported {} ({} events)", path.display(), report.events),
                        )?;
                        None
                    }
                },
                Err(e) => Some(e),
            };
            if let Some(reason) = failure {
                match state.failed(path, &reason, now) {
                    Some(Skipped::GaveUp { reason }) => emit(
                        out,
                        opts.json,
                        json!({"event": "skipped", "path": path.display().to_string(), "reason": reason}),
                        format!("gave up on {}: {reason}", path.display()),
                    )?,
                    _ => {
                        let _ = writeln!(err, "{BIN_NAME}: {} could not be read yet ({reason}); will retry", path.display());
                    }
                }
            }
        }
        if imported_any && !texts.is_empty() {
            let files: Vec<KnowledgeText<'_>> = texts.iter().map(|(name, text)| KnowledgeText { name, text }).collect();
            let report = match_known_issues(store, &files).map_err(store_message)?;
            for m in state.fresh_matches(&report) {
                announced += 1;
                emit(
                    out,
                    opts.json,
                    json!({"event": "issue", "id": m.issue.id, "title": m.issue.title, "count": m.count, "fix": m.issue.fix}),
                    format!("known issue: {} ({} events). {}", m.issue.title, m.count, m.issue.fix),
                )?;
            }
        }
        if stop.load(Ordering::Relaxed) || (opts.once && state.pending_len() == 0) {
            return Ok(announced);
        }
        // Sleep in short steps so Ctrl+C is prompt.
        let until = Instant::now()
            + Duration::from_millis(if opts.once {
                opts.debounce_ms.min(opts.interval_ms)
            } else {
                opts.interval_ms
            });
        while Instant::now() < until && !stop.load(Ordering::Relaxed) {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
}
