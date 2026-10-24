//! The launcher, the authenticated connection and the supervisor against real child processes (the fake child):
//! the host listens and the child connects; wrong tokens and processes the host did not start are refused; a
//! child that crashes, hangs, sends garbage or an oversized frame fails its calls and is restarted (with its
//! state restored) or quarantined; the sandbox denies what it should.

use debrief_ipc::launch::{launch, Child, ChildSpec};
use debrief_ipc::peer::{code, CallCtx, Handler, NoCalls, StreamEvent};
use debrief_ipc::process::SpawnSpec;
use debrief_ipc::supervisor::{ChildState, Supervisor, SupervisorConfig};
use debrief_ipc::{IpcError, RemoteError};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn spec(args: &[&str]) -> ChildSpec {
    let mut s = SpawnSpec::new(env!("CARGO_BIN_EXE_debrief-ipc-fake-child"), "the fake child");
    s.args = args.iter().map(|a| a.to_string()).collect();
    s.endpoint_prefix = "debrief-test".into();
    s.connect_window = Duration::from_secs(10);
    ChildSpec::new(s)
}

const T: Option<Duration> = Some(Duration::from_secs(10));

/// Like [`ask`] for a call that touches hundreds of MB: an upper bound for a loaded, memory-limited CI container (page
/// faults there are throttled), not a delay; the answer returns as soon as it is ready.
fn ask_big(c: &Child, text: &str) -> Result<String, IpcError> {
    c.peer()
        .call(text.as_bytes().to_vec(), Some(Duration::from_secs(120)))
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

fn ask(c: &Child, text: &str) -> Result<String, IpcError> {
    c.peer()
        .call(text.as_bytes().to_vec(), T)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[test]
fn a_child_connects_authenticates_and_answers() {
    let mut s = spec(&[]);
    s.spawn.env = vec![("DEBRIEF_TEST".into(), "1".into())];
    let host: Arc<dyn Handler> = Arc::new(|_: &CallCtx, b: &[u8]| -> Result<Vec<u8>, RemoteError> { Ok([b"host says ", b].concat()) });
    let c = launch(&s, host).unwrap();
    assert!(c.pid() > 0 && !c.exited());
    assert_eq!(ask(&c, "echo héllo").unwrap(), "héllo");
    // A cleared environment plus what the spec names.
    assert_eq!(ask(&c, "env DEBRIEF_TEST").unwrap(), "1");
    assert_eq!(ask(&c, "env CARGO_PKG_NAME").unwrap(), "<unset>");
    // Calls back into the host.
    assert_eq!(ask(&c, "ask hi").unwrap(), "host says hi");
    // A panic is a typed error; the child keeps serving.
    assert!(matches!(ask(&c, "panic"), Err(IpcError::Remote(e)) if e.code == code::PANIC));
    assert_eq!(ask(&c, "echo still").unwrap(), "still");
    // Streams.
    let mut st = c.peer().stream(b"stream 50".to_vec(), 4).unwrap();
    let mut n = 0;
    while let StreamEvent::Item(_) = st.next(T).unwrap() {
        n += 1;
    }
    assert_eq!(n, 50);
    // A large answer within the frame limit.
    assert_eq!(ask(&c, "big 2000000").unwrap().len(), 2_000_000);
    c.stop();
    let started = Instant::now();
    while !c.exited() && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(c.exited());
}

#[test]
fn children_that_die_or_never_connect_fail_the_launch() {
    match launch(&spec(&["--exit-now"]), Arc::new(NoCalls)) {
        Err(IpcError::Died(m)) => assert!(m.contains("exited while starting"), "{m}"),
        Err(e) => panic!("{e:?}"),
        Ok(_) => panic!("launched"),
    }
    let mut s = spec(&["--no-connect"]);
    s.spawn.connect_window = Duration::from_millis(500);
    match launch(&s, Arc::new(NoCalls)) {
        Err(IpcError::Died(m)) => assert!(m.contains("did not connect"), "{m}"),
        Err(e) => panic!("{e:?}"),
        Ok(_) => panic!("launched"),
    }
}

#[test]
fn a_child_of_another_protocol_version_is_refused_with_what_to_update() {
    let host = |args: &[&str]| {
        let mut s = spec(args);
        s.peer.protocol = debrief_ipc::frame::ProtocolVersion::new(1, 2);
        s
    };
    assert!(launch(&host(&["--protocol=1.0"]), Arc::new(NoCalls)).is_ok());
    for (args, says) in [
        (["--protocol=0.9"], "too old"),
        (["--protocol=1.3"], "needs a newer Debrief"),
        (["--protocol=2.0"], "needs a newer Debrief"),
    ] {
        match launch(&host(&args), Arc::new(NoCalls)) {
            Err(IpcError::Spawn(m)) => assert!(m.contains(says) && m.contains("the fake child"), "{m}"),
            Err(e) => panic!("{args:?}: {e:?}"),
            Ok(_) => panic!("{args:?} accepted"),
        }
    }
    // Unversioned on both sides (the workers) still connects.
    assert!(launch(&spec(&[]), Arc::new(NoCalls)).is_ok());
}

#[test]
fn a_wrong_token_or_another_process_is_refused() {
    match launch(&spec(&["--bad-token"]), Arc::new(NoCalls)) {
        Err(IpcError::Protocol(m)) => assert!(m.contains("wrong session token"), "{m}"),
        Err(e) => panic!("{e:?}"),
        Ok(_) => panic!("accepted a wrong token"),
    }
    // The right token from a process the host did not start (the OS reports the connecting pid).
    match launch(&spec(&["--relay"]), Arc::new(NoCalls)) {
        Err(IpcError::Protocol(m)) => assert!(m.contains("is not the fake child"), "{m}"),
        Err(e) => panic!("{e:?}"),
        Ok(_) => panic!("accepted another process"),
    }
}

#[test]
fn garbage_or_an_oversized_frame_after_the_handshake_is_a_protocol_failure() {
    for flag in ["--garbage", "--oversize"] {
        let c = launch(&spec(&[flag]), Arc::new(NoCalls)).unwrap();
        assert!(c.peer().wait_closed(Some(Duration::from_secs(10))), "{flag}");
        assert!(
            matches!(c.peer().closed(), Some(IpcError::Protocol(_))),
            "{flag}: {:?}",
            c.peer().closed()
        );
        assert!(matches!(ask(&c, "echo x"), Err(IpcError::Protocol(_))));
    }
}

#[test]
fn a_crash_fails_the_call_and_a_hang_is_a_timeout() {
    let c = launch(&spec(&[]), Arc::new(NoCalls)).unwrap();
    match ask(&c, "exit 7") {
        Err(IpcError::Died(_)) => {}
        other => panic!("{other:?}"),
    }
    let c = launch(&spec(&[]), Arc::new(NoCalls)).unwrap();
    let started = Instant::now();
    assert_eq!(
        c.peer().call(b"hang".to_vec(), Some(Duration::from_millis(100))).unwrap_err(),
        IpcError::Timeout
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    // A killed child: its calls fail at once.
    let c = launch(&spec(&[]), Arc::new(NoCalls)).unwrap();
    c.stop();
    assert!(ask(&c, "echo x").is_err());
}

#[test]
fn the_supervisor_restarts_restores_and_quarantines_real_children() {
    let restores = Arc::new(AtomicUsize::new(0));
    let r2 = restores.clone();
    let cfg = SupervisorConfig {
        backoff_base: Duration::from_millis(1),
        backoff_max: Duration::from_millis(4),
        ..SupervisorConfig::new("the fake child")
    };
    let s = spec(&[]);
    let sup = Supervisor::new(cfg, move || launch(&s, Arc::new(NoCalls))).with_restore(move |c: &Child, _| {
        r2.fetch_add(1, Ordering::SeqCst);
        ask(c, "echo restored").map(|_| ())
    });
    let first = sup.call(|c| Ok(c.pid())).unwrap();
    assert_eq!(sup.status().state, ChildState::Running);
    assert!(matches!(sup.call(|c| ask(c, "exit 1")), Err(IpcError::Died(_))));
    let second = sup.call(|c| Ok(c.pid())).unwrap();
    assert_ne!(first, second, "a fresh process");
    assert_eq!(restores.load(Ordering::SeqCst), 2, "restored after the restart");
    // Typed errors are not failures.
    assert!(matches!(sup.call(|c| ask(c, "nope")), Err(IpcError::Remote(_))));
    assert_eq!(sup.status().crashes_in_window, 1);
    let _ = sup.call(|c| c.peer().call(b"hang".to_vec(), Some(Duration::from_millis(50))));
    let _ = sup.call(|c| ask(c, "exit 1"));
    assert_eq!(sup.status().state, ChildState::Quarantined);
    assert!(matches!(sup.call(|c| ask(c, "echo x")), Err(IpcError::Spawn(_))));
}

#[cfg(windows)]
#[test]
fn a_locked_child_cannot_write_the_users_files_or_start_programs() {
    let dir = std::env::temp_dir().join(format!("debrief-sandbox-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("written.txt");
    let open = launch(&spec(&[]), Arc::new(NoCalls)).unwrap();
    assert_eq!(ask(&open, &format!("write {}", target.display())).unwrap(), "ok");
    assert_eq!(ask(&open, "spawn").unwrap(), "ok");
    let locked = launch(&spec(&[]).sandbox(debrief_ipc::sandbox::SandboxSpec::locked()), Arc::new(NoCalls)).unwrap();
    let w = ask(&locked, &format!("write {}", target.display())).unwrap();
    assert!(w.starts_with("denied"), "a low-integrity child wrote a user file: {w}");
    let p = ask(&locked, "spawn").unwrap();
    assert!(p.starts_with("denied"), "a child without children started a program: {p}");
    // Its connection still works (opened before the lockdown).
    assert_eq!(ask(&locked, "echo alive").unwrap(), "alive");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A copy of the fake child in its own folder (a contained child can only run from folders granted to it; granting
/// cargo's target folder would rewrite the permissions of everything in it), contained by the launcher.
#[cfg(any(windows, target_os = "linux"))]
fn contained(args: &[&str], root: &std::path::Path) -> ChildSpec {
    let bin = root.join("bin");
    let data = root.join("data");
    std::fs::create_dir_all(&bin).unwrap();
    std::fs::create_dir_all(&data).unwrap();
    let exe = bin.join(
        std::path::Path::new(env!("CARGO_BIN_EXE_debrief-ipc-fake-child"))
            .file_name()
            .unwrap(),
    );
    std::fs::copy(env!("CARGO_BIN_EXE_debrief-ipc-fake-child"), &exe).unwrap();
    let mut s = spec(args);
    s.spawn.exe = exe;
    s.spawn.sandbox = debrief_ipc::sandbox::SandboxSpec {
        containment: debrief_ipc::sandbox::Containment::Launcher {
            name: "debrief.test.fake-child".into(),
            read: vec![bin],
            write: vec![data],
        },
        ..debrief_ipc::sandbox::SandboxSpec::locked()
    };
    s
}

#[cfg(windows)]
#[test]
fn a_contained_child_is_confined_even_when_it_skips_its_own_lockdown() {
    let root = tempfile::tempdir().unwrap();
    // A file of the user's, outside every granted folder.
    let user = tempfile::tempdir().unwrap();
    let secret = user.path().join("secret.txt");
    std::fs::write(&secret, b"top secret").unwrap();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    for args in [&[][..], &["--skip-lockdown"][..]] {
        let c = launch(&contained(args, root.path()), Arc::new(NoCalls)).unwrap();
        assert_eq!(ask(&c, "echo contained").unwrap(), "contained", "{args:?}");
        let r = ask(&c, &format!("read {}", secret.display())).unwrap();
        assert!(r.starts_with("denied"), "{args:?}: a contained child read a user file: {r}");
        let w = ask(&c, &format!("write {}", user.path().join("x.txt").display())).unwrap();
        assert!(w.starts_with("denied"), "{args:?}: {w}");
        let own = root.path().join("data").join("own.txt");
        assert_eq!(
            ask(&c, &format!("write {}", own.display())).unwrap(),
            "ok",
            "{args:?}: its data folder"
        );
        let n = ask(&c, &format!("connect {addr}")).unwrap();
        assert!(
            n.starts_with("denied"),
            "{args:?}: a contained child without the network connected: {n}"
        );
        let p = ask(&c, "spawn").unwrap();
        assert!(p.starts_with("denied"), "{args:?}: {p}");
        // The environment is the spec's only.
        assert_eq!(ask(&c, "env CARGO_PKG_NAME").unwrap(), "<unset>");
        c.stop();
    }
    // A wrong token is still refused from inside a container.
    assert!(matches!(
        launch(&contained(&["--bad-token"], root.path()), Arc::new(NoCalls)),
        Err(IpcError::Protocol(_))
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn a_contained_child_is_confined_even_when_it_skips_its_own_lockdown() {
    // Containment by the launcher is landlock: a kernel (or container) without it cannot confine files.
    if !std::fs::read_to_string("/sys/kernel/security/lsm").is_ok_and(|l| l.split(',').any(|m| m.trim() == "landlock")) {
        eprintln!("skipped: landlock is not enabled in this kernel or container");
        return;
    }
    let root = tempfile::tempdir().unwrap();
    let user = tempfile::tempdir().unwrap();
    let secret = user.path().join("secret.txt");
    std::fs::write(&secret, b"top secret").unwrap();
    let c = launch(&contained(&["--skip-lockdown"], root.path()), Arc::new(NoCalls)).unwrap();
    assert_eq!(ask(&c, "echo contained").unwrap(), "contained");
    let r = ask(&c, &format!("read {}", secret.display())).unwrap();
    assert!(r.starts_with("denied"), "{r}");
    let w = ask(&c, &format!("write {}", user.path().join("x.txt").display())).unwrap();
    assert!(w.starts_with("denied"), "{w}");
    let own = root.path().join("data").join("own.txt");
    assert_eq!(ask(&c, &format!("write {}", own.display())).unwrap(), "ok");
    let n = ask(&c, "connect 127.0.0.1:9").unwrap();
    assert!(n.starts_with("denied"), "{n}");
}

/// The launcher reports how far landlock confined a contained child (from between fork and exec): nothing when it
/// did, the one status line when the kernel lacks landlock (user decision: no refusal, just this line).
#[cfg(target_os = "linux")]
#[test]
fn a_contained_child_reports_whether_its_files_are_confined() {
    use debrief_ipc::sandbox::{Gap, FILES_NOT_CONFINED};
    let root = tempfile::tempdir().unwrap();
    let c = launch(&contained(&["--skip-lockdown"], root.path()), Arc::new(NoCalls)).unwrap();
    assert_eq!(ask(&c, "echo contained").unwrap(), "contained");
    let files: Vec<String> = c
        .sandbox_notes()
        .iter()
        .filter(|n| n.gap == Gap::Files)
        .map(|n| n.message.clone())
        .collect();
    if debrief_ipc::sandbox::files_confinable() {
        assert!(files.iter().all(|m| m.contains("partly")), "{files:?}");
        assert!(debrief_ipc::sandbox::host_notes(&debrief_ipc::SandboxSpec::locked()).is_empty());
    } else {
        assert_eq!(files, [FILES_NOT_CONFINED]);
        assert_eq!(
            debrief_ipc::sandbox::host_notes(&debrief_ipc::SandboxSpec::locked())[0].message,
            FILES_NOT_CONFINED
        );
    }
}

/// What the child's lockdown could not enforce on this system (a CI container may filter landlock or seccomp).
#[cfg(target_os = "linux")]
fn notes(c: &debrief_ipc::Child) -> Vec<debrief_ipc::sandbox::Note> {
    serde_json::from_str(&ask(c, "gaps").unwrap()).unwrap()
}

#[cfg(target_os = "linux")]
fn gaps(c: &debrief_ipc::Child) -> Vec<debrief_ipc::sandbox::Gap> {
    notes(c).into_iter().map(|n| n.gap).collect()
}

#[cfg(target_os = "linux")]
#[test]
fn a_locked_child_cannot_write_files_or_start_programs() {
    use debrief_ipc::sandbox::Gap;
    let target = format!("/tmp/debrief-sandbox-test-{}", std::process::id());
    let open = launch(&spec(&[]), Arc::new(NoCalls)).unwrap();
    assert_eq!(ask(&open, &format!("write {target}")).unwrap(), "ok");
    assert!(gaps(&open).is_empty(), "an unrestricted child has nothing to degrade");
    let locked = launch(&spec(&[]).sandbox(debrief_ipc::sandbox::SandboxSpec::locked()), Arc::new(NoCalls)).unwrap();
    // Whatever this kernel lacks, the child runs, says so, and its connection works.
    assert_eq!(ask(&locked, "echo alive").unwrap(), "alive");
    // The child locks down before it starts any thread (landlock restricts only those started afterwards), and
    // the handler runs on threads started after it: the checks below are made from such a thread.
    let notes = notes(&locked);
    // Where landlock is enforced, no thread may escape it; without landlock no thread is file-restricted anyway (and
    // that gap is reported on its own), so an early thread changes nothing there.
    let landlock_missing = notes
        .iter()
        .any(|n| n.message.contains("landlock is not available") || n.message.contains("landlock is not enabled"));
    if landlock_missing {
        eprintln!("skipped the early-thread check: landlock is not available here: {notes:?}");
    } else {
        assert!(!notes.iter().any(|n| n.message.contains("before the lockdown")), "{notes:?}");
    }
    let gaps = gaps(&locked);
    let w = ask(&locked, &format!("write {target}")).unwrap();
    if gaps.contains(&Gap::Files) {
        eprintln!("skipped the file check: landlock is not available here (the child logged it); wrote: {w}");
    } else {
        assert!(w.starts_with("denied"), "{w}");
    }
    let p = ask(&locked, "spawn").unwrap();
    if gaps.contains(&Gap::Syscalls) {
        eprintln!("skipped the program check: seccomp is not available here (the child logged it); spawn: {p}");
    } else {
        assert!(p.starts_with("denied"), "{p}");
    }
    let _ = std::fs::remove_file(&target);
}

#[test]
fn the_memory_cap_stops_a_child_over_it_but_not_a_busy_one() {
    let capped = debrief_ipc::sandbox::SandboxSpec {
        memory_limit: Some(256 << 20),
        ..debrief_ipc::sandbox::SandboxSpec::unrestricted()
    };
    let open = launch(&spec(&[]), Arc::new(NoCalls)).unwrap();
    assert!(ask_big(&open, "alloc 400000000").unwrap().starts_with("ok"));
    let c = launch(&spec(&[]).sandbox(capped), Arc::new(NoCalls)).unwrap();
    // Many calls at once (handler threads, each a malloc arena that reserves address space) stay well within a
    // 256 MiB cap of the memory used, which an address-space or data rlimit did not.
    let answers: Vec<_> = std::thread::scope(|s| {
        let calls: Vec<_> = (0..32).map(|_| s.spawn(|| ask_big(&c, "alloc 2000000"))).collect();
        calls.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(
        answers.iter().all(|a| a.as_deref().is_ok_and(|a| a.starts_with("ok"))),
        "{answers:?}"
    );
    // Over the cap, the host stops the child (it watches the memory used; an rlimit counted reservations).
    assert!(ask(&c, "alloc 400000000").is_err());
    assert!(c.exit_text().contains("memory cap of 256 MB"), "{}", c.exit_text());
}

#[test]
fn the_memory_watcher_tells_the_host_at_80_percent_at_the_stop_and_at_the_end() {
    use debrief_ipc::memory_watch::{MemoryEvent, MemoryListener};
    let heard: Arc<std::sync::Mutex<Vec<(String, MemoryEvent)>>> = Arc::default();
    let h = heard.clone();
    let listener = MemoryListener::new(move |label, e| h.lock().unwrap().push((label.to_string(), e)));
    let capped = |mb: u64| {
        let mut s = spec(&[]).sandbox(debrief_ipc::sandbox::SandboxSpec {
            memory_limit: Some(mb << 20),
            ..debrief_ipc::sandbox::SandboxSpec::unrestricted()
        });
        s.spawn.memory_listener = Some(listener.clone());
        s
    };
    let events = |heard: &std::sync::Mutex<Vec<(String, MemoryEvent)>>| heard.lock().unwrap().clone();
    let wait_for = |what: &dyn Fn(&MemoryEvent) -> bool| {
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        while std::time::Instant::now() < deadline {
            if events(&heard).iter().any(|(_, e)| what(e)) {
                return true;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        false
    };
    // 220 MB held for a second in a child capped at 256 MB: past 80%, under the cap.
    let c = launch(&capped(256), Arc::new(NoCalls)).unwrap();
    assert!(ask(&c, "hold 220000000 1200").unwrap().starts_with("ok"));
    assert!(wait_for(&|e| matches!(e, MemoryEvent::Warning(_))), "{:?}", events(&heard));
    let (label, warning) = events(&heard)
        .into_iter()
        .find(|(_, e)| matches!(e, MemoryEvent::Warning(_)))
        .unwrap();
    assert_eq!(label, "the fake child");
    let u = warning.usage();
    assert!(u.used >= u.limit * 8 / 10 && u.used <= u.limit && u.limit == 256 << 20, "{u:?}");
    assert!(c.memory_peak() >= 200 << 20, "{}", c.memory_peak());
    // Ending normally reports the peak.
    c.stop();
    assert!(
        wait_for(&|e| matches!(e, MemoryEvent::Ended(u) if u.peak >= 200 << 20)),
        "{:?}",
        events(&heard)
    );
    // Over the cap: stopped, with what it used.
    heard.lock().unwrap().clear();
    let c = launch(&capped(128), Arc::new(NoCalls)).unwrap();
    assert!(ask(&c, "hold 400000000 3000").is_err());
    assert!(
        wait_for(&|e| matches!(e, MemoryEvent::Stopped(u) if u.used > u.limit && u.peak >= u.used)),
        "{:?}",
        events(&heard)
    );
    assert!(
        !events(&heard).iter().any(|(_, e)| matches!(e, MemoryEvent::Ended(_))),
        "a stop is not also an end"
    );
}
