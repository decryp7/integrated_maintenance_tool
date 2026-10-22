//! Live alerts and the tail status over the store's ports ([`imt_core::live::LiveSession`]): rules, silences and
//! detectors raise their alerts once per cooldown, mutes silence them, history opened with the files does not alert,
//! and the status lists each followed file with its rate.

mod common;

use common::{store_with, TempDir};
use imt_core::live::LiveSession;
use imt_core::ports::{Query, Tail};
use imt_core::workspace::{AlertTarget, TailFileState, TailOptions, WatchMode};
use imt_core::Profile;
use imt_store_duck::DuckStore;
use std::io::Write;
use std::path::Path;

/// One CLEF event `s` seconds into the hour.
fn line(s: u32, level: &str, machine: &str, msg: &str) -> String {
    format!(
        "{{\"@t\":\"2026-10-14T10:{:02}:{:02}.000Z\",\"@l\":\"{level}\",\"@m\":\"{msg}\",\"MachineName\":\"{machine}\"}}\n",
        s / 60,
        s % 60
    )
}

fn append(path: &Path, text: &str) {
    let mut f = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    f.write_all(text.as_bytes()).unwrap();
}

fn options() -> TailOptions {
    TailOptions {
        detect_every_ms: 0,
        ..TailOptions::default()
    }
}

fn refresh(store: &DuckStore, session: &mut LiveSession, file: &Path) -> imt_core::workspace::TailReport {
    let appended = imt_core::ports::Ingest::append_tail(store, &[file.to_path_buf()]).unwrap();
    session.after_refresh(store, &appended, &options())
}

fn store_with_profile(toml: &str, file: &str, text: &str) -> (TempDir, TempDir, DuckStore, std::path::PathBuf) {
    let logs = TempDir::new("live-logs");
    let path = logs.0.join(file);
    std::fs::write(&path, text).unwrap();
    let dir = TempDir::new("live-store");
    let (store, _) = store_with(&dir, Profile::from_toml(toml).unwrap(), std::slice::from_ref(&logs.0)).unwrap();
    (logs, dir, store, path)
}

#[test]
fn rules_fire_once_per_cooldown_and_mutes_silence_them() {
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
    let history: String = (0..60).map(|i| line(i, "Information", "WEB01", &format!("tick {i}"))).collect();
    let (_logs, _dir, store, path) = store_with_profile(toml, "app.clef", &history);
    let mut session = LiveSession::default();
    session.start(&store).unwrap();
    // Errors that were already there when live tail started do not alert; new ones do.
    append(&path, &line(60, "Information", "WEB01", "tick"));
    let first = refresh(&store, &mut session, &path);
    assert!(first.alerts.iter().all(|a| a.kind != "rule"), "{:?}", first.alerts);
    append(
        &path,
        &(61..64).map(|i| line(i, "Error", "WEB01", "Payment failed")).collect::<String>(),
    );
    let r = refresh(&store, &mut session, &path);
    let rule = r.alerts.iter().find(|a| a.kind == "rule").expect("a rule alert");
    assert_eq!(
        (rule.rule.as_deref(), rule.severity),
        (Some("payment errors"), imt_core::api::Severity::Critical)
    );
    assert_eq!((r.alerts_raised, r.appended), (r.alerts.len(), 3));
    assert!(rule.id.starts_with("rule:payment errors:"));
    // The cooldown: the same rule does not fire again.
    append(&path, &line(64, "Error", "WEB01", "Payment failed"));
    assert!(refresh(&store, &mut session, &path).alerts.iter().all(|a| a.kind != "rule"));

    // Muted, a rule stays quiet; unmuted (and past the cooldown it would fire again).
    let mut muted = LiveSession::default();
    muted.start(&store).unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_millis() as i64;
    muted.mute(AlertTarget::Rule("payment errors".into()), now + 60_000).unwrap();
    append(
        &path,
        &(65..69).map(|i| line(i, "Error", "WEB01", "Payment failed")).collect::<String>(),
    );
    assert!(refresh(&store, &mut muted, &path).alerts.iter().all(|a| a.kind != "rule"));
    assert!(muted.mute(AlertTarget::Rule(" ".into()), now + 60_000).is_err());
    assert!(muted.unmute(&AlertTarget::Rule("payment errors".into())));
    assert!(!muted.unmute(&AlertTarget::Rule("payment errors".into())));
}

