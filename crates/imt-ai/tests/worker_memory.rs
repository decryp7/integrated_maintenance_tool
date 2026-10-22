//! The AI worker's own memory cap (Settings ▸ Performance, ADR 0031): the host's watcher stops a worker over it and
//! tells the listener, and the cap is read at every launch.

use imt_ai::worker::{fixed_env, CappedLauncher, FramesLauncher, WorkerLauncher};
use imt_ipc::memory_watch::{MemoryEvent, MemoryListener};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

fn launcher(cap: Arc<AtomicU64>, heard: Arc<Mutex<Vec<(String, MemoryEvent)>>>) -> CappedLauncher {
    CappedLauncher {
        launcher: FramesLauncher {
            exe: PathBuf::from(env!("CARGO_BIN_EXE_imt-ai-fake-worker")),
            args: vec![],
            env: fixed_env(vec![(
                "LLVM_PROFILE_FILE".into(),
                if cfg!(windows) { "NUL".into() } else { "/dev/null".into() },
            )]),
        },
        memory_cap: Arc::new(move || Some(cap.load(Ordering::Relaxed))),
        listener: Some(MemoryListener::new(move |label, e| {
            heard.lock().unwrap().push((label.to_string(), e))
        })),
    }
}

#[test]
fn a_worker_over_its_cap_is_stopped_and_the_host_hears_it() {
    let heard: Arc<Mutex<Vec<(String, MemoryEvent)>>> = Arc::default();
    // 1 MB: any process is over it at the first look.
    let cap = Arc::new(AtomicU64::new(1 << 20));
    let l = launcher(cap.clone(), heard.clone());
    let _transport = l.launch();
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline && heard.lock().unwrap().is_empty() {
        std::thread::sleep(Duration::from_millis(20));
    }
    let events = heard.lock().unwrap().clone();
    let (label, e) = events.first().cloned().expect("the watcher reported the worker");
    assert_eq!(label, "the AI engine");
    assert!(
        matches!(e, MemoryEvent::Stopped(u) if u.limit == 1 << 20 && u.used > u.limit),
        "{e:?}"
    );
    // A raised cap reaches the next launch: that worker runs.
    heard.lock().unwrap().clear();
    cap.store(4 << 30, Ordering::Relaxed);
    let t = l.launch().expect("the worker starts under a generous cap");
    std::thread::sleep(Duration::from_millis(600));
    assert!(
        !heard.lock().unwrap().iter().any(|(_, e)| matches!(e, MemoryEvent::Stopped(_))),
        "not stopped"
    );
    drop(t);
}
