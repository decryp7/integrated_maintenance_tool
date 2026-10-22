use super::*;
use crate::storage::model::Kind;
use crate::storage::walk::tests::{hold, link_dir, release, write};
use crate::test_support::TempDir;
use std::sync::Mutex;
use std::time::Duration;

const TAG: &str = "b200";
/// The file name of an autosaved session (`lifecycle`).
const SESSION_FILE: &str = "last-session.imtproj";

/// Sets a file's modification time.
fn touch(path: &Path, t: SystemTime) {
    std::fs::OpenOptions::new().write(true).open(path).unwrap().set_modified(t).unwrap();
}

/// A file written before (`old`) or during this run.
fn file_at(path: &Path, bytes: usize, t: SystemTime) {
    write(path, bytes);
    touch(path, t);
}

struct Clock {
    start: SystemTime,
    old: SystemTime,
    new: SystemTime,
}

fn clock() -> Clock {
    let start = SystemTime::now();
    Clock {
        start,
        old: start - Duration::from_secs(3600),
        new: start + Duration::from_secs(60),
    }
}

#[derive(Default)]
struct FakeAi {
    engine: Option<String>,
    setup: Option<String>,
    custom: Option<PathBuf>,
    models_dir: PathBuf,
    runtimes: PathBuf,
    models: Vec<(String, bool)>,
    calls: Mutex<Vec<String>>,
}

impl AiPort for FakeAi {
    fn engine_busy(&self) -> Option<String> {
        self.engine.clone()
    }
    fn setup_busy(&self) -> Option<String> {
        self.setup.clone()
    }
    fn custom_server(&self) -> Option<PathBuf> {
        self.custom.clone()
    }
    fn models_dir(&self) -> PathBuf {
        self.models_dir.clone()
    }
    fn models(&self) -> Vec<(String, bool)> {
        self.models.clone()
    }
    fn delete_runtime(&self, tag: &str, id: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!("runtime {tag}/{id}"));
        std::fs::remove_dir_all(self.runtimes.join(tag).join(id)).map_err(|e| e.to_string())
    }
    fn delete_model(&self, file_name: &str) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!("model {file_name}"));
        if file_name == "locked.gguf" {
            return Err("the model file is in use".into());
        }
        std::fs::remove_file(self.models_dir.join(file_name)).map_err(|e| e.to_string())
    }
}

fn folders<'a>(d: &Path, ai: Option<&'a dyn AiPort>, start: Option<SystemTime>) -> Folders<'a> {
    Folders {
        bundles: Some(d.join("tmp").join("imt-bundles")),
        webview: Some(d.join("local")),
        data: Some(d.join("data")),
        log: Some(d.join("logs")),
        active_log: Some(d.join("logs").join("imt.2026-10-18.log")),
        runtime_tag: TAG.into(),
        session_start: start,
        main_instance: true,
        previous_crash: false,
        ai,
    }
}

fn find(cats: &[Category], id: CategoryId) -> &Category {
    cats.iter().find(|c| c.id == id).unwrap()
}

fn clear_one(f: &Folders<'_>, id: CategoryId) -> CategoryClearResult {
    let cats = categories(f);
    clear(find(&cats, id), f.ai, &AtomicBool::new(false), &mut || {})
}

#[test]
fn every_category_resolves_from_the_app_folders() {
    let d = TempDir::new("storage-plan-folders");
    let ai = FakeAi {
        models_dir: d.0.join("custom-models"),
        ..FakeAi::default()
    };
    let f = folders(&d.0, Some(&ai), Some(SystemTime::now()));
    let cats = categories(&f);
    let ids: Vec<_> = cats.iter().map(|c| c.id).collect();
    assert_eq!(ids, CategoryId::ALL[..8], "stores are listed only once their folder exists");
    let shown = |id| find(&cats, id).shown.clone().unwrap();
    assert_eq!(shown(CategoryId::BundleExtracts), d.0.join("tmp").join("imt-bundles"));
    assert_eq!(shown(CategoryId::WebviewCache), d.0.join("local"));
    assert_eq!(shown(CategoryId::AiRuntimesOutdated), d.0.join("data").join("ai").join("runtimes"));
    assert_eq!(
        shown(CategoryId::AiRuntimes),
        d.0.join("data").join("ai").join("runtimes").join(TAG)
    );
    assert_eq!(shown(CategoryId::AiModels), d.0.join("custom-models"));
    assert_eq!(shown(CategoryId::Logs), d.0.join("logs"));
    assert_eq!(shown(CategoryId::ErrorSnapshots), d.0.join("logs").join("snapshots"));
    assert_eq!(shown(CategoryId::Sessions), d.0.join("data").join("sessions"));
    // Sessions are removed only inside the data folder.
    assert_eq!(find(&cats, CategoryId::Sessions).root, Some(d.0.join("data")));

    std::fs::create_dir_all(d.0.join("data").join("stores")).unwrap();
    let cats = categories(&f);
    let stores = find(&cats, CategoryId::Stores);
    assert!(stores.clearable.is_err(), "stores are listed, not cleared, until ADR 0010 lands");
    assert!(clear(stores, None, &AtomicBool::new(false), &mut || {}).error.is_some());
}

