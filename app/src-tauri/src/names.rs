//! Human-name matching in the shell (ADR 0053): each window holds the [`Names`] of its workspace, rebuilt when the
//! entity catalogue is built, persisted (user corrections and AI aliases) in the configuration folder, and fed by a
//! low-priority background job that asks Debrief's own local model for aliases.
//!
//! Thin by design: grouping, matching, ranking, questions and validation of the model's answers are the engine's
//! (`debrief_core::names`); this module owns the files, the Jobs entry and the commands.
//!
//! The alias job runs as a quiet maintenance job (its own queue, so it never sits in front of a chat) and waits
//! for the assistant's queue to be empty before every request to the model; it is skipped when the Settings switch is
//! off, when no model is set up, or when the model is not in memory and is not loaded automatically (it never loads a
//! model by itself).
//!
//! Commands (all validated here, the engine validates again): `names_accept`, `names_reject`, `names_undo`,
//! `names_remove_alias`; `find_values` (in `search_everything`) answers through [`WindowNames::search`].

use crate::ai::commands::core;
use crate::error::{CmdError, ErrorCode};
use crate::jobs::{JobCtx, JobKind, JobSpec, Jobs};
use crate::windows::{CurrentWindow, WindowState, Windows};
use crate::CmdResult;
use debrief_ai::names_model::LocalAliasModel;
use debrief_core::find::Resolved;
use debrief_core::names::alias_job::{AliasJobConfig, AliasModel};
use debrief_core::names::book::{NameBook, UserFact, MAX_TERM_CHARS};
use debrief_core::names::{NameSettings, Names};
use debrief_core::ports::Store;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime};

/// File of the name book below the configuration folder.
pub const BOOK_FILE: &str = "names/book.json";
/// Longest group id accepted from the webview.
const MAX_ID_CHARS: usize = 256;
/// How often the import watcher asks whether the catalogue is built, and how long it keeps asking.
const WATCH_POLL: Duration = Duration::from_millis(500);
const WATCH_MAX: Duration = Duration::from_secs(10 * 60);
/// How often the alias job asks whether the assistant is idle, and how long it waits for that.
const IDLE_POLL: Duration = Duration::from_millis(500);
const IDLE_MAX: Duration = Duration::from_secs(10 * 60);

/// What the table was last built from: events and sources of the window.
type Signature = (u64, usize);

/// Whether [`WindowNames::ensure_fresh`] had to rebuild the table.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Freshness {
    /// Built from the workspace as it is.
    Current,
    /// Rebuilt just now (new names may need aliases).
    Rebuilt,
    /// The catalogue is not built yet (or the store cannot say): the name layer stays as it was.
    NotBuilt,
}

#[derive(Default)]
struct Inner {
    names: Arc<Names>,
    file: Option<PathBuf>,
    seen: Mutex<Option<Signature>>,
    /// An alias job of the window is queued or running.
    job: AtomicBool,
}

/// A window's name layer. Cheap to clone.
#[derive(Clone, Default)]
pub struct WindowNames(Arc<Inner>);

impl WindowNames {
    /// The layer with its book read from `file` (a missing or damaged file gives an empty book and a log line).
    pub fn open(file: Option<PathBuf>) -> WindowNames {
        let names = Arc::new(Names::default());
        if let Some(f) = &file {
            let (book, note) = NameBook::load(f);
            if let Some(n) = note {
                log::warn!("{n}");
            }
            names.set_book(book);
        }
        WindowNames(Arc::new(Inner {
            names,
            file,
            ..Inner::default()
        }))
    }

    pub fn names(&self) -> &Arc<Names> {
        &self.0.names
    }

    /// Whether an alias job of the window is queued or running.
    #[cfg(test)]
    pub fn job_pending(&self) -> bool {
        self.0.job.load(Ordering::SeqCst)
    }

    /// Writes the book (user corrections and AI aliases). A failure is logged: the session keeps working.
    pub fn save(&self) {
        let Some(file) = &self.0.file else { return };
        if let Some(dir) = file.parent() {
            if let Err(e) = std::fs::create_dir_all(dir) {
                log::warn!("the name book folder cannot be created: {e}");
                return;
            }
        }
        if let Err(e) = self.0.names.read(|b, _| b.save(file)) {
            log::warn!("the name book cannot be saved: {e}");
        }
    }

    /// Applies the Settings switches.
    pub fn apply_settings(&self, ai_aliases: bool) {
        self.0.names.set_settings(NameSettings {
            ai_aliases,
            ..self.0.names.settings()
        });
    }

