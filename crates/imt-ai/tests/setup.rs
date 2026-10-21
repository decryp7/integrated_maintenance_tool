//! The set-up orchestrator, lazy loading with idle unload, and the installed-model loader, with fakes for
//! every stage (hardware, host, disk, engine).

mod common;
use common::{blob, sha256_hex};
use imt_ai::catalog::{Catalog, CatalogModel};
use imt_ai::gguf;
use imt_ai::hardware::{Gpu, Hardware, HardwareProbe};
use imt_ai::llm::*;
use imt_ai::models::*;
use imt_ai::runtime::*;
use imt_ai::settings::{AiSettings, GpuOffload};
use imt_ai::setup::*;
use imt_ai::testing::gguf::{projector_like, qwen_like};
use imt_ai::AiError;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

struct Probe(Hardware);
impl HardwareProbe for Probe {
    fn probe(&self) -> Hardware {
        self.0.clone()
    }
}

fn machine(vram: Option<f64>, ram: f64) -> Hardware {
    Hardware {
        os: "x".into(),
        arch: "x".into(),
        cpu_cores: 8,
        total_ram_gb: ram,
        available_ram_gb: ram * 0.75,
        gpus: vram
            .map(|v| {
                vec![Gpu {
                    name: "GPU".into(),
                    vram_gb: Some(v),
                    unified: false,
                }]
            })
            .unwrap_or_default(),
    }
}

struct Disk(Option<u64>);
impl DiskSpace for Disk {
    fn available_bytes(&self, _: &Path) -> Option<u64> {
        self.0
    }
}

/// Serves one synthetic model file.
struct FakeSource {
    data: Vec<u8>,
    sha: String,
    /// Served for files named like a projector (`mmproj`).
    projector: Vec<u8>,
    projector_sha: String,
    fetches: AtomicUsize,
    describe_fails: bool,
    slow: bool,
}

impl FakeSource {
    fn new() -> FakeSource {
        let data = qwen_like();
        let projector = projector_like(2560);
        FakeSource {
            sha: sha256_hex(&data),
            data,
            projector_sha: sha256_hex(&projector),
            projector,
            fetches: AtomicUsize::new(0),
            describe_fails: false,
            slow: false,
        }
    }
}

impl ModelSource for FakeSource {
    fn describe(&self, repo: &str, file: &str) -> imt_ai::Result<RemoteFile> {
        if self.describe_fails {
            return Err(AiError::Network("offline".into()));
        }
        let (size, sha256) = if file.contains("mmproj") {
            (self.projector.len() as u64, self.projector_sha.clone())
        } else {
            (self.data.len() as u64, self.sha.clone())
        };
        Ok(RemoteFile {
            repo: repo.into(),
            file: file.into(),
            size,
            sha256,
            license: Some("apache-2.0".into()),
            gated: false,
        })
    }
    fn fetch(
        &self,
        r: &RemoteFile,
        offset: u64,
        sink: &mut dyn FnMut(&[u8]) -> imt_ai::Result<()>,
        cancel: &AtomicBool,
    ) -> imt_ai::Result<()> {
        self.fetches.fetch_add(1, Ordering::SeqCst);
        let data = if r.file.contains("mmproj") { &self.projector } else { &self.data };
        for chunk in data[offset as usize..r.size as usize].chunks(16 * 1024) {
            if cancel.load(Ordering::SeqCst) {
                return Err(AiError::Cancelled);
            }
            if self.slow {
                std::thread::sleep(Duration::from_millis(40));
            }
            sink(chunk)?;
        }
        Ok(())
    }
}

fn entry(src: &FakeSource) -> CatalogModel {
    let mut c = Catalog::embedded().models[0].clone();
    c.id = "fake".into();
    c.name = "Fake 4B".into();
    c.file = "fake-4b.gguf".into();
    c.hf_repo = "org/fake".into();
    c.sha256 = src.sha.clone();
    c.size_bytes = src.data.len() as u64;
    c.params_b = 4.0;
    c.projector = None;
    c
}

