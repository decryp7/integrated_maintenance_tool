//! Capturing a real child's standard error and exit into its child log: crash vs normal vs stopped by IMT.

use imt_ipc::child_log::{self, ChildKind, ChildLogSpec, ChildRunState};
use imt_ipc::launch::{launch, ChildSpec};
use imt_ipc::peer::NoCalls;
use imt_ipc::process::{ChildProcess, SpawnSpec};
use std::sync::Arc;
use std::time::{Duration, Instant};

fn spec(args: &[&str], log: &str) -> SpawnSpec {
    let mut s = SpawnSpec::new(env!("CARGO_BIN_EXE_imt-ipc-fake-child"), "the fake child");
    s.args = args.iter().map(|a| a.to_string()).collect();
    s.endpoint_prefix = "imt-test".into();
    s.connect_window = Duration::from_secs(10);
    s.log = Some(ChildLogSpec::new(log, ChildKind::Plugin).with_version("9.9.9"));
    s
}

fn wait_for(what: &str, f: impl Fn() -> bool) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn a_crash_is_recorded_with_its_output_and_exit_code() {
    let p = ChildProcess::spawn(&spec(&["--say=boom token=s3cr3t", "--exit-now"], "capture-crash")).unwrap();
    let log = p.log().unwrap().clone();
    wait_for("the exit line", || log.status().state != ChildRunState::Running);
    let s = log.status();
    assert_eq!(s.state, ChildRunState::Crashed, "{s:?}");
    assert_eq!(s.last_exit.as_deref(), Some("exit code 3"));
    assert_eq!((s.pid, s.version.as_deref()), (Some(p.pid()), Some("9.9.9")));
    let text = log.tail_text();
    assert!(text.contains("boom token=***") && !text.contains("s3cr3t"), "{text}");
    // Stopping it afterwards does not turn the crash into a stop.
    p.kill();
    let lines = log
        .tail_text()
        .lines()
        .filter(|l| l.contains("\"Event\":\"crash\"") || l.contains("\"Event\":\"stop\""))
        .count();
    assert_eq!(lines, 1);
}

#[test]
fn a_child_stopped_by_imt_and_restarted_counts_runs() {
    let s = ChildSpec::new(spec(&["--say=ready"], "capture-stop"));
    let first = launch(&s, Arc::new(NoCalls)).unwrap();
    first.stop();
    let second = launch(&s, Arc::new(NoCalls)).unwrap();
    drop(second);
    let log = child_log::find("capture-stop").unwrap();
    let st = log.status();
    assert_eq!((st.starts, st.restarts, st.state), (2, 1, ChildRunState::Stopped), "{st:?}");
    assert_eq!(st.last_exit.as_deref(), Some("stopped by IMT"));
    let text = log.tail_text();
    assert_eq!(text.matches("\"Event\":\"stop\"").count(), 2, "{text}");
    assert!(text.contains("\"@m\":\"ready\""), "{text}");
}

#[test]
fn a_child_that_cannot_start_leaves_a_note() {
    let mut s = spec(&[], "capture-missing");
    s.exe = "/no/such/imt-child".into();
    assert!(ChildProcess::spawn(&s).is_err());
    assert!(child_log::find("capture-missing").unwrap().tail_text().contains("Could not start"));
}

#[test]
fn without_a_log_nothing_is_captured() {
    let mut s = spec(&["--exit-now"], "capture-none");
    s.log = None;
    let p = ChildProcess::spawn(&s).unwrap();
    assert!(p.log().is_none());
    p.kill();
    assert!(child_log::find("capture-none").is_none());
}