#[test]
fn unresolved_folders_are_listed_but_not_clearable() {
    let f = Folders {
        bundles: None,
        webview: None,
        data: None,
        log: None,
        active_log: None,
        runtime_tag: TAG.into(),
        session_start: None,
        main_instance: true,
        previous_crash: false,
        ai: None,
    };
    let cats = categories(&f);
    assert_eq!(cats.len(), 8);
    for c in &cats {
        assert!(c.shown.is_none() || c.id == CategoryId::AiModels, "{:?}", c.id);
        assert!(c.clearable.is_err(), "{:?}", c.id);
        let row = usage(c, &mut Budget::standard());
        assert!(!row.clearable && row.note.is_some() && !row.exists);
        assert!(clear(c, None, &AtomicBool::new(false), &mut || {}).error.is_some());
    }
}

#[test]
fn the_current_log_file_is_never_removed() {
    let d = TempDir::new("storage-plan-logs");
    let logs = d.0.join("logs");
    write(&logs.join("imt.2026-10-18.log"), 10);
    write(&logs.join("imt.2026-10-17.log"), 20);
    write(&logs.join("imt.2026-10-16.log"), 30);
    write(&logs.join("imt.log"), 5);
    write(&logs.join("other.txt"), 7);
    write(&logs.join("snapshots").join("a.png"), 1);
    let f = folders(&d.0, None, None);
    let cats = categories(&f);
    let row = usage(find(&cats, CategoryId::Logs), &mut Budget::standard());
    assert_eq!((row.bytes, row.files, row.clearable_bytes), (65, 4, 50));
    let r = clear_one(&f, CategoryId::Logs);
    assert_eq!(r.freed_bytes, 50);
    assert!(logs.join("imt.2026-10-18.log").is_file(), "today's log stays");
    assert!(logs.join("imt.log").is_file());
    assert!(logs.join("other.txt").is_file(), "only IMT's logs are listed");
    assert!(!logs.join("imt.2026-10-17.log").exists());
    assert_eq!(r.skipped_count, 2);
    assert_eq!(r.skipped[0].reason, "IMT is writing to it");
}

#[test]
fn error_snapshots_are_cleared_but_the_notice_marker_stays() {
    let d = TempDir::new("storage-plan-snapshots");
    let dir = d.0.join("logs").join("snapshots");
    write(&dir.join("20261017T000000000-aa.png"), 100);
    write(&dir.join("20261017T000000000-aa.json"), 10);
    write(&dir.join("notice-shown"), 1);
    let f = folders(&d.0, None, None);
    let r = clear_one(&f, CategoryId::ErrorSnapshots);
    assert_eq!((r.freed_bytes, r.removed_files), (110, 2));
    assert!(dir.join("notice-shown").is_file());
}

