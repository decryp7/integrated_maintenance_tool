//! The transport against a real child process (the fake child): connect, token, typed errors, deadlines,
//! crashes, hangs, the stdin lifeline, and the supervisor restarting and quarantining a real process.

use imt_ipc::client::AuthChannel;
use imt_ipc::error::from_status;
use imt_ipc::pb::control_client::ControlClient;
use imt_ipc::process::{launch, ChildProcess, Launched, SpawnSpec};
use imt_ipc::supervisor::{ChildState, Connection, Supervisor, SupervisorConfig};
use imt_ipc::testing::echo_client::EchoClient;
use imt_ipc::testing::EchoRequest;
use imt_ipc::{IpcError, RemoteError};
use std::time::{Duration, Instant};
use tonic::Request;

fn rt() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap()
}

fn spec(args: &[&str]) -> SpawnSpec {
    let mut s = SpawnSpec::new(env!("CARGO_BIN_EXE_imt-ipc-fake-child"), "the fake child");
    s.args = args.iter().map(|a| a.to_string()).collect();
    s.endpoint_prefix = "imt-test".into();
    s.connect_window = Duration::from_secs(10);
    s
}

async fn echo(
    ch: &AuthChannel,
    process: &ChildProcess,
    text: &str,
    action: &str,
    delay_ms: u64,
    timeout: Duration,
) -> Result<String, IpcError> {
    let mut c = EchoClient::new(ch.clone());
    let mut req = Request::new(EchoRequest {
        text: text.into(),
        delay_ms,
        action: action.into(),
    });
    req.set_timeout(timeout);
    match tokio::time::timeout(timeout + Duration::from_millis(500), c.echo(req)).await {
        Err(_) => Err(IpcError::Timeout),
        Ok(Err(s)) => Err(from_status(&s, "the fake child", || process.exit_text())),
        Ok(Ok(r)) => Ok(r.into_inner().text),
    }
}

#[test]
fn a_child_is_started_connected_and_answers_with_the_token() {
    let rt = rt();
    let mut s = spec(&[]);
    s.clear_env = true;
    s.env = vec![("IMT_TEST".into(), "1".into())];
    s.current_dir = Some(std::env::temp_dir());
    let l = rt.block_on(launch(&s)).unwrap();
    assert!(l.process.pid() > 0);
    #[cfg(unix)]
    {
        // Below-normal priority.
        // SAFETY: plain call with integer arguments.
        let nice = unsafe { libc::getpriority(libc::PRIO_PROCESS, l.process.pid() as libc::id_t) };
        assert_eq!(nice, 10);
    }
    let t = Duration::from_secs(5);
    assert_eq!(rt.block_on(echo(&l.channel, &l.process, "héllo", "", 0, t)).unwrap(), "héllo");
    // Typed errors pass through; a panicking handler is an error, not a dead process.
    assert_eq!(
        rt.block_on(echo(&l.channel, &l.process, "", "error", 0, t)).unwrap_err(),
        IpcError::Remote(RemoteError::new(7, "typed failure"))
    );
    assert!(rt.block_on(echo(&l.channel, &l.process, "", "panic", 0, t)).is_err());
    assert_eq!(rt.block_on(echo(&l.channel, &l.process, "still", "", 0, t)).unwrap(), "still");
    // The control service.
    let st = rt
        .block_on(ControlClient::new(l.channel.clone()).status(imt_ipc::pb::StatusRequest {}))
        .unwrap()
        .into_inner();
    assert_eq!((st.pid, st.name.as_str()), (l.process.pid(), "fake child"));
    assert!(st.rss_bytes > 0);
    rt.block_on(ControlClient::new(l.channel.clone()).shutdown(imt_ipc::pb::ShutdownRequest {}))
        .unwrap();
    let started = Instant::now();
    while l.process.try_exit().is_none() && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(l.process.try_exit().is_some(), "the child exits after Shutdown");
}

#[test]
fn a_wrong_token_is_rejected() {
    let rt = rt();
    let l = rt.block_on(launch(&spec(&[]))).unwrap();
    let address = l.endpoint.address.clone();
    let channel = rt.block_on(imt_ipc::client::connect_channel(address)).unwrap();
    let wrong = imt_ipc::client::authenticated(channel.clone(), "0000").unwrap();
    let e = rt
        .block_on(echo(&wrong, &l.process, "x", "", 0, Duration::from_secs(5)))
        .unwrap_err();
    assert_eq!(e, IpcError::Protocol("the fake child rejected the session token".into()));
    // No token at all.
    let s = rt.block_on(EchoClient::new(channel).echo(EchoRequest::default())).unwrap_err();
    assert_eq!(s.code(), tonic::Code::Unauthenticated);
    // The right one still works.
    assert!(rt
        .block_on(echo(&l.channel, &l.process, "x", "", 0, Duration::from_secs(5)))
        .is_ok());
}