    /// Rebuilds the table when the workspace changed since the last build and its catalogue is built.
    pub fn ensure_fresh(&self, store: &dyn Store) -> Freshness {
        let Ok(summary) = store.summary() else { return Freshness::NotBuilt };
        let signature = (summary.total as u64, summary.sources.len());
        if *crate::sync::lock(&self.0.seen) == Some(signature) && self.0.names.is_built() {
            return Freshness::Current;
        }
        if self.0.names.refresh(store).is_err() || !self.0.names.is_built() {
            return Freshness::NotBuilt;
        }
        *crate::sync::lock(&self.0.seen) = Some(signature);
        Freshness::Rebuilt
    }

    /// The layer for the assistant: up to date, with the Settings switches applied.
    pub fn for_assistant(&self, store: Option<&dyn Store>, ai_aliases: bool) -> Arc<Names> {
        self.apply_settings(ai_aliases);
        if let Some(s) = store {
            self.ensure_fresh(s);
        }
        self.0.names.clone()
    }

    /// The values and entities the words refer to (see [`Names::search`]).
    pub fn search(&self, store: &dyn Store, text: &str, limit: usize) -> CmdResult<(Resolved, Freshness)> {
        let fresh = self.ensure_fresh(store);
        let found = self
            .0
            .names
            .search(store, text, limit)
            .map_err(|e| CmdError::new(ErrorCode::Internal, format!("the values cannot be looked up: {e}")))?;
        Ok((found, fresh))
    }
}

// The alias job ----------------------------------------------------------------------------------------------------

/// A model that lets the assistant go first: before each request it waits until no AI job is queued or running.
struct Polite<'a> {
    inner: &'a dyn AliasModel,
    jobs: Option<Jobs>,
    cancel: Arc<AtomicBool>,
    poll: Duration,
    max_wait: Duration,
}

impl AliasModel for Polite<'_> {
    fn version(&self) -> String {
        self.inner.version()
    }

    fn complete(&self, prompt: &str) -> Result<String, String> {
        if let Some(jobs) = &self.jobs {
            let started = std::time::Instant::now();
            while !jobs.find_active(|j| j.kind == JobKind::Ai).is_empty() {
                if self.cancel.load(Ordering::Relaxed) {
                    return Err("cancelled".into());
                }
                if started.elapsed() > self.max_wait {
                    return Err("the assistant stayed busy; the names are learned next time".into());
                }
                std::thread::sleep(self.poll);
            }
        }
        self.inner.complete(prompt)
    }
}

/// How an alias job ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AliasEnd {
    Done(String),
    Cancelled,
    Failed(String),
}

/// Runs the alias job of `win` with `model` (none: nothing to ask), then saves the book.
pub fn run_alias_job(
    win: &WindowNames,
    model: Option<&dyn AliasModel>,
    jobs: Option<Jobs>,
    ctx: &JobCtx,
    cfg: &AliasJobConfig,
) -> AliasEnd {
    let polite = model.map(|m| Polite {
        inner: m,
        jobs,
        cancel: ctx.cancel_flag(),
        poll: IDLE_POLL,
        max_wait: IDLE_MAX,
    });
    let report = win
        .0
        .names
        .run_alias_job(polite.as_ref().map(|p| p as &dyn AliasModel), cfg, &|| ctx.is_cancelled());
    if report.stored > 0 {
        win.save();
    }
    if report.cancelled || ctx.is_cancelled() {
        return AliasEnd::Cancelled;
    }
    if !report.skipped.is_empty() {
        return AliasEnd::Done(format!("Skipped: {}", report.skipped));
    }
    match report.error {
        Some(e) => AliasEnd::Failed(e),
        None => AliasEnd::Done(format!("{} names learned, {} already known", report.stored, report.cached)),
    }
}