#[test]
fn session_backups_older_than_this_run_are_cleared_and_this_runs_are_kept() {
    let d = TempDir::new("storage-plan-sessions");
    let t = clock();
    let data = d.0.join("data");
    file_at(&data.join("sessions").join("main").join(SESSION_FILE), 10, t.new);
    file_at(&data.join("sessions").join("w-2").join(SESSION_FILE), 20, t.old);
    file_at(&data.join("recovered").join("main").join(SESSION_FILE), 30, t.old);
    write(&data.join("sessions").join("manifest.json"), 1);
    write(&data.join("running.marker"), 1);

    // A crashed previous run keeps what it left for the restore offer.
    let mut f = folders(&d.0, None, Some(t.start));
    f.previous_crash = true;
    let r = clear_one(&f, CategoryId::Sessions);
    assert_eq!(r.freed_bytes, 20);
    assert!(data.join("recovered").join("main").join(SESSION_FILE).is_file());
    assert!(r.skipped.iter().any(|s| s.reason.contains("crash")));

    f.previous_crash = false;
    let r = clear_one(&f, CategoryId::Sessions);
    assert_eq!(r.freed_bytes, 30);
    assert!(
        data.join("sessions").join("main").join(SESSION_FILE).is_file(),
        "this run's session stays"
    );
    assert!(r.skipped.iter().any(|s| s.reason == "saved in this session"));
    assert!(!data.join("sessions").join("w-2").exists());
    assert!(
        data.join("sessions").join("manifest.json").is_file(),
        "only session folders are removed"
    );
    assert!(data.join("running.marker").is_file());
}

#[test]
fn sessions_are_kept_when_the_run_start_is_unknown_or_another_instance_owns_them() {
    let d = TempDir::new("storage-plan-sessions-safe");
    let t = clock();
    let data = d.0.join("data");
    file_at(&data.join("sessions").join("w-2").join(SESSION_FILE), 20, t.old);
    let f = folders(&d.0, None, None);
    let r = clear_one(&f, CategoryId::Sessions);
    assert_eq!((r.freed_bytes, r.skipped_count), (0, 1));
    let mut f = folders(&d.0, None, Some(t.start));
    f.main_instance = false;
    assert!(clear_one(&f, CategoryId::Sessions).error.is_some());
    assert!(data.join("sessions").join("w-2").join(SESSION_FILE).is_file());
}

#[test]
fn bundles_opened_in_this_run_are_kept() {
    let d = TempDir::new("storage-plan-bundles");
    let t = clock();
    let root = d.0.join("tmp").join("imt-bundles");
    file_at(&root.join("old-1").join("project.imtproj"), 50, t.old);
    file_at(&root.join("open-now").join("project.imtproj"), 70, t.new);
    let f = folders(&d.0, None, Some(t.start));
    let cats = categories(&f);
    let row = usage(find(&cats, CategoryId::BundleExtracts), &mut Budget::standard());
    assert_eq!((row.bytes, row.clearable_bytes, row.kind), (120, 50, Kind::Cache));
    let r = clear_one(&f, CategoryId::BundleExtracts);
    assert_eq!(r.freed_bytes, 50);
    assert!(!root.join("old-1").exists());
    assert!(root.join("open-now").join("project.imtproj").is_file());
    assert_eq!(r.skipped[0].reason, "opened in this session");
}

#[test]
fn only_the_webview_caches_are_cleared_never_its_storage() {
    let d = TempDir::new("storage-plan-webview");
    let local = d.0.join("local");
    let caches = webview_cache_dirs(&local);
    for c in &caches {
        write(&c.join("data_0"), 8);
    }
    let keep = local.join("EBWebView").join("Default").join("Local Storage").join("leveldb.ldb");
    write(&keep, 9);
    let f = folders(&d.0, None, None);
    let r = clear_one(&f, CategoryId::WebviewCache);
    if cfg!(target_os = "macos") {
        assert!(r.error.is_some());
        return;
    }
    assert_eq!(r.freed_bytes, 8 * caches.len() as u64);
    assert!(caches.iter().all(|c| !c.exists()));
    assert!(keep.is_file(), "local storage holds the UI's preferences");
}

#[test]
fn files_in_use_are_skipped_and_the_rest_is_cleared() {
    let d = TempDir::new("storage-plan-in-use");
    let local = d.0.join("local");
    let Some(cache) = webview_cache_dirs(&local).into_iter().next() else {
        return;
    };
    let busy = cache.join("index").join("busy");
    write(&busy, 3);
    write(&cache.join("free"), 5);
    let Some(_hold) = hold(&busy) else { return };
    let r = clear_one(&folders(&d.0, None, None), CategoryId::WebviewCache);
    release(&busy);
    assert!(busy.is_file());
    assert_eq!(r.freed_bytes, 5);
    assert!(r.skipped_count >= 1 && r.skipped[0].reason.contains("in use"), "{r:?}");
    assert!(r.error.is_none());
}

