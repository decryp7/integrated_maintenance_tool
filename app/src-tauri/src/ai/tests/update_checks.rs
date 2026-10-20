//! The AI update checks: policies (Automatic waits for a chat, Ask announces once, Later, Skip), quiet
//! failures with backoff, the record across restarts, model updates that keep the old file until the new one
//! verifies, and the runtime update flow through IPC (message, job, retiring the replaced build).

use super::*;
use crate::ai::update_commands;
use crate::ai::updates::{AiUpdates, AiUpdatesStatus, UpdateEffects};
use imt_ai::net::{NetError, NetKind};
use imt_ai::setup::SetupStage;
use imt_ai::updates::{UpdateCandidate, UpdateKind};
use std::sync::atomic::AtomicI64;

const OLD_TAG: &str = "b5";
const OLD_MODEL: &str = "Old-4B-Q4_K_M.gguf";

/// A complete build of an older release, as an earlier IMT installed it.
fn plant_build(tmp: &TempDir, tag: &str, id: &str) -> PathBuf {
    let dir = tmp.0.join("runtimes").join(tag).join(id);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(SERVER_FILE), b"old server").unwrap();
    std::fs::write(dir.join(imt_ai::runtimes::store::MARKER), b"{}").unwrap();
    dir
}

/// What the service asked for, instead of jobs and messages.
#[derive(Default)]
struct Recorder {
    installs: Mutex<Vec<String>>,
    announced: Mutex<Vec<String>>,
    refuse: AtomicBool,
}

struct Effects(Arc<Recorder>);
impl UpdateEffects for Effects {
    fn install(&self, u: &UpdateCandidate) -> super::super::error::AiResult<()> {
        if self.0.refuse.load(Ordering::SeqCst) {
            return Err(super::super::error::AiCmdError::busy("another install runs"));
        }
        self.0.installs.lock().unwrap().push(u.key.clone());
        Ok(())
    }
    fn announce(&self, u: &UpdateCandidate) {
        self.0.announced.lock().unwrap().push(u.key.clone());
    }
    fn changed(&self, _: &AiUpdatesStatus) {}
}

struct Service {
    updates: AiUpdates,
    rec: Arc<Recorder>,
    busy: Arc<AtomicBool>,
    now: Arc<AtomicI64>,
}

fn service(core: Arc<AiCore>, record_dir: &Path) -> Service {
    let rec = Arc::new(Recorder::default());
    let busy = Arc::new(AtomicBool::new(false));
    let now = Arc::new(AtomicI64::new(1_000_000));
    let (b, n) = (busy.clone(), now.clone());
    let updates = AiUpdates::new(
        core,
        Some(record_dir.to_path_buf()),
        Arc::new(move || b.load(Ordering::SeqCst)),
        Box::new(move || n.load(Ordering::SeqCst)),
        Box::new(Effects(rec.clone())),
    );
    Service { updates, rec, busy, now }
}

fn set_policies(core: &AiCore, runtime: &str, model: &str) {
    let mut s = core.settings();
    s.runtime_updates = imt_ai::updates::UpdatePolicy::parse(runtime).unwrap();
    s.model_updates = imt_ai::updates::UpdatePolicy::parse(model).unwrap();
    core.apply_settings(s).unwrap();
}

fn kind(status: &AiUpdatesStatus, k: UpdateKind) -> &crate::ai::updates::KindStatus {
    status.kinds.iter().find(|s| s.kind == k).unwrap()
}

fn runtime_key(id: &str) -> String {
    format!("runtime:{id}@{RT_TAG}")
}

#[test]
fn automatic_installs_wait_for_a_running_chat() {
    let f = Fixture::new("upd-auto");
    plant_build(&f.tmp, OLD_TAG, "cpu");
    let core = f.core();
    set_policies(&core, "automatic", "manual");
    let s = service(core, &f.tmp.0.join("config"));
    s.busy.store(true, Ordering::SeqCst);
    let st = s.updates.check(UpdateKind::Runtime);
    let rt = kind(&st, UpdateKind::Runtime);
    assert_eq!(rt.updates.len(), 1);
    assert!(rt.updates[0].waiting && !rt.updates[0].installing);
    assert_eq!(
        (rt.updates[0].update.from.as_str(), rt.updates[0].update.to.as_str()),
        (OLD_TAG, RT_TAG)
    );
    assert!(s.rec.installs.lock().unwrap().is_empty(), "never during a chat");
    s.updates.tick();
    assert!(s.rec.installs.lock().unwrap().is_empty(), "still chatting");
    // The chat ended: the next beat starts the install (the check itself is not due again).
    s.busy.store(false, Ordering::SeqCst);
    s.updates.tick();
    assert_eq!(*s.rec.installs.lock().unwrap(), [runtime_key("cpu")]);
    assert!(kind(&s.updates.status(), UpdateKind::Runtime).updates[0].installing);
    assert!(s.rec.announced.lock().unwrap().is_empty(), "automatic does not ask");
}