/// The same entry, able to read images: its projector is `mmproj-F16.gguf` in the repository.
fn vision_entry(src: &FakeSource) -> CatalogModel {
    CatalogModel {
        projector: Some(imt_ai::catalog::CatalogProjector {
            file: "mmproj-F16.gguf".into(),
            sha256: src.projector_sha.clone(),
            size_bytes: src.projector.len() as u64,
        }),
        ..entry(src)
    }
}

fn catalog(src: &FakeSource) -> Catalog {
    Catalog {
        version: 1,
        updated: String::new(),
        note: String::new(),
        models: vec![entry(src)],
    }
}

#[derive(Clone, Copy)]
enum Engine {
    Works,
    GpuFails,
    NeverLoads,
    WarmUpFails,
}

struct FakeRuntime {
    mode: Engine,
    loads: Mutex<Vec<GpuMode>>,
    projectors: Mutex<Vec<Option<std::path::PathBuf>>>,
}

impl FakeRuntime {
    fn new(mode: Engine) -> FakeRuntime {
        FakeRuntime {
            mode,
            loads: Mutex::new(vec![]),
            projectors: Mutex::new(vec![]),
        }
    }
}

impl ModelRuntime for FakeRuntime {
    fn load(&self, req: &LoadRequest) -> imt_ai::Result<Loaded> {
        self.loads.lock().unwrap().push(req.gpu);
        self.projectors.lock().unwrap().push(req.projector.clone());
        match (self.mode, req.gpu) {
            (Engine::NeverLoads, _) | (Engine::GpuFails, GpuMode::Auto | GpuMode::Layers(_)) => Err(AiError::Engine("device lost".into())),
            (m, gpu) => {
                let replies = if matches!(m, Engine::WarmUpFails) {
                    vec![Reply::Fail(AiError::Engine("warm-up crashed".into()))]
                } else {
                    vec![Reply::Text("OK".into())]
                };
                Ok(Loaded {
                    provider: Arc::new(ScriptedProvider::new(replies).with_context(req.context)),
                    backend: if gpu == GpuMode::Cpu { "CPU".into() } else { "Metal".into() },
                })
            }
        }
    }
}

struct Fixture {
    dir: tempfile::TempDir,
    store: ModelStore,
    catalog: Catalog,
    source: FakeSource,
    runtime: FakeRuntime,
    probe: Probe,
    disk: Disk,
    settings: AiSettings,
}

fn fixture(mode: Engine) -> Fixture {
    let dir = tempfile::tempdir().unwrap();
    let source = FakeSource::new();
    Fixture {
        store: ModelStore::new(dir.path().join("models")),
        catalog: catalog(&source),
        source,
        runtime: FakeRuntime::new(mode),
        probe: Probe(machine(Some(8.0), 16.0)),
        disk: Disk(Some(u64::MAX)),
        settings: AiSettings::default(),
        dir,
    }
}

impl Fixture {
    fn setup(&self) -> Setup<'_> {
        Setup {
            probe: &self.probe,
            source: &self.source,
            disk: &self.disk,
            runtime: &self.runtime,
            store: &self.store,
            catalog: &self.catalog,
            settings_path: Box::leak(self.dir.path().join("ai.json").into_boxed_path()),
            settings: self.settings.clone(),
            backoff: Duration::from_millis(1),
        }
    }
    fn settings_file(&self) -> std::path::PathBuf {
        self.dir.path().join("ai.json")
    }
}

fn run(f: &Fixture) -> (imt_ai::Result<SetupOutcome>, Vec<SetupStage>) {
    let mut stages = Vec::new();
    let model = f.catalog.models[0].clone();
    let r = f.setup().run(&model, &Progress::new(), &mut |s| stages.push(s));
    (r, stages)
}

