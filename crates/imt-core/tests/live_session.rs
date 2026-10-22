//! `LiveSession` (alerts, mutes, the tail status) over the ports, with the in-memory workspace as the store: the
//! same behaviour the DuckDB store's tests check, without a database.

use imt_core::api::Severity;
use imt_core::live::LiveSession;
use imt_core::model::Level;
use imt_core::ports::{Ingest, Tail, TailAppend};
use imt_core::workspace::{AlertTarget, TailFileState, TailOptions, TailReport, WatchMode, WorkspaceStore};
use imt_core::{Profile, Workspace};
use std::io::Write;
use std::path::{Path, PathBuf};

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("imt-live-session-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(std::fs::canonicalize(p).unwrap())
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn line(s: u32, level: &str, machine: &str, msg: &str) -> String {
    format!(
        "2026-10-14 10:{:02}:{:02}.000 +00:00 [{level}] {msg} {{\"MachineName\": \"{machine}\"}}\n",
        s / 60,
        s % 60
    )
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).create(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn open_store(profile: &str, dir: &Dir, text: &str) -> (WorkspaceStore, PathBuf) {
    let path = dir.0.join("app.log");
    std::fs::write(&path, text).unwrap();
    let mut ws = Workspace::new(Profile::from_toml(profile).unwrap()).unwrap();
    ws.open(std::slice::from_ref(&dir.0)).unwrap();
    (WorkspaceStore::new(ws), path)
}

fn refresh(store: &WorkspaceStore, session: &mut LiveSession, path: &Path) -> TailReport {
    let appended = store.append_tail(&[path.to_path_buf()]).unwrap();
    session.after_refresh(
        store,
        &appended,
        &TailOptions {
            detect_every_ms: 0,
            ..TailOptions::default()
        },
    )
}

#[test]
fn rules_cooldowns_mutes_and_silences() {
    let toml = r#"
[live]
silence_after = "20s"
cooldown = "1h"
[[live.rule]]
name = "payment errors"
when = 'level = "Error" and message contains "Payment"'
threshold = 3
window = "1m"
severity = "critical"
"#;
    let dir = Dir::new("rules");
    let mut base = String::new();
    for i in 0..60 {
        base += &line(i, "INF", "WEB01", "tick");
        if i < 30 {
            base += &line(i, "INF", "WEB02", "tock");
        }
    }
    let (store, path) = open_store(toml, &dir, &base);
    let mut session = LiveSession::default();
    session.start(&store).unwrap();
    append(&path, &line(60, "INF", "WEB01", "tick"));
    let first = refresh(&store, &mut session, &path);
    assert!(first.alerts.iter().all(|a| a.kind != "rule"), "{:?}", first.alerts);
    assert_eq!(first.appended, 1);
    let silent: Vec<_> = first.alerts.iter().filter(|a| a.kind == "silence").collect();
    assert!(silent.is_empty(), "the first evaluation only sets now: {silent:?}");
    append(
        &path,
        &(61..64).map(|i| line(i, "ERR", "WEB01", "Payment failed")).collect::<String>(),
    );
    let r = refresh(&store, &mut session, &path);
    let rule = r.alerts.iter().find(|a| a.kind == "rule").expect("a rule alert");
    assert_eq!((rule.rule.as_deref(), rule.severity), (Some("payment errors"), Severity::Critical));
    assert_eq!(r.alerts_raised, r.alerts.len());
    // The cooldown: the same rule does not fire again.
    append(&path, &line(64, "ERR", "WEB01", "Payment failed"));
    assert!(refresh(&store, &mut session, &path).alerts.iter().all(|a| a.kind != "rule"));

    // Muted alerts are not raised, and a mute does not start the cooldown.
    let mut muted = LiveSession::default();
    muted.start(&store).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    muted.mute(AlertTarget::Rule("payment errors".into()), now + 60_000).unwrap();
    assert!(muted.mute(AlertTarget::Rule("".into()), now + 60_000).is_err());
    assert!(muted.mute(AlertTarget::Rule("x".into()), now - 1).is_err());
    append(
        &path,
        &(65..69).map(|i| line(i, "ERR", "WEB01", "Payment failed")).collect::<String>(),
    );
    assert!(refresh(&store, &mut muted, &path).alerts.iter().all(|a| a.kind != "rule"));
    assert!(muted.unmute(&AlertTarget::Rule("payment errors".into())));
    append(
        &path,
        &(69..73).map(|i| line(i, "ERR", "WEB01", "Payment failed")).collect::<String>(),
    );
    assert!(
        refresh(&store, &mut muted, &path).alerts.iter().any(|a| a.kind == "rule"),
        "unmuted, it fires"
    );
}

#[test]
fn a_burst_of_errors_is_a_spike_and_the_wall_clock_can_be_now() {
    let dir = Dir::new("spike");
    let base: String = (0..600).map(|i| line(i, "INF", "WEB01", "steady")).collect();
    let (store, path) = open_store("", &dir, &base);
    let mut session = LiveSession::default();
    session.start(&store).unwrap();
    append(&path, &line(600, "INF", "WEB01", "steady"));
    refresh(&store, &mut session, &path);
    append(
        &path,
        &(0..40).map(|_| line(601, "ERR", "WEB01", "Database down")).collect::<String>(),
    );
    let r = refresh(&store, &mut session, &path);
    let spike = r.alerts.iter().find(|a| a.kind == "errorSpike").expect("an error spike");
    assert_eq!(
        (spike.severity, spike.evidence_query.levels.clone()),
        (Severity::Critical, vec![Level::Error, Level::Fatal])
    );
    assert_eq!(r.newest_ts, store.read().summary().to);

    // With the wall clock as now, 2026 events are old: nothing in the window.
    let dir2 = Dir::new("wall");
    let (store2, path2) = open_store("[live]\nclock = \"wall\"", &dir2, &line(0, "INF", "WEB01", "a"));
    let mut wall = LiveSession::default();
    wall.start(&store2).unwrap();
    append(&path2, &line(1, "ERR", "WEB01", "b"));
    assert!(refresh(&store2, &mut wall, &path2).alerts.is_empty());
}

#[test]
fn file_changes_become_states_and_the_status_lists_rates() {
    let dir = Dir::new("status");
    let (store, path) = open_store("", &dir, &(0..3).map(|i| line(i, "INF", "A", "x")).collect::<String>());
    let mut session = LiveSession::default();
    session.start(&store).unwrap();
    let report = session.after_refresh(
        &store,
        &TailAppend {
            added: vec![1],
            truncated: vec![2],
            reimported: vec![3],
            deleted: vec![4],
            sources_changed: vec![1],
            appended: 5,
            held: Vec::new(),
        },
        &TailOptions::default(),
    );
    let states: Vec<_> = report.file_states.iter().map(|c| (c.source_id, c.state.clone())).collect();
    assert_eq!(
        states,
        [
            (1, TailFileState::Tailing),
            (2, TailFileState::Truncated),
            (3, TailFileState::Whole),
            (4, TailFileState::Deleted)
        ]
    );
    append(&path, &(3..6).map(|i| line(i, "INF", "A", "x")).collect::<String>());
    refresh(&store, &mut session, &path);
    let st = session.status(&store, |_| WatchMode::Events).unwrap();
    assert_eq!((st.totals.files, st.totals.tailing, st.totals.records), (1, 1, 6));
    let f = &st.files[0];
    assert_eq!((f.state.clone(), f.mode), (TailFileState::Tailing, WatchMode::Events));
    assert_eq!(store.tail_files().unwrap().len(), 1);
    std::fs::remove_file(&path).unwrap();
    assert_eq!(
        session.status(&store, |_| WatchMode::Off).unwrap().files[0].state,
        TailFileState::Deleted
    );
}

#[test]
fn a_silent_instance_raises_one_silence_alert_while_time_moves_on() {
    let toml = "[live]\nsilence_after = \"20s\"\ncooldown = \"1h\"\n";
    let dir = Dir::new("silence");
    let mut base = String::new();
    for i in 0..31 {
        base += &line(i, "INF", "WEB01", "tick");
        if i <= 20 {
            base += &line(i, "INF", "WEB02", "tick");
        }
    }
    let (store, path) = open_store(toml, &dir, &base);
    let mut session = LiveSession::default();
    session.start(&store).unwrap();
    // The first refresh only learns what time it is; WEB02 has been silent for 11 s, less than the limit.
    append(&path, &line(31, "INF", "WEB01", "tick"));
    assert!(refresh(&store, &mut session, &path).alerts.iter().all(|a| a.kind != "silence"));
    // Time moves on past the limit: WEB02 is reported once, with evidence for its lane.
    append(&path, &line(45, "INF", "WEB01", "tick"));
    let r = refresh(&store, &mut session, &path);
    let silence: Vec<_> = r.alerts.iter().filter(|a| a.kind == "silence").collect();
    assert_eq!(silence.len(), 1, "{:?}", r.alerts);
    let a = silence[0];
    assert_eq!(a.instance.as_deref(), Some("WEB02"));
    assert_eq!(a.severity, Severity::Warning);
    assert!(a.message.starts_with("WEB02 has not logged for 25"), "{}", a.message);
    assert_eq!(a.evidence_query.groups, ["WEB02"]);
    assert_eq!(a.to - a.from, 25_000);
    assert!(a.id.starts_with("silence:WEB02:"));
    // The cooldown keeps it from being raised again on the next refresh; unmuting nothing changes that.
    append(&path, &line(50, "INF", "WEB01", "tick"));
    assert!(refresh(&store, &mut session, &path).alerts.iter().all(|a| a.kind != "silence"));
}

#[test]
fn an_empty_store_has_no_newest_time_and_rules_without_a_start_count_the_whole_window() {
    // Nothing loaded: no "now", so no alerts and no newest time.
    let store = WorkspaceStore::new(Workspace::new(Profile::default_profile()).unwrap());
    let mut session = LiveSession::default();
    let r = session.after_refresh(&store, &TailAppend::default(), &TailOptions::default());
    assert!(r.alerts.is_empty() && r.newest_ts.is_none());

    // A session that was never started has no baseline: a rule counts everything in its window.
    let toml = "[[live.rule]]\nname = 'errors'\nwhen = 'level = \"Error\"'\nthreshold = 2\nwindow = \"1m\"\nseverity = \"warning\"\n";
    let dir = Dir::new("nobaseline");
    let history: String = (0..3).map(|i| line(i, "ERR", "WEB01", "boom")).collect();
    let (store, path) = open_store(toml, &dir, &history);
    let mut never_started = LiveSession::default();
    append(&path, &line(5, "INF", "WEB01", "tick"));
    let r = refresh(&store, &mut never_started, &path);
    let alert = r
        .alerts
        .iter()
        .find(|a| a.rule.as_deref() == Some("errors"))
        .expect("history counts without a baseline");
    assert_eq!(alert.severity, Severity::Warning);
    assert!(alert.message.contains("3 matching events"), "{}", alert.message);
}

#[test]
fn many_refreshes_keep_a_bounded_rate_history() {
    let dir = Dir::new("rate");
    let (store, path) = open_store("", &dir, &line(0, "INF", "A", "x"));
    let mut session = LiveSession::default();
    session.start(&store).unwrap();
    for i in 1..=80 {
        append(&path, &line(i, "INF", "A", "x"));
        refresh(&store, &mut session, &path);
        session.status(&store, |_| WatchMode::Events).unwrap();
    }
    let st = session.status(&store, |_| WatchMode::Events).unwrap();
    assert_eq!(st.totals.records, 81);
    // 64 samples of one line each within the window: 6.4 lines per second.
    assert!((st.files[0].lines_per_sec - 6.4).abs() < 0.2, "{}", st.files[0].lines_per_sec);
    assert_eq!(st.files[0].size.max(1), st.files[0].size);
}