#[test]
fn an_instance_going_quiet_raises_a_silence_and_a_burst_of_errors_a_spike() {
    let mut base = String::new();
    for i in 0..10 {
        base += &line(i, "Information", "WEB01", "a");
        base += &line(i, "Information", "WEB02", "b");
    }
    let (_logs, _dir, store, path) = store_with_profile("[live]\nsilence_after = \"20s\"", "app.clef", &base);
    // Instances are told apart by the store's instance key.
    let key = store.summary().unwrap().default_group_by;
    assert!(!key.is_empty(), "the samples name an instance key");
    let mut session = LiveSession::default();
    session.start(&store).unwrap();
    append(&path, &line(10, "Information", "WEB01", "a"));
    assert!(refresh(&store, &mut session, &path).alerts.is_empty());
    append(&path, &(11..40).map(|i| line(i, "Information", "WEB01", "a")).collect::<String>());
    let r = refresh(&store, &mut session, &path);
    let silence: Vec<_> = r.alerts.iter().filter(|a| a.kind == "silence").collect();
    assert_eq!(silence.len(), 1, "{:?}", r.alerts);
    assert!(silence[0].instance.as_deref().is_some_and(|i| i.contains("WEB02")), "{silence:?}");

    let steady: String = (0..600).map(|i| line(i, "Information", "WEB01", "steady")).collect();
    let (_l2, _d2, store2, path2) = store_with_profile("", "app.clef", &steady);
    let mut spikes = LiveSession::default();
    spikes.start(&store2).unwrap();
    append(&path2, &line(600, "Information", "WEB01", "steady"));
    refresh(&store2, &mut spikes, &path2);
    append(
        &path2,
        &(0..40).map(|_| line(601, "Error", "WEB01", "Database down")).collect::<String>(),
    );
    let r = refresh(&store2, &mut spikes, &path2);
    let spike = r.alerts.iter().find(|a| a.kind == "errorSpike").expect("an error spike");
    assert_eq!(spike.severity, imt_core::api::Severity::Critical);
    assert_eq!(
        spike.evidence_query.levels,
        [imt_core::model::Level::Error, imt_core::model::Level::Fatal]
    );
}

#[test]
fn the_status_lists_followed_files_with_their_rate_and_watch_mode() {
    let logs = TempDir::new("live-status-logs");
    std::fs::create_dir_all(logs.0.join("logs")).unwrap();
    let a = logs.0.join("logs/app.clef.1");
    std::fs::write(&a, (0..3).map(|i| line(i, "Information", "A", "x")).collect::<String>()).unwrap();
    std::fs::write(logs.0.join("logs/app.clef"), line(3, "Information", "A", "x")).unwrap();
    let dir = TempDir::new("live-status-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    let mut session = LiveSession::default();
    session.start(&store).unwrap();
    let before = session.status(&store, |_| WatchMode::Polling).unwrap();
    assert_eq!((before.totals.files, before.totals.records, before.totals.tailing), (2, 4, 2));
    append(&a, &(4..6).map(|i| line(i, "Information", "A", "x")).collect::<String>());
    refresh(&store, &mut session, &a);
    let st = session.status(&store, |_| WatchMode::Polling).unwrap();
    let f = st.files.iter().find(|f| f.name.ends_with("app.clef.1")).unwrap();
    assert_eq!(
        (f.state.clone(), f.mode, f.offset == f.size),
        (TailFileState::Tailing, WatchMode::Polling, true)
    );
    assert!(f.lines_per_sec > 0.0 && f.last_event.is_some(), "{f:?}");
    assert_eq!((st.totals.files, st.totals.records), (2, 6));
    // A followed file that disappears is shown as deleted; its events stay.
    std::fs::remove_file(&a).unwrap();
    let st = session.status(&store, |_| WatchMode::Off).unwrap();
    assert_eq!(
        st.files.iter().find(|f| f.name.ends_with("app.clef.1")).unwrap().state,
        TailFileState::Deleted
    );
    assert_eq!(st.totals.records, 6);
    let _ = store.tail_files().unwrap();
}