fn names(stages: &[SetupStage]) -> Vec<&'static str> {
    let mut out: Vec<&'static str> = Vec::new();
    for s in stages {
        let n = match s {
            SetupStage::Probing => "probing",
            SetupStage::Recommending => "recommending",
            SetupStage::Downloading { .. } => "downloading",
            SetupStage::Verifying { .. } => "verifying",
            SetupStage::Validating => "validating",
            SetupStage::Loading { .. } => "loading",
            SetupStage::FellBackToCpu { .. } => "fallback",
            SetupStage::WarmingUp => "warming",
            SetupStage::Ready { .. } => "ready",
        };
        if out.last() != Some(&n) {
            out.push(n);
        }
    }
    out
}

#[test]
fn the_plan_shows_the_licence_and_download_size_before_anything_is_downloaded() {
    let f = fixture(Engine::Works);
    let plan = f.setup().plan().unwrap();
    assert_eq!(plan.chosen.as_ref().unwrap().model.id, "fake");
    assert_eq!(
        (plan.license.as_deref(), plan.gated, plan.already_installed, plan.download_bytes),
        (Some("apache-2.0"), false, false, f.source.data.len() as u64)
    );
    assert_eq!(f.source.fetches.load(Ordering::SeqCst), 0);
    assert!(plan.note.is_none() && plan.hardware.cpu_cores == 8);
    // host unreachable: the catalog's licence is shown with a note
    let mut f2 = fixture(Engine::Works);
    f2.source.describe_fails = true;
    let plan = f2.setup().plan().unwrap();
    assert_eq!(plan.license.as_deref(), Some("apache-2.0"));
    assert!(plan.note.unwrap().contains("offline"));
    // nothing fits
    let mut f3 = fixture(Engine::Works);
    f3.probe = Probe(machine(None, 1.0));
    assert!(f3.setup().plan().unwrap().chosen.is_none());
}

#[test]
fn set_up_runs_every_stage_and_remembers_the_model() {
    let f = fixture(Engine::Works);
    let (r, stages) = run(&f);
    let out = r.unwrap();
    assert_eq!(
        names(&stages),
        [
            "probing",
            "recommending",
            "downloading",
            "verifying",
            "validating",
            "loading",
            "warming",
            "ready"
        ]
    );
    assert!(matches!(stages.last().unwrap(), SetupStage::Ready { model, backend } if model == "fake-4b.gguf" && backend == "Metal"));
    assert!(matches!(
        stages.iter().find(|s| matches!(s, SetupStage::Loading { .. })),
        Some(SetupStage::Loading { gpu: true })
    ));
    assert!(stages
        .iter()
        .any(|s| matches!(s, SetupStage::Downloading { total, .. } if *total == f.source.data.len() as u64)));
    assert_eq!(out.backend, "Metal");
    assert!(out.notes.is_empty());
    assert!(f.store.dir().join("fake-4b.gguf").is_file() && !f.store.dir().join("fake-4b.gguf.part").exists());
    let (saved, warn) = AiSettings::load(&f.settings_file());
    assert!(warn.is_none());
    assert_eq!(saved.active_model.as_deref(), Some("fake-4b.gguf"));
    assert_eq!(out.settings.active_model, saved.active_model);
    // the stages serialise for the UI
    let j = serde_json::to_value(&stages[2]).unwrap();
    assert_eq!(j["stage"], "downloading");
    assert!(j["bytesPerSec"].is_number());
}

#[test]
fn an_installed_model_is_not_downloaded_again() {
    let f = fixture(Engine::Works);
    std::fs::create_dir_all(f.store.dir()).unwrap();
    std::fs::write(f.store.dir().join("fake-4b.gguf"), &f.source.data).unwrap();
    let plan = f.setup().plan().unwrap();
    assert!(plan.already_installed && plan.download_bytes == 0);
    let (r, stages) = run(&f);
    r.unwrap();
    assert_eq!(f.source.fetches.load(Ordering::SeqCst), 0);
    assert!(!names(&stages).contains(&"downloading"));
}