#[test]
fn outdated_runtimes_are_other_releases_only() {
    let d = TempDir::new("storage-plan-outdated");
    let rt = d.0.join("data").join("ai").join("runtimes");
    write(&rt.join("b100").join("cpu").join("llama-server"), 11);
    write(&rt.join("b150").join("vulkan").join("llama-server"), 13);
    write(&rt.join(TAG).join("cpu").join("llama-server"), 17);
    write(&rt.join("downloads").join("x.zip.part"), 19);
    write(&rt.join("NOT-A-TAG").join("f"), 1);
    let ai = FakeAi {
        custom: Some(rt.join("b150").join("vulkan").join("llama-server")),
        ..FakeAi::default()
    };
    let f = folders(&d.0, Some(&ai), None);
    let r = clear_one(&f, CategoryId::AiRuntimesOutdated);
    assert_eq!(r.freed_bytes, 11);
    assert!(!rt.join("b100").exists());
    assert!(rt.join("b150").exists(), "the custom llama-server the settings name stays");
    assert!(
        rt.join(TAG).join("cpu").join("llama-server").is_file(),
        "the pinned release is not outdated"
    );
    assert!(rt.join("downloads").join("x.zip.part").is_file());
    assert!(rt.join("NOT-A-TAG").exists());
}

#[test]
fn runtime_builds_in_use_are_skipped_and_idle_ones_go_through_the_engine() {
    let d = TempDir::new("storage-plan-runtimes");
    let rt = d.0.join("data").join("ai").join("runtimes");
    write(&rt.join(TAG).join("cpu").join("llama-server"), 11);
    write(&rt.join(TAG).join("cuda-12").join("llama-server"), 13);
    write(&rt.join(TAG).join("manifest.json"), 1);
    let busy = FakeAi {
        engine: Some("in use by the AI engine".into()),
        runtimes: rt.clone(),
        ..FakeAi::default()
    };
    let r = clear_one(&folders(&d.0, Some(&busy), None), CategoryId::AiRuntimes);
    assert_eq!((r.freed_bytes, r.skipped_count), (0, 2));
    assert!(busy.calls.lock().unwrap().is_empty(), "nothing is asked of a busy engine");
    assert!(rt.join(TAG).join("cpu").is_dir());

    let idle = FakeAi {
        runtimes: rt.clone(),
        ..FakeAi::default()
    };
    let r = clear_one(&folders(&d.0, Some(&idle), None), CategoryId::AiRuntimes);
    assert_eq!((r.freed_bytes, r.removed_files), (24, 2));
    assert_eq!(
        *idle.calls.lock().unwrap(),
        [format!("runtime {TAG}/cpu"), format!("runtime {TAG}/cuda-12")]
    );
    assert!(rt.join(TAG).join("manifest.json").is_file());

    // Without the AI engine its files are not touched.
    let cats = categories(&folders(&d.0, None, None));
    assert!(find(&cats, CategoryId::AiRuntimes).clearable.is_err());
    assert!(find(&cats, CategoryId::AiModels).clearable.is_err());
}

#[test]
fn the_active_model_is_kept_and_engine_refusals_are_reported() {
    let d = TempDir::new("storage-plan-models");
    let dir = d.0.join("models");
    write(&dir.join("active.gguf"), 100);
    write(&dir.join("spare.gguf"), 200);
    write(&dir.join("locked.gguf"), 300);
    let ai = FakeAi {
        models_dir: dir.clone(),
        models: vec![
            ("active.gguf".into(), true),
            ("spare.gguf".into(), false),
            ("locked.gguf".into(), false),
        ],
        ..FakeAi::default()
    };
    let f = folders(&d.0, Some(&ai), None);
    let cats = categories(&f);
    let row = usage(find(&cats, CategoryId::AiModels), &mut Budget::standard());
    assert_eq!((row.bytes, row.clearable_bytes, row.kind), (600, 500, Kind::Data));
    let r = clear_one(&f, CategoryId::AiModels);
    assert_eq!(r.freed_bytes, 200);
    assert!(dir.join("active.gguf").is_file());
    assert!(!dir.join("spare.gguf").exists());
    let reasons: Vec<_> = r.skipped.iter().map(|s| s.reason.as_str()).collect();
    assert!(reasons[0].starts_with("the active model"));
    assert_eq!(reasons[1], "the model file is in use");

    let setting_up = FakeAi {
        setup: Some("a model is being set up".into()),
        models_dir: dir.clone(),
        models: vec![("locked.gguf".into(), false)],
        ..FakeAi::default()
    };
    let r = clear_one(&folders(&d.0, Some(&setting_up), None), CategoryId::AiModels);
    assert_eq!((r.freed_bytes, r.skipped_count), (0, 1));
    assert!(setting_up.calls.lock().unwrap().is_empty());
}