/// Queues the window's alias job (once at a time) when the switch is on and the local model is ready.
pub fn schedule_alias_job<R: Runtime>(app: &AppHandle<R>, label: &str, win: &WindowNames) -> bool {
    if win.0.job.swap(true, Ordering::SeqCst) {
        return false;
    }
    let started = (|| {
        let core = core(app).ok()?;
        if !core.settings().name_aliases || !core.background_ready() {
            return None;
        }
        let jobs = app.try_state::<Jobs>()?.inner().clone();
        let spec = JobSpec::new(JobKind::Maintenance, "Learning human names").window(label).quiet();
        let (worker, service) = (win.clone(), jobs.clone());
        jobs.submit_work(spec, crate::jobs::Controls::default(), move |ctx| {
            let end = match core.provider() {
                Ok(provider) => {
                    let model = LocalAliasModel::new(provider, ctx.cancel_flag());
                    run_alias_job(&worker, Some(&model), Some(service), ctx, &AliasJobConfig::default())
                }
                Err(e) => AliasEnd::Failed(e.message),
            };
            worker.0.job.store(false, Ordering::SeqCst);
            match end {
                AliasEnd::Done(m) => {
                    ctx.message(&m);
                    crate::jobs::queue::JobEnd::Done(None)
                }
                AliasEnd::Cancelled => crate::jobs::queue::JobEnd::Cancelled,
                AliasEnd::Failed(e) => crate::jobs::queue::JobEnd::Failed(e),
            }
        })
        .ok()
    })();
    if started.is_none() {
        win.0.job.store(false, Ordering::SeqCst);
    }
    started.is_some()
}

/// Waits (in the background) for the window's catalogue to be built after an import, then rebuilds the name table and
/// starts the alias job. Ends when the window is gone, the table is current, or after `max`.
pub fn watch_catalogue<R: Runtime>(app: &AppHandle<R>, label: &str, poll: Duration, max: Duration) {
    let (app, label) = (app.clone(), label.to_string());
    let spawned = std::thread::Builder::new().name("debrief-names".into()).spawn(move || {
        let started = std::time::Instant::now();
        while started.elapsed() < max {
            let Some(win) = app.try_state::<Windows>().and_then(|w| w.get(&label)) else {
                return;
            };
            let Ok(store) = win.engine.store() else { return };
            match win.names.ensure_fresh(store.as_ref()) {
                Freshness::Rebuilt => {
                    schedule_alias_job(&app, &label, &win.names);
                    return;
                }
                Freshness::Current => return,
                Freshness::NotBuilt => {}
            }
            drop((win, store));
            std::thread::sleep(poll);
        }
    });
    if let Err(e) = spawned {
        log::warn!("the name watcher cannot start: {e}");
    }
}

/// An import finished in `label`: its names are learned once the catalogue is ready.
pub fn after_import<R: Runtime>(app: &AppHandle<R>, label: &str) {
    watch_catalogue(app, label, WATCH_POLL, WATCH_MAX);
}

// Commands ---------------------------------------------------------------------------------------------------------

/// What the user said about a match: the words they typed and the entity (`group` of a `ValueMatch`) they mean or not.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NameFeedback {
    pub text: String,
    pub group: String,
    /// Remember it for every workspace, not only this one.
    #[serde(default)]
    pub global: bool,
}

/// An AI alias to remove from an entity.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemoveAlias {
    pub group: String,
    pub alias: String,
}

/// The fact that was recorded (its id undoes it), or none when the book already said so.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FactReply {
    pub fact: Option<u64>,
}

/// What `names_undo` took back.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UndoReply {
    pub undone: Option<UserFact>,
}

/// What `names_remove_alias` did.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemovedReply {
    pub removed: bool,
}

fn invalid(message: impl Into<String>) -> CmdError {
    CmdError::new(ErrorCode::Invalid, message)
}

/// A phrase from the webview: non-empty, bounded, no control characters.
fn phrase(what: &str, s: &str) -> CmdResult<String> {
    let t = s.trim();
    if t.is_empty() {
        return Err(invalid(format!("{what} is empty")));
    }
    if t.chars().count() > MAX_TERM_CHARS {
        return Err(invalid(format!("{what} is longer than {MAX_TERM_CHARS} characters")));
    }
    if t.chars().any(char::is_control) {
        return Err(invalid(format!("{what} holds control characters")));
    }
    Ok(t.to_string())
}

/// A group id the book knows.
fn known_group(names: &Names, id: &str) -> CmdResult<String> {
    if id.is_empty() || id.chars().count() > MAX_ID_CHARS {
        return Err(invalid("the name is not valid; search again"));
    }
    if names.read(|b, _| b.group(id).is_none()) {
        return Err(invalid("that name is no longer in the logs; search again"));
    }
    Ok(id.to_string())
}

fn feedback(
    state: &WindowState,
    request: &NameFeedback,
    apply: impl FnOnce(&mut NameBook, &str, &str, bool) -> Option<u64>,
) -> CmdResult<FactReply> {
    let text = phrase("the words", &request.text)?;
    let group = known_group(state.names.names(), &request.group)?;
    let fact = state.names.names().write(|b| apply(b, &text, &group, request.global));
    state.names.save();
    Ok(FactReply { fact })
}