#[test]
fn a_gpu_failure_falls_back_to_the_cpu_with_a_note() {
    let f = fixture(Engine::GpuFails);
    let (r, stages) = run(&f);
    let out = r.unwrap();
    assert_eq!(out.backend, "CPU");
    assert!(out.notes[0].contains("GPU could not be used") && out.notes[0].contains("device lost"));
    assert!(names(&stages).contains(&"fallback"));
    assert_eq!(*f.runtime.loads.lock().unwrap(), vec![GpuMode::Auto, GpuMode::Cpu]);
    // settings can force the CPU from the start
    let mut f = fixture(Engine::GpuFails);
    f.settings.gpu_offload = GpuOffload::Layers(0);
    let (r, stages) = run(&f);
    assert_eq!(r.unwrap().backend, "CPU");
    assert!(matches!(
        stages.iter().find(|s| matches!(s, SetupStage::Loading { .. })),
        Some(SetupStage::Loading { gpu: false })
    ));
    assert_eq!(*f.runtime.loads.lock().unwrap(), vec![GpuMode::Cpu]);
}

#[test]
fn failures_at_each_stage_are_clear_and_leave_nothing_half_done() {
    // the engine cannot load the model at all
    let f = fixture(Engine::NeverLoads);
    assert!(matches!(run(&f).0, Err(AiError::Engine(_))));
    assert!(!f.settings_file().exists(), "settings are saved only after a model is ready");
    // warm-up fails
    let f = fixture(Engine::WarmUpFails);
    assert!(run(&f).0.err().unwrap().to_string().contains("warm-up crashed"));
    assert!(!f.settings_file().exists());
    // the catalog is stale
    let mut f = fixture(Engine::Works);
    f.source.sha = "0".repeat(64);
    assert!(matches!(run(&f).0, Err(AiError::Integrity(_))));
    // not enough disk space
    let mut f = fixture(Engine::Works);
    f.disk = Disk(Some(1000));
    assert!(matches!(run(&f).0, Err(AiError::Disk(_))));
    // too little memory
    let mut f = fixture(Engine::Works);
    f.probe = Probe(machine(None, 0.3));
    std::fs::create_dir_all(f.store.dir()).unwrap();
    std::fs::write(f.store.dir().join("fake-4b.gguf"), &f.source.data).unwrap();
    let mut big = f.catalog.models[0].clone();
    big.size_bytes = f.source.data.len() as u64;
    let mut stages = Vec::new();
    let mut st = f.setup();
    let tiny = Probe(Hardware {
        total_ram_gb: 0.4,
        available_ram_gb: 0.1,
        ..machine(None, 1.0)
    });
    st.probe = &tiny;
    assert!(matches!(
        st.run(&big, &Progress::new(), &mut |s| stages.push(s)),
        Err(AiError::Memory(_))
    ));
    // a file that is not a model
    let f = fixture(Engine::Works);
    std::fs::create_dir_all(f.store.dir()).unwrap();
    std::fs::write(f.store.dir().join("junk.gguf"), b"<html>error page</html>").unwrap();
    assert!(f
        .setup()
        .activate("junk.gguf", &Progress::new(), &mut |_| {})
        .err()
        .unwrap()
        .to_string()
        .contains("web page"));
    assert!(matches!(
        f.setup().activate("../x.gguf", &Progress::new(), &mut |_| {}),
        Err(AiError::Refused(_))
    ));
}

#[test]
fn cancelling_the_download_stops_set_up_and_keeps_the_partial_file() {
    let mut f = fixture(Engine::Works);
    f.source.slow = true;
    f.source.data = {
        let mut d = qwen_like();
        d.extend(blob(400_000));
        d
    };
    f.source.sha = sha256_hex(&f.source.data);
    f.catalog = catalog(&f.source);
    let progress = Progress::new();
    let flag = progress.cancel_flag();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        flag.store(true, Ordering::SeqCst);
    });
    let model = f.catalog.models[0].clone();
    let r = f.setup().run(&model, &progress, &mut |_| {});
    canceller.join().unwrap();
    assert_eq!(r.err(), Some(AiError::Cancelled));
    assert!(f.store.dir().join("fake-4b.gguf.part").exists() && !f.store.dir().join("fake-4b.gguf").exists());
}