#[test]
fn clearing_refuses_anything_outside_the_category_root() {
    let d = TempDir::new("storage-plan-refuse");
    let root = d.0.join("root");
    write(&root.join("ok.bin"), 1);
    let victim = d.0.join("victim.bin");
    write(&victim, 1);
    let c = Category {
        id: CategoryId::BundleExtracts,
        shown: Some(root.clone()),
        root: Some(root.clone()),
        items: vec![
            Item::path(victim.clone()),
            Item::path(root.join("..").join("victim.bin")),
            Item::path(root.join("ok.bin")),
        ],
        clearable: Ok(()),
        note: None,
    };
    let r = clear(&c, None, &AtomicBool::new(false), &mut || {});
    assert!(victim.is_file());
    assert_eq!(r.skipped_count, 2);
    assert!(r.skipped.iter().all(|s| s.reason.contains("outside")));
    assert_eq!(r.removed_files, 1);
}

#[test]
fn a_link_inside_a_category_is_removed_without_touching_its_target() {
    let d = TempDir::new("storage-plan-link");
    let t = clock();
    let outside = d.0.join("precious");
    write(&outside.join("keep.bin"), 9);
    let root = d.0.join("tmp").join("imt-bundles");
    std::fs::create_dir_all(&root).unwrap();
    if !link_dir(&outside, &root.join("evil")) {
        return;
    }
    let f = folders(&d.0, None, Some(t.new + Duration::from_secs(3600)));
    let cats = categories(&f);
    let row = usage(find(&cats, CategoryId::BundleExtracts), &mut Budget::standard());
    assert_eq!(row.bytes, 0, "the link is not followed when sizing");
    let r = clear_one(&f, CategoryId::BundleExtracts);
    assert!(outside.join("keep.bin").is_file());
    assert_eq!(r.freed_bytes, 0);
}

#[test]
fn a_missing_folder_has_nothing_to_clear_and_progress_steps_per_item() {
    let d = TempDir::new("storage-plan-missing");
    let f = folders(&d.0, None, None);
    let r = clear_one(&f, CategoryId::Logs);
    assert_eq!((r.freed_bytes, r.skipped_count, r.error.clone()), (0, 0, None));

    write(&d.0.join("logs").join("imt.2026-01-01.log"), 1);
    write(&d.0.join("logs").join("imt.2026-01-02.log"), 1);
    let cats = categories(&f);
    let mut steps = 0;
    clear(find(&cats, CategoryId::Logs), None, &AtomicBool::new(false), &mut || steps += 1);
    assert_eq!(steps, 2);
}

#[test]
fn usage_is_capped_by_the_budget() {
    let d = TempDir::new("storage-plan-capped");
    for i in 0..10 {
        write(&d.0.join("logs").join(format!("imt.2026-01-{i:02}.log")), 1);
    }
    let cats = categories(&folders(&d.0, None, None));
    let row = usage(find(&cats, CategoryId::Logs), &mut Budget::new(3, Duration::from_secs(60)));
    assert!(row.capped);
    assert!(row.files < 10);
}

#[test]
fn ids_from_the_ui_are_parsed_and_unknown_ones_refused() {
    let s = |v: &[&str]| v.iter().map(|x| x.to_string()).collect::<Vec<_>>();
    assert_eq!(
        parse_ids(&s(&["logs", "webview-cache", "logs"])).unwrap(),
        [CategoryId::Logs, CategoryId::WebviewCache]
    );
    assert!(parse_ids(&[]).is_err());
    assert!(parse_ids(&s(&["logs", "C:\\Windows"]))
        .unwrap_err()
        .contains("unknown storage category"));
    assert!(parse_ids(&vec!["logs".to_string(); 100]).is_err());
}