#[test]
fn an_install_that_cannot_start_waits_for_the_next_beat() {
    let f = Fixture::new("upd-refused");
    plant_build(&f.tmp, OLD_TAG, "cpu");
    let core = f.core();
    set_policies(&core, "automatic", "manual");
    let s = service(core, &f.tmp.0.join("config"));
    s.rec.refuse.store(true, Ordering::SeqCst);
    let st = s.updates.check(UpdateKind::Runtime);
    assert!(kind(&st, UpdateKind::Runtime).updates[0].waiting);
    s.rec.refuse.store(false, Ordering::SeqCst);
    s.updates.tick();
    assert_eq!(*s.rec.installs.lock().unwrap(), [runtime_key("cpu")]);
}

#[test]
fn ask_announces_once_later_asks_again_and_skip_is_remembered() {
    let f = Fixture::new("upd-ask");
    plant_build(&f.tmp, OLD_TAG, "cpu");
    plant_build(&f.tmp, OLD_TAG, "vulkan");
    let core = f.core();
    set_policies(&core, "ask", "manual");
    let config = f.tmp.0.join("config");
    let s = service(core.clone(), &config);
    s.busy.store(true, Ordering::SeqCst);
    s.updates.check(UpdateKind::Runtime);
    let mut announced = s.rec.announced.lock().unwrap().clone();
    announced.sort();
    assert_eq!(
        announced,
        [runtime_key("cpu"), runtime_key("vulkan")],
        "asking never waits for a chat"
    );
    s.updates.check(UpdateKind::Runtime);
    assert_eq!(s.rec.announced.lock().unwrap().len(), 2, "announced once per version");
    assert!(s.rec.installs.lock().unwrap().is_empty());

    s.updates.later(&runtime_key("cpu")).unwrap();
    s.updates.skip(&runtime_key("vulkan")).unwrap();
    let st = s.updates.check(UpdateKind::Runtime);
    assert_eq!(
        s.rec.announced.lock().unwrap().last().unwrap(),
        &runtime_key("cpu"),
        "Later: asked again"
    );
    assert_eq!(s.rec.announced.lock().unwrap().len(), 3);
    let views = &kind(&st, UpdateKind::Runtime).updates;
    assert!(
        views.iter().any(|v| v.update.id == "vulkan" && v.skipped),
        "skipped ones stay listed in Settings"
    );

    // A restart remembers both the skip and what was announced.
    let again = service(core.clone(), &config);
    again.updates.check(UpdateKind::Runtime);
    assert!(again.rec.announced.lock().unwrap().is_empty());
    // Skipped updates are not installed automatically either, but "Update" still installs them.
    set_policies(&core, "automatic", "manual");
    again.updates.check(UpdateKind::Runtime);
    assert_eq!(*again.rec.installs.lock().unwrap(), [runtime_key("cpu")]);
    assert_eq!(
        code(serde_json::to_value(again.updates.apply(&runtime_key("vulkan")).unwrap_err()).unwrap()),
        "busy",
        "one install at a time"
    );
    let unknown = again.updates.apply("runtime:rocm@b7").unwrap_err();
    assert_eq!(code(serde_json::to_value(unknown).unwrap()), "notFound");
    assert_eq!(
        code(serde_json::to_value(again.updates.skip("bad key!").unwrap_err()).unwrap()),
        "invalid"
    );
    assert_eq!(
        code(serde_json::to_value(again.updates.later("plugin:x@1").unwrap_err()).unwrap()),
        "invalid"
    );
}

#[test]
fn manual_only_checks_when_asked() {
    let f = Fixture::new("upd-manual");
    plant_build(&f.tmp, OLD_TAG, "cpu");
    let core = f.core();
    set_policies(&core, "manual", "manual");
    let s = service(core, &f.tmp.0.join("config"));
    s.updates.tick();
    let st = s.updates.status();
    assert!(kind(&st, UpdateKind::Runtime).last_check.is_none(), "no check by itself");
    assert!(kind(&st, UpdateKind::Runtime).next_check.is_none());
    let st = s.updates.check(UpdateKind::Runtime);
    assert_eq!(kind(&st, UpdateKind::Runtime).updates.len(), 1, "Check now lists it");
    assert!(s.rec.announced.lock().unwrap().is_empty() && s.rec.installs.lock().unwrap().is_empty());
}