#[test]
fn activating_a_model_the_user_added_works_without_a_catalog_entry() {
    let f = fixture(Engine::Works);
    let src = f.dir.path().join("mine.gguf");
    std::fs::write(&src, qwen_like()).unwrap();
    let added = f.store.add_local(&src, AddMode::Copy).unwrap();
    let mut stages = Vec::new();
    let out = f
        .setup()
        .activate(&added.file_name, &Progress::new(), &mut |s| stages.push(s))
        .unwrap();
    assert_eq!(out.settings.active_model.as_deref(), Some("mine.gguf"));
    assert_eq!(names(&stages), ["probing", "validating", "loading", "warming", "ready"]);
}

// ---- lazy loading ----

struct Loader {
    delay: Duration,
    fail_first: AtomicUsize,
    loads: AtomicUsize,
    panics: bool,
}

impl Loader {
    fn new(delay_ms: u64) -> Arc<Loader> {
        Arc::new(Loader {
            delay: Duration::from_millis(delay_ms),
            fail_first: AtomicUsize::new(0),
            loads: AtomicUsize::new(0),
            panics: false,
        })
    }
}

impl ModelLoader for Loader {
    fn load(&self) -> imt_ai::Result<Arc<dyn LlmProvider>> {
        self.loads.fetch_add(1, Ordering::SeqCst);
        std::thread::sleep(self.delay);
        if self.panics {
            panic!("native code blew up");
        }
        if self.fail_first.load(Ordering::SeqCst) > 0 {
            self.fail_first.fetch_sub(1, Ordering::SeqCst);
            return Err(AiError::Engine("out of memory".into()));
        }
        Ok(Arc::new(
            ScriptedProvider::new((0..50).map(|_| Reply::Text("hi".into())).collect()).with_context(2048),
        ))
    }
    fn context_hint(&self) -> usize {
        1234
    }
}

fn ask(p: &LazyProvider) -> imt_ai::Result<String> {
    p.chat(
        &ChatRequest {
            messages: vec![ChatMessage::user("x")],
            ..ChatRequest::default()
        },
        &mut |_| {},
    )
    .map(|r| r.text)
}

#[test]
fn the_model_loads_on_first_use_and_unloads_when_idle() {
    let loader = Loader::new(30);
    let p = LazyProvider::new(loader.clone(), Duration::from_millis(100));
    assert_eq!(
        (p.state(), p.context_length(), p.display_name().as_str(), p.id()),
        (LoadState::Unloaded, 1234, "Local model (not loaded)", "embedded")
    );
    assert_eq!(ask(&p).unwrap(), "hi");
    assert_eq!(p.state(), LoadState::Ready);
    assert_eq!((p.context_length(), p.display_name().as_str()), (2048, "Scripted test model"));
    assert!(!p.unload_if_idle(Instant::now()), "just used");
    assert!(p.unload_if_idle(Instant::now() + Duration::from_secs(1)));
    assert_eq!(p.state(), LoadState::Unloaded);
    assert_eq!(ask(&p).unwrap(), "hi");
    assert_eq!(loader.loads.load(Ordering::SeqCst), 2);
    p.set_idle(Duration::from_secs(3600));
    assert!(!p.unload_if_idle(Instant::now() + Duration::from_secs(60)));
    p.unload();
    assert_eq!(p.state(), LoadState::Unloaded);
}