#[test]
fn deadlines_are_enforced() {
    let rt = rt();
    let l = rt.block_on(launch(&spec(&[]))).unwrap();
    let started = Instant::now();
    let e = rt
        .block_on(echo(&l.channel, &l.process, "x", "", 5_000, Duration::from_millis(200)))
        .unwrap_err();
    assert_eq!(e, IpcError::Timeout);
    assert!(started.elapsed() < Duration::from_secs(2));
}

#[test]
fn a_crash_is_reported_with_the_exit_status() {
    let rt = rt();
    let l = rt.block_on(launch(&spec(&[]))).unwrap();
    let e = rt
        .block_on(echo(&l.channel, &l.process, "", "abort", 0, Duration::from_secs(5)))
        .unwrap_err();
    assert!(matches!(&e, IpcError::Died(m) if m.contains("the fake child")), "{e:?}");
    assert!(l.process.exited());
}

#[test]
fn children_that_die_or_never_listen_fail_the_launch() {
    let rt = rt();
    let e = rt.block_on(launch(&spec(&["--exit-now"]))).err().unwrap();
    assert!(matches!(&e, IpcError::Died(m) if m.contains("exited while starting")), "{e:?}");
    let mut s = spec(&["--no-listen"]);
    s.connect_window = Duration::from_millis(300);
    let e = rt.block_on(launch(&s)).err().unwrap();
    assert!(matches!(&e, IpcError::Died(m) if m.contains("did not start listening")), "{e:?}");
    let missing = SpawnSpec::new("/definitely/not/here", "the missing child");
    let e = rt.block_on(launch(&missing)).err().unwrap();
    assert!(
        matches!(&e, IpcError::Spawn(m) if m.contains("cannot start the missing child")),
        "{e:?}"
    );
}

#[test]
fn a_child_exits_when_its_lifeline_closes() {
    // What happens when IMT dies: the child's stdin closes and it exits by itself (no orphan).
    let rt = rt();
    let pidfile = tempfile::NamedTempFile::new().unwrap();
    let arg = format!("--write-pid={}", pidfile.path().display());
    let l = rt.block_on(launch(&spec(&[&arg]))).unwrap();
    assert_eq!(std::fs::read_to_string(pidfile.path()).unwrap(), l.process.pid().to_string());
    l.process.close_lifeline();
    let started = Instant::now();
    while l.process.try_exit().is_none() && started.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(l.process.try_exit().is_some(), "the child must exit when stdin closes");
    // `close` on an exited child is immediate.
    l.process.close(Duration::from_secs(1));
}

#[test]
fn a_child_that_ignores_the_lifeline_is_killed_by_close() {
    let rt = rt();
    let mut s = spec(&["--no-listen"]);
    s.connect_window = Duration::from_millis(100);
    // A child that never listens: start it directly.
    let p = ChildProcess::spawn(&s).unwrap();
    p.send_handshake(&imt_ipc::endpoint::Handshake {
        address: "unused".into(),
        token: "t".into(),
    })
    .unwrap();
    let started = Instant::now();
    p.close(Duration::from_millis(200));
    assert!(p.exited());
    assert!(started.elapsed() < Duration::from_secs(3));
    // Writing to a dead child is a crash.
    assert!(matches!(
        p.send_handshake(&imt_ipc::endpoint::Handshake {
            address: "a".into(),
            token: "t".into()
        }),
        Err(IpcError::Died(_))
    ));
    drop(rt);
}

struct Conn(Launched);

impl Connection for Conn {
    fn pid(&self) -> Option<u32> {
        Some(self.0.process.pid())
    }
    fn kill(&self) {
        self.0.process.kill();
    }
}

#[test]
fn the_supervisor_restarts_a_crashed_child_with_a_new_process_then_quarantines() {
    let rt = std::sync::Arc::new(rt());
    let r2 = rt.clone();
    let cfg = SupervisorConfig {
        backoff_base: Duration::from_millis(10),
        ..SupervisorConfig::new("the fake child")
    };
    let sup = Supervisor::new(cfg, move || r2.block_on(launch(&spec(&[]))).map(Conn));
    let call = |action: &str| sup.call(|c| rt.block_on(echo(&c.0.channel, &c.0.process, "ok", action, 0, Duration::from_secs(5))));
    assert_eq!(call("").unwrap(), "ok");
    let first = sup.status().pid.unwrap();
    assert!(matches!(call("abort"), Err(IpcError::Died(_))));
    assert_eq!(call("").unwrap(), "ok");
    let second = sup.status().pid.unwrap();
    assert_ne!(first, second, "a fresh process after a crash");
    // A hang counts too.
    let hang = sup.call(|c| rt.block_on(echo(&c.0.channel, &c.0.process, "", "", 10_000, Duration::from_millis(100))));
    assert_eq!(hang.unwrap_err(), IpcError::Timeout);
    assert!(matches!(call("abort"), Err(IpcError::Died(_))));
    let st = sup.status();
    assert_eq!(st.state, ChildState::Quarantined, "{st:?}");
    assert!(matches!(call(""), Err(IpcError::Spawn(m)) if m.contains("failed 3 times")));
    sup.reset();
    assert_eq!(call("").unwrap(), "ok");
}
