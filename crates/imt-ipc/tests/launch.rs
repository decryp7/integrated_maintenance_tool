//! The launcher, the authenticated connection and the supervisor against real child processes (the fake child):
//! the host listens and the child connects; wrong tokens and processes the host did not start are refused; a
//! child that crashes, hangs, sends garbage or an oversized frame fails its calls and is restarted (with its
//! state restored) or quarantined; the sandbox denies what it should.

use imt_ipc::launch::{launch, Child, ChildSpec};
use imt_ipc::peer::{code, CallCtx, Handler, NoCalls, StreamEvent};
use imt_ipc::process::SpawnSpec;
use imt_ipc::supervisor::{ChildState, Supervisor, SupervisorConfig};
use imt_ipc::{IpcError, RemoteError};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn spec(args: &[&str]) -> ChildSpec {
    let mut s = SpawnSpec::new(env!("CARGO_BIN_EXE_imt-ipc-fake-child"), "the fake child");
    s.args = args.iter().map(|a| a.to_string()).collect();
    s.endpoint_prefix = "imt-test".into();
    s.connect_window = Duration::from_secs(10);
    ChildSpec::new(s)
}

const T: Option<Duration> = Some(Duration::from_secs(10));

fn ask(c: &Child, text: &str) -> Result<String, IpcError> {
    c.peer()
        .call(text.as_bytes().to_vec(), T)
        .map(|b| String::from_utf8_lossy(&b).into_owned())
}

#[test]
fn a_child_connects_authenticates_and_answers() {
    let mut s = spec(&[]);
    s.spawn.env = vec![("IMT_TEST".into(), "1".into())];
    let host: Arc<dyn Handler> = Arc::new(|_: &CallCtx, b: &[u8]| -> Result<Vec<u8>, RemoteError> { Ok([b"host says ", b].concat()) });
    let c = launch(&s, host).unwrap();
    assert!(c.pid() > 0 && !c.exited());
    assert_eq!(ask(&c, "echo héllo").unwrap(), "héllo");
    // A cleared environment plus what the spec names.
    assert_eq!(ask(&c, "env IMT_TEST").unwrap(), "1");
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
    let dir = std::env::temp_dir().join(format!("imt-sandbox-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("written.txt");
    let open = launch(&spec(&[]), Arc::new(NoCalls)).unwrap();
    assert_eq!(ask(&open, &format!("write {}", target.display())).unwrap(), "ok");
    assert_eq!(ask(&open, "spawn").unwrap(), "ok");
    let locked = launch(&spec(&[]).sandbox(imt_ipc::sandbox::SandboxSpec::locked()), Arc::new(NoCalls)).unwrap();
    let w = ask(&locked, &format!("write {}", target.display())).unwrap();
    assert!(w.starts_with("denied"), "a low-integrity child wrote a user file: {w}");
    let p = ask(&locked, "spawn").unwrap();
    assert!(p.starts_with("denied"), "a child without children started a program: {p}");
    // Its connection still works (opened before the lockdown).
    assert_eq!(ask(&locked, "echo alive").unwrap(), "alive");
    let _ = std::fs::remove_dir_all(&dir);
}

#[cfg(windows)]
#[test]
fn the_job_memory_cap_fails_a_large_allocation() {
    let capped = imt_ipc::sandbox::SandboxSpec {
        memory_limit: Some(64 << 20),
        ..imt_ipc::sandbox::SandboxSpec::unrestricted()
    };
    let c = launch(&spec(&[]).sandbox(capped), Arc::new(NoCalls)).unwrap();
    // 200 MB cannot be committed under a 64 MiB cap: the child dies (allocation failure aborts) or errors.
    assert!(ask(&c, "big 200000000").is_err());
}

/// What the child's lockdown could not enforce on this system (a CI container may filter landlock or seccomp).
#[cfg(target_os = "linux")]
fn gaps(c: &imt_ipc::Child) -> Vec<imt_ipc::sandbox::Gap> {
    let notes: Vec<imt_ipc::sandbox::Note> = serde_json::from_str(&ask(c, "gaps").unwrap()).unwrap();
    notes.into_iter().map(|n| n.gap).collect()
}

#[cfg(target_os = "linux")]
#[test]
fn a_locked_child_cannot_write_files_or_start_programs() {
    use imt_ipc::sandbox::Gap;
    let target = format!("/tmp/imt-sandbox-test-{}", std::process::id());
    let open = launch(&spec(&[]), Arc::new(NoCalls)).unwrap();
    assert_eq!(ask(&open, &format!("write {target}")).unwrap(), "ok");
    assert!(gaps(&open).is_empty(), "an unrestricted child has nothing to degrade");
    let locked = launch(&spec(&[]).sandbox(imt_ipc::sandbox::SandboxSpec::locked()), Arc::new(NoCalls)).unwrap();
    // Whatever this kernel lacks, the child runs, says so, and its connection works.
    assert_eq!(ask(&locked, "echo alive").unwrap(), "alive");
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

#[cfg(target_os = "linux")]
#[test]
fn the_memory_cap_fails_a_large_allocation_but_not_a_busy_child() {
    let capped = imt_ipc::sandbox::SandboxSpec {
        memory_limit: Some(256 << 20),
        ..imt_ipc::sandbox::SandboxSpec::unrestricted()
    };
    let open = launch(&spec(&[]), Arc::new(NoCalls)).unwrap();
    assert!(ask(&open, "alloc 400000000").unwrap().starts_with("ok"));
    let c = launch(&spec(&[]).sandbox(capped), Arc::new(NoCalls)).unwrap();
    // Many calls at once (handler threads, each a malloc arena that reserves address space) stay well within a
    // 256 MiB cap of the memory used, which an address-space cap did not.
    let answers: Vec<_> = std::thread::scope(|s| {
        let calls: Vec<_> = (0..32).map(|_| s.spawn(|| ask(&c, "alloc 2000000"))).collect();
        calls.into_iter().map(|h| h.join().unwrap()).collect()
    });
    assert!(
        answers.iter().all(|a| a.as_deref().is_ok_and(|a| a.starts_with("ok"))),
        "{answers:?}"
    );
    let big = ask(&c, "alloc 400000000").unwrap();
    assert!(big.starts_with("denied"), "{big}");
}