#[test]
fn preload_runs_in_the_background() {
    let loader = Loader::new(150);
    let p = LazyProvider::new(loader.clone(), Duration::from_secs(60));
    let t = Instant::now();
    p.preload();
    p.preload();
    assert!(t.elapsed() < Duration::from_millis(100));
    assert_eq!(p.state(), LoadState::Loading);
    assert_eq!(ask(&p).unwrap(), "hi", "a question waits for the load");
    assert_eq!(loader.loads.load(Ordering::SeqCst), 1);
}

#[test]
fn load_failures_are_reported_and_retried() {
    let loader = Loader::new(0);
    loader.fail_first.store(1, Ordering::SeqCst);
    let p = LazyProvider::new(loader.clone(), Duration::from_secs(60));
    assert!(ask(&p).unwrap_err().to_string().contains("out of memory"));
    assert_eq!(ask(&p).unwrap(), "hi");
    // a crash inside the loader does not leave the state stuck in "loading"
    let panicking = Arc::new(Loader {
        delay: Duration::ZERO,
        fail_first: AtomicUsize::new(0),
        loads: AtomicUsize::new(0),
        panics: true,
    });
    let p = LazyProvider::new(panicking, Duration::from_secs(60));
    assert!(ask(&p).unwrap_err().to_string().contains("crashed"));
    assert!(matches!(p.state(), LoadState::Unloaded | LoadState::Failed(_) | LoadState::Loading));
}

#[test]
fn a_cancelled_question_does_not_wait_for_the_load() {
    let p = LazyProvider::new(Loader::new(400), Duration::from_secs(60));
    let cancel = Arc::new(AtomicBool::new(true));
    let t = Instant::now();
    let r = p.chat(
        &ChatRequest {
            messages: vec![ChatMessage::user("x")],
            cancel: Some(cancel),
            ..ChatRequest::default()
        },
        &mut |_| {},
    );
    assert_eq!(r.err(), Some(AiError::Cancelled));
    assert!(t.elapsed() < Duration::from_millis(300));
}