/// A catalog whose test model replaces [`OLD_MODEL`], which is installed and active.
fn model_update_core(f: &Fixture, deps: AiDeps) -> Arc<AiCore> {
    let dir = ai_dir(&f.tmp);
    let mut catalog: Value = serde_json::from_str(&std::fs::read_to_string(dir.join("local-models.json")).unwrap()).unwrap();
    catalog["models"][0]["supersedes"] = json!([OLD_MODEL]);
    std::fs::write(dir.join("local-models.json"), catalog.to_string()).unwrap();
    let models = f.tmp.0.join("models");
    std::fs::create_dir_all(&models).unwrap();
    std::fs::write(models.join(OLD_MODEL), model_bytes()).unwrap();
    let core = Arc::new(AiCore::new(Some(dir), models, false, deps));
    core.update_settings(|s| s.active_model = Some(OLD_MODEL.into())).unwrap();
    core
}

/// A model host that cannot be reached through the network's TLS interception.
struct Intercepted;
impl ModelSource for Intercepted {
    fn describe(&self, _: &str, _: &str) -> imt_ai::Result<RemoteFile> {
        Err(AiError::Connection(NetError::new(
            NetKind::Tls,
            "huggingface.co",
            "invalid peer certificate: UnknownIssuer",
        )))
    }
    fn fetch(&self, _: &RemoteFile, _: u64, _: &mut dyn FnMut(&[u8]) -> imt_ai::Result<()>, _: &AtomicBool) -> imt_ai::Result<()> {
        unreachable!()
    }
}

#[test]
fn offline_checks_are_quiet_and_back_off() {
    let f = Fixture::new("upd-offline");
    let mut deps = f.deps();
    deps.source = Arc::new(|_| Ok(Box::new(Intercepted) as Box<dyn ModelSource>));
    let core = model_update_core(&f, deps);
    set_policies(&core, "manual", "ask");
    let s = service(core, &f.tmp.0.join("config"));
    let start = s.now.load(Ordering::SeqCst);
    let st = s.updates.check(UpdateKind::Model);
    let m = kind(&st, UpdateKind::Model);
    assert!(m.updates.is_empty() && !m.checking);
    assert!(m.last_error.as_deref().unwrap().contains("Zscaler"), "{:?}", m.last_error);
    assert!(m.manual_suggested, "a TLS failure offers the manual download");
    assert_eq!(m.last_check, Some(start * 1000));
    assert_eq!(m.next_check, Some((start + 300) * 1000), "first retry after five minutes");
    assert!(s.rec.announced.lock().unwrap().is_empty(), "quiet: no message");

    // Not retried before the backoff, then retried and backing off further.
    s.now.store(start + 200, Ordering::SeqCst);
    s.updates.tick();
    assert_eq!(kind(&s.updates.status(), UpdateKind::Model).last_check, Some(start * 1000));
    s.now.store(start + 300, Ordering::SeqCst);
    s.updates.tick();
    let m = kind(&s.updates.status(), UpdateKind::Model).clone();
    assert_eq!(m.last_check, Some((start + 300) * 1000));
    assert_eq!(m.next_check, Some((start + 900) * 1000));
}

#[test]
fn a_model_update_replaces_the_old_file_only_after_it_verifies() {
    let f = Fixture::new("upd-model");
    let core = model_update_core(&f, f.deps());
    set_policies(&core, "manual", "ask");
    let s = service(core.clone(), &f.tmp.0.join("config"));
    let st = s.updates.check(UpdateKind::Model);
    let m = kind(&st, UpdateKind::Model);
    assert!(m.last_error.is_none(), "{:?}", m.last_error);
    assert_eq!(m.updates.len(), 1);
    let u = m.updates[0].update.clone();
    assert_eq!((u.from.as_str(), u.to.as_str(), u.id.as_str()), (OLD_MODEL, TEST_FILE, TEST_MODEL));
    assert_eq!(*s.rec.announced.lock().unwrap(), std::slice::from_ref(&u.key));

    // A chat runs: the new file is downloaded and verified, but the switch waits for the chat to end.
    let busy = Arc::new(AtomicBool::new(true));
    let b = busy.clone();
    let p = Progress::new();
    let models = f.tmp.0.join("models");
    std::thread::scope(|scope| {
        let job = scope.spawn(|| core.install_model_update(&u, &p, &mut |_: SetupStage| {}, &move || b.load(Ordering::SeqCst)));
        wait_until("the new file", || models.join(TEST_FILE).is_file());
        std::thread::sleep(Duration::from_millis(100));
        assert_eq!(
            core.settings().active_model.as_deref(),
            Some(OLD_MODEL),
            "never switched during a chat"
        );
        assert!(models.join(OLD_MODEL).is_file());
        busy.store(false, Ordering::SeqCst);
        job.join().unwrap().unwrap();
    });
    assert_eq!(core.settings().active_model.as_deref(), Some(TEST_FILE));
    assert!(!models.join(OLD_MODEL).exists(), "the replaced file goes");
    s.updates.finished(&u, &Ok(()));
    assert!(kind(&s.updates.status(), UpdateKind::Model).updates.is_empty());
    assert!(s.updates.check(UpdateKind::Model).kinds.iter().all(|k| k.updates.is_empty()));
}