/// "Yes, that is what I meant": the words now mean the entity (outranks everything, including the AI's aliases).
#[tauri::command]
pub async fn names_accept(state: CurrentWindow, request: NameFeedback) -> CmdResult<FactReply> {
    feedback(&state, &request, NameBook::accept)
}

/// "No, not that": the words never mean the entity again.
#[tauri::command]
pub async fn names_reject(state: CurrentWindow, request: NameFeedback) -> CmdResult<FactReply> {
    feedback(&state, &request, NameBook::reject)
}

/// Takes back the user's latest correction.
#[tauri::command]
pub async fn names_undo(state: CurrentWindow) -> CmdResult<UndoReply> {
    let undone = state.names.names().write(NameBook::undo);
    state.names.save();
    Ok(UndoReply { undone })
}

/// Removes an AI-suggested alias (it is not suggested again by the same model).
#[tauri::command]
pub async fn names_remove_alias(state: CurrentWindow, request: RemoveAlias) -> CmdResult<RemovedReply> {
    let alias = phrase("the alias", &request.alias)?;
    let group = known_group(state.names.names(), &request.group)?;
    let removed = state.names.names().write(|b| b.remove_ai_alias(&group, &alias));
    if removed {
        state.names.save();
    }
    Ok(RemovedReply { removed })
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_core::names::alias_job::AliasModel;
    use debrief_core::names::group::NameGroup;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("debrief-names-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    struct Fixed(&'static str);
    impl AliasModel for Fixed {
        fn version(&self) -> String {
            "fixed".into()
        }
        fn complete(&self, _: &str) -> Result<String, String> {
            Ok(self.0.into())
        }
    }

    fn with_group(win: &WindowNames, spelling: &str, count: u64) {
        use debrief_core::catalogue::{Candidate, Scope};
        use debrief_core::entities::EntityKind;
        use debrief_core::names::group::group;
        let c = Candidate {
            scope: Scope::Entity(EntityKind::Executable),
            name: spelling.into(),
            count,
            first_ts: None,
            last_ts: None,
            seen_in: vec![],
        };
        let groups: Vec<NameGroup> = group(&[c], &debrief_core::names::Vocabulary::builtin());
        win.names().write(|b| b.rebuild(groups));
    }

    #[test]
    fn the_book_survives_a_restart_and_tolerates_bad_files() {
        let dir = temp("persist");
        let file = dir.join(BOOK_FILE);
        let win = WindowNames::open(Some(file.clone()));
        with_group(&win, "graphicbuilder.exe", 9);
        assert!(win
            .names()
            .write(|b| b.accept("gfx", "executable|graphic builder", false))
            .is_some());
        win.save();
        let again = WindowNames::open(Some(file.clone()));
        assert_eq!(again.names().read(|b, _| b.facts().len()), 1);
        // A damaged file gives an empty book, not a failure.
        std::fs::write(&file, b"{ not json").unwrap();
        assert_eq!(WindowNames::open(Some(file.clone())).names().read(|b, _| b.facts().len()), 0);
        // A missing file and no file at all too.
        std::fs::remove_dir_all(&dir).unwrap();
        assert_eq!(WindowNames::open(Some(file)).names().read(|b, _| b.groups().len()), 0);
        WindowNames::open(None).save();
    }

    #[test]
    fn webview_arguments_are_validated() {
        assert!(phrase("x", "  ").is_err());
        assert!(phrase("x", &"a".repeat(MAX_TERM_CHARS + 1)).is_err());
        assert!(phrase("x", "a\u{0}b").is_err());
        assert_eq!(phrase("x", "  gfx ").unwrap(), "gfx");
        let win = WindowNames::open(None);
        with_group(&win, "graphicbuilder.exe", 9);
        assert!(known_group(win.names(), "executable|graphic builder").is_ok());
        for bad in ["", "nope", &"g".repeat(MAX_ID_CHARS + 1)] {
            assert_eq!(known_group(win.names(), bad).unwrap_err().code, ErrorCode::Invalid);
        }
    }

    #[test]
    fn accept_reject_undo_and_remove_alias_change_the_book_and_save_it() {
        let dir = temp("feedback");
        let file = dir.join(BOOK_FILE);
        let win = WindowNames::open(Some(file.clone()));
        with_group(&win, "graphicbuilder.exe", 9);
        let mut state = crate::windows::tests::window_state("names-fb", debrief_core::PluginRegistry::new());
        state.names = win.clone();
        let req = |text: &str, group: &str| NameFeedback {
            text: text.into(),
            group: group.into(),
            global: false,
        };
        let id = "executable|graphic builder";
        assert!(feedback(&state, &req("gfx", id), NameBook::accept).unwrap().fact.is_some());
        assert!(feedback(&state, &req("paint", id), NameBook::reject).unwrap().fact.is_some());
        assert_eq!(win.names().read(|b, _| b.facts().len()), 2);
        assert_eq!(WindowNames::open(Some(file)).names().read(|b, _| b.facts().len()), 2, "saved");
        assert_eq!(
            feedback(&state, &req("", id), NameBook::accept).unwrap_err().code,
            ErrorCode::Invalid
        );
        assert_eq!(
            feedback(&state, &req("gfx", "nope"), NameBook::accept).unwrap_err().code,
            ErrorCode::Invalid
        );
        assert!(win.names().write(NameBook::undo).is_some());
        assert_eq!(win.names().read(|b, _| b.facts().len()), 1);
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_alias_job_stores_valid_answers_saves_and_reports() {
        let dir = temp("job");
        let file = dir.join(BOOK_FILE);
        let win = WindowNames::open(Some(file.clone()));
        with_group(&win, "graphicbuilder.exe", 50);
        let cfg = AliasJobConfig {
            pause: Duration::ZERO,
            ..AliasJobConfig::default()
        };
        let ctx = JobCtx::detached();
        let good = Fixed(r#"{"items":[{"id":1,"name":"Graphic Builder","aliases":["image editor"]}]}"#);
        match run_alias_job(&win, Some(&good), None, &ctx, &cfg) {
            AliasEnd::Done(m) => assert!(m.starts_with("1 names learned"), "{m}"),
            other => panic!("{other:?}"),
        }
        assert_eq!(WindowNames::open(Some(file)).names().read(|b, _| b.ai_len()), 1, "saved");
        // No model: skipped, not failed.
        assert_eq!(
            run_alias_job(&win, None, None, &ctx, &cfg),
            AliasEnd::Done("Skipped: no model".into())
        );
        // A model that cannot answer fails the job with its message.
        struct Broken;
        impl AliasModel for Broken {
            fn version(&self) -> String {
                "broken".into()
            }
            fn complete(&self, _: &str) -> Result<String, String> {
                Err("the model stopped".into())
            }
        }
        let other = WindowNames::open(None);
        with_group(&other, "trayagent.exe", 50);
        assert_eq!(
            run_alias_job(&other, Some(&Broken), None, &ctx, &cfg),
            AliasEnd::Failed("the model stopped".into())
        );
        // The Settings switch off: nothing is asked.
        other.apply_settings(false);
        assert_eq!(
            run_alias_job(&other, Some(&Broken), None, &ctx, &cfg),
            AliasEnd::Done("Skipped: off".into())
        );
        std::fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn the_job_lets_the_assistant_go_first() {
        use crate::jobs::{Done, JobKind, JobSpec, SystemClock};
        let jobs = Jobs::new(Arc::new(SystemClock));
        let release = Arc::new(AtomicBool::new(false));
        let r = release.clone();
        let handle = jobs
            .submit::<(), String>(JobSpec::new(JobKind::Ai, "chat"), crate::jobs::Controls::default(), move |_| {
                while !r.load(Ordering::SeqCst) {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Ok(Done::new(()))
            })
            .unwrap();
        let inner = Fixed("{}");
        let cancel = Arc::new(AtomicBool::new(false));
        let polite = Polite {
            inner: &inner,
            jobs: Some(jobs.clone()),
            cancel: cancel.clone(),
            poll: Duration::from_millis(5),
            max_wait: Duration::from_millis(80),
        };
        crate::jobs::wait_until("the chat job runs", || !jobs.find_active(|j| j.kind == JobKind::Ai).is_empty());
        assert!(polite.complete("x").unwrap_err().contains("stayed busy"), "waits while a chat runs");
        cancel.store(true, Ordering::SeqCst);
        assert_eq!(polite.complete("x").unwrap_err(), "cancelled");
        cancel.store(false, Ordering::SeqCst);
        release.store(true, Ordering::SeqCst);
        handle.wait().unwrap().unwrap();
        crate::jobs::wait_until("the chat job ends", || jobs.find_active(|j| j.kind == JobKind::Ai).is_empty());
        assert_eq!(polite.complete("x").unwrap(), "{}", "answers once the assistant is idle");
        jobs.shutdown();
    }
}