#[test]
fn the_idle_watcher_unloads_the_model_and_stops() {
    let p = Arc::new(LazyProvider::new(Loader::new(0), Duration::from_millis(60)));
    let watcher = p.spawn_idle_watcher(Duration::from_millis(20));
    assert_eq!(ask(&p).unwrap(), "hi");
    let end = Instant::now() + Duration::from_secs(3);
    while p.state() == LoadState::Ready && Instant::now() < end {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(p.state(), LoadState::Unloaded);
    p.shutdown();
    watcher.join().unwrap();
    // the watcher also ends when the provider is dropped
    let p = Arc::new(LazyProvider::new(Loader::new(0), Duration::from_secs(60)));
    let w = p.spawn_idle_watcher(Duration::from_millis(10));
    drop(p);
    w.join().unwrap();
}

#[test]
fn the_installed_model_loader_validates_checks_memory_and_loads() {
    let dir = tempfile::tempdir().unwrap();
    let store = ModelStore::new(dir.path().join("models"));
    std::fs::create_dir_all(store.dir()).unwrap();
    std::fs::write(store.dir().join("m.gguf"), qwen_like()).unwrap();
    let loader = |file: &str, hw: Hardware, mode: Engine| InstalledModelLoader {
        runtime: Arc::new(FakeRuntime::new(mode)),
        store: ModelStore::new(store.dir()),
        file_name: file.into(),
        settings: AiSettings::default(),
        hardware: hw,
    };
    let ok = loader("m.gguf", machine(Some(8.0), 16.0), Engine::Works);
    assert_eq!(ok.context_hint(), 8192);
    // the context never exceeds what the model was trained for (here 32768, above the setting)
    assert_eq!(ok.load().unwrap().context_length(), 8192);
    assert!(
        loader("m.gguf", machine(Some(8.0), 16.0), Engine::GpuFails).load().is_ok(),
        "falls back to the CPU"
    );
    assert!(loader("m.gguf", machine(Some(8.0), 16.0), Engine::NeverLoads).load().is_err());
    assert!(matches!(
        loader("../m.gguf", machine(None, 16.0), Engine::Works).load(),
        Err(AiError::Refused(_))
    ));
    assert!(matches!(
        loader("missing.gguf", machine(None, 16.0), Engine::Works).load(),
        Err(AiError::ModelFile(_))
    ));
    let p = LazyProvider::new(
        Arc::new(loader("m.gguf", machine(Some(8.0), 16.0), Engine::Works)),
        Duration::from_secs(60),
    );
    assert_eq!(ask(&p).unwrap(), "OK");
    let info = gguf::inspect(&store.dir().join("m.gguf")).unwrap();
    assert_eq!(
        effective_context(
            &AiSettings {
                context_length: 131_072,
                ..AiSettings::default()
            },
            &info
        ),
        32768
    );
    assert_eq!(GpuMode::from(GpuOffload::Layers(7)), GpuMode::Layers(7));
}

#[test]
fn a_model_that_reads_images_is_installed_with_its_projector() {
    let mut f = fixture(Engine::Works);
    f.catalog.models[0] = vision_entry(&f.source);
    let plan = f.setup().plan().unwrap();
    assert_eq!(plan.download_bytes, (f.source.data.len() + f.source.projector.len()) as u64);
    let (r, stages) = run(&f);
    r.unwrap();
    // One install downloads both, each verified; the projector is kept next to the model under its own name.
    assert_eq!(f.source.fetches.load(Ordering::SeqCst), 2);
    assert!(stages
        .iter()
        .any(|s| matches!(s, SetupStage::Downloading { total, .. } if *total == f.source.projector.len() as u64)));
    let p = f.store.dir().join("fake-4b.mmproj.gguf");
    assert!(p.is_file() && !f.store.dir().join("mmproj-F16.gguf").exists());
    let list = f.store.list();
    assert_eq!(list.len(), 1);
    assert!(list[0].vision());
    // The one model is loaded with its projector.
    assert_eq!(f.runtime.projectors.lock().unwrap().last().unwrap().as_deref(), Some(p.as_path()));
    assert_eq!(f.setup().plan().unwrap().download_bytes, 0);
}

#[test]
fn the_projector_of_an_installed_model_is_added_on_its_own() {
    let mut f = fixture(Engine::Works);
    std::fs::create_dir_all(f.store.dir()).unwrap();
    std::fs::write(f.store.dir().join("fake-4b.gguf"), &f.source.data).unwrap();
    let m = vision_entry(&f.source);
    f.catalog.models[0] = m.clone();
    let plan = f.setup().plan().unwrap();
    assert!(plan.already_installed);
    assert_eq!(
        plan.download_bytes,
        f.source.projector.len() as u64,
        "only the projector is missing"
    );
    f.setup().download_projector(&m, &Progress::new(), &mut |_| {}).unwrap();
    assert_eq!(f.source.fetches.load(Ordering::SeqCst), 1);
    assert!(f.store.list()[0].vision());
    // Again: nothing to do. A model without a projector: nothing either.
    f.setup().download_projector(&m, &Progress::new(), &mut |_| {}).unwrap();
    f.setup()
        .download_projector(&entry(&f.source), &Progress::new(), &mut |_| {})
        .unwrap();
    assert_eq!(f.source.fetches.load(Ordering::SeqCst), 1);
}

#[test]
fn a_projector_that_differs_from_the_catalog_is_refused_and_the_model_kept() {
    let mut f = fixture(Engine::Works);
    let m = vision_entry(&f.source);
    f.catalog.models[0] = m;
    f.source.projector_sha = "0".repeat(64);
    let (r, _) = run(&f);
    assert!(matches!(r, Err(AiError::Integrity(_))));
    assert!(f.store.dir().join("fake-4b.gguf").is_file());
    assert!(!f.store.dir().join("fake-4b.mmproj.gguf").exists());
    assert!(!f.settings_file().exists());
}