#[test]
fn a_model_update_that_fails_verification_keeps_the_old_model() {
    let f = Fixture::new("upd-corrupt");
    let mut deps = f.deps();
    let good = model_bytes();
    let remote = RemoteFile {
        repo: "test/Test-4B-GGUF".into(),
        file: TEST_FILE.into(),
        size: good.len() as u64,
        sha256: sha256_of(&good, &f.tmp.0),
        license: None,
        gated: false,
    };
    let mut bad = good.clone();
    let last = bad.len() - 1;
    bad[last] ^= 0xff;
    deps.source = Arc::new(move |_| {
        Ok(Box::new(FakeHost {
            remote: remote.clone(),
            bytes: bad.clone(),
            delay: Duration::ZERO,
        }) as Box<dyn ModelSource>)
    });
    let core = model_update_core(&f, deps);
    set_policies(&core, "manual", "automatic");
    let s = service(core.clone(), &f.tmp.0.join("config"));
    s.updates.check(UpdateKind::Model);
    let u = kind(&s.updates.status(), UpdateKind::Model).updates[0].update.clone();
    assert_eq!(
        *s.rec.installs.lock().unwrap(),
        std::slice::from_ref(&u.key),
        "automatic, no chat: installs"
    );
    let r = core.install_model_update(&u, &Progress::new(), &mut |_: SetupStage| {}, &|| false);
    let e = r.clone().unwrap_err();
    assert_eq!(code(serde_json::to_value(&e).unwrap()), "integrity", "{e:?}");
    let models = f.tmp.0.join("models");
    assert!(models.join(OLD_MODEL).is_file() && !models.join(TEST_FILE).exists());
    assert_eq!(core.settings().active_model.as_deref(), Some(OLD_MODEL));
    s.updates.finished(&u, &r);
    let m = kind(&s.updates.status(), UpdateKind::Model).clone();
    assert!(
        m.last_error.as_deref().unwrap().contains("installed version is kept"),
        "{:?}",
        m.last_error
    );
    assert!(!m.updates[0].installing, "offered again");
    assert!(m.next_check.is_some());
}

#[test]
fn a_runtime_update_keeps_the_old_build_working_until_the_new_one_verifies() {
    let f = Fixture::new("upd-runtime-fail");
    plant_build(&f.tmp, OLD_TAG, "cuda-13");
    let core = f.core();
    // Before the update, the worker runs the older release's builds.
    let dir = |core: &AiCore| {
        core.deps
            .runtimes
            .worker_env(&core.settings())
            .into_iter()
            .find(|(k, _)| k == "IMT_LLAMA_DIR")
            .unwrap()
            .1
    };
    assert!(dir(&core).ends_with(OLD_TAG), "{}", dir(&core));
    let u = core.update_candidates(UpdateKind::Runtime).unwrap();
    assert_eq!(u.len(), 1);
    // The pinned cuda-13 archive of the test catalog has no llama-server: the install fails its check.
    let p = core.begin_runtime_install("cuda-13").unwrap();
    let r = core.install_runtime("cuda-13", &[], &p, &mut |_| {});
    core.end_runtime_install();
    assert_eq!(code(serde_json::to_value(r.unwrap_err()).unwrap()), "integrity");
    assert!(f.tmp.0.join("runtimes").join(OLD_TAG).join("cuda-13").join(SERVER_FILE).is_file());
    assert!(dir(&core).ends_with(OLD_TAG), "still the old build");
    assert!(core.retire_runtime(RT_TAG, "cuda-13"), "the pinned release is never retired");
}

#[test]
fn a_runtime_update_through_ipc_asks_installs_and_retires_the_old_build() {
    let f = Fixture::new("upd-ipc");
    let old = plant_build(&f.tmp, OLD_TAG, "cpu");
    let core = f.core();
    let app = f.app(core.clone());
    update_commands::manage(app.handle(), core.clone(), Box::new(|| 1_000_000));
    let main = window(&app, "main");
    let events = runtime_events(&app);

    let st = ipc(&main, "ai_updates_status", json!({})).unwrap();
    assert_eq!(st["intervalHours"], 24);
    assert_eq!(st["kinds"][0]["policy"], "ask");
    let st = ipc(&main, "ai_updates_check", json!({"kind": "runtime"})).unwrap();
    let rt = &st["kinds"][0];
    assert_eq!(
        (rt["kind"].as_str(), rt["updates"][0]["key"].as_str()),
        (Some("runtime"), Some(runtime_key("cpu").as_str()))
    );
    assert!(rt["lastCheck"].as_i64().is_some() && rt["lastError"].is_null());

    // Ask: one message with Update, Later and Skip this version.
    let notices = ipc(&main, "notifications_list", json!({})).unwrap();
    let n = notices["notices"]
        .as_array()
        .unwrap()
        .iter()
        .find(|n| n["primary"]["type"] == "updateAi")
        .cloned()
        .unwrap();
    assert_eq!(n["primary"]["key"], runtime_key("cpu"));
    assert_eq!(
        (n["secondary"]["type"].as_str(), n["tertiary"]["type"].as_str()),
        (Some("laterAiUpdate"), Some("skipAiUpdate"))
    );
    assert!(n["title"].as_str().unwrap().contains("CPU") && n["body"].as_str().unwrap().contains(OLD_TAG));

    for (cmd, args) in [
        ("ai_updates_check", json!({"kind": "plugins"})),
        ("ai_updates_apply", json!({"key": ""})),
        ("ai_updates_skip", json!({"key": "x".repeat(200)})),
    ] {
        assert_eq!(code(ipc(&main, cmd, args).unwrap_err()), "invalid", "{cmd}");
    }

    // Update: a download job installs the pinned build; the old one is retired (no model is loaded).
    ipc(&main, "ai_updates_apply", json!({"key": runtime_key("cpu")})).unwrap();
    assert_eq!(install_end(&events, "cpu")["stage"], "ready");
    wait_until("the old build to be retired", || !old.exists());
    let st = ipc(&main, "ai_updates_status", json!({})).unwrap();
    assert!(st["kinds"][0]["updates"].as_array().unwrap().is_empty(), "{st}");
    assert!(f.tmp.0.join("runtimes").join(RT_TAG).join("cpu").join(SERVER_FILE).is_file());
    let later = ipc(&main, "ai_updates_later", json!({"key": runtime_key("cpu")})).unwrap();
    assert_eq!(later["kinds"][1]["kind"], "model");
    assert!(app.try_state::<update_commands::AiUpdatesState>().is_some());
}

#[test]
fn update_messages_name_the_versions_and_sizes() {
    let u = |kind, bytes| UpdateCandidate {
        kind,
        key: "model:m@1".into(),
        id: "m".into(),
        name: "Model".into(),
        from: "a.gguf".into(),
        to: "b.gguf".into(),
        download_bytes: bytes,
    };
    let n = update_commands::update_notice(&u(UpdateKind::Model, 5_400_000_000));
    assert!(
        n.title.contains("model update") && n.body.contains("b.gguf replaces a.gguf (5.4 GB"),
        "{}",
        n.body
    );
    let n = update_commands::update_notice(&u(UpdateKind::Runtime, 20_000));
    assert!(n.title.contains("runtime update") && n.body.contains("(1 MB"), "{}", n.body);
    assert_eq!(n.dedupe_key.as_deref(), Some("ai-update:runtime:m"));
}

#[test]
fn the_record_tolerates_a_damaged_file() {
    let f = Fixture::new("upd-record");
    let config = f.tmp.0.join("config");
    std::fs::create_dir_all(&config).unwrap();
    std::fs::write(config.join(crate::ai::updates::RECORD_FILE), "{ damaged").unwrap();
    let s = service(f.core(), &config);
    let st = s.updates.status();
    assert!(st.kinds.iter().all(|k| k.last_check.is_none() && k.updates.is_empty()));
    assert_eq!(st.kinds.len(), 2);
}
