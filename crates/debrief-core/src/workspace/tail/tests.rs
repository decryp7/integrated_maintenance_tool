//! Live tail through a workspace: per-file state changes in reports, stable
//! alert ids, engine-side mutes (with an injected clock) and option clamping.

use super::*;
use crate::analysis::AnomalyKind;
use crate::Profile;
use std::io::Write;
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::Arc;

struct Dir(PathBuf);

impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("debrief-tail-unit-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(std::fs::canonicalize(p).unwrap())
    }
    fn append(&self, name: &str, text: &str) -> PathBuf {
        let p = self.0.join(name);
        let mut f = std::fs::OpenOptions::new().append(true).create(true).open(&p).unwrap();
        f.write_all(text.as_bytes()).unwrap();
        p
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn line(s: u32, level: &str, msg: &str) -> String {
    format!(
        "2026-10-14 10:{:02}:{:02}.000 +00:00 [{level}] {msg} {{\"MachineName\": \"WEB01\"}}\n",
        s / 60,
        s % 60
    )
}

const RULE: &str = r#"
[live]
cooldown = "1ms"
[[live.rule]]
name = "payment errors"
when = 'level = "Error"'
threshold = 1
window = "1h"
"#;

fn workspace(d: &Dir, profile: &str) -> (Workspace, PathBuf, Arc<AtomicI64>) {
    let p = d.append("app.log", &line(0, "INF", "start"));
    let mut ws = Workspace::new(Profile::from_toml(profile).unwrap()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let t = Arc::new(AtomicI64::new(1_000_000));
    let c = t.clone();
    ws.tail.mutes = AlertMutes::with_clock(Arc::new(move || c.load(Ordering::SeqCst)));
    (ws, p, t)
}

fn rule_alerts(ws: &mut Workspace, d: &Dir, p: &Path, s: u32) -> usize {
    d.append("app.log", &line(s, "ERR", "Payment failed"));
    let r = ws.refresh_tail(&[p.to_path_buf()]).unwrap();
    r.alerts.iter().filter(|a| a.kind == "rule").count()
}

#[test]
fn mutes_suppress_rule_alerts_until_they_expire_or_are_lifted() {
    let d = Dir::new("mute-rule");
    let (mut ws, p, t) = workspace(&d, RULE);
    assert_eq!(rule_alerts(&mut ws, &d, &p, 1), 1);
    // Muted: not raised.
    let rule = AlertTarget::Rule("payment errors".into());
    ws.mute_alerts(rule.clone(), 1_060_000).unwrap();
    assert_eq!(rule_alerts(&mut ws, &d, &p, 2), 0);
    // A kind mute does not silence a rule.
    ws.mute_alerts(AlertTarget::Kind(AnomalyKind::ErrorSpike), 1_060_000).unwrap();
    // Expired: raised again.
    t.store(1_060_000, Ordering::SeqCst);
    assert_eq!(rule_alerts(&mut ws, &d, &p, 3), 1);
    // Lifted early: raised again.
    ws.mute_alerts(rule.clone(), 1_120_000).unwrap();
    assert_eq!(rule_alerts(&mut ws, &d, &p, 4), 0);
    assert!(ws.unmute_alerts(&rule));
    assert!(!ws.unmute_alerts(&rule));
    assert_eq!(rule_alerts(&mut ws, &d, &p, 5), 1);
    // Invalid mutes are refused; mutes survive a reload.
    assert!(ws.mute_alerts(rule.clone(), 999).is_err());
    ws.mute_alerts(rule.clone(), 1_120_000).unwrap();
    ws.reload().unwrap();
    assert!(ws.tail.mutes.covers("rule", Some("payment errors")));
    // Through the store trait.
    let mut store: Box<dyn crate::LogStore> = Box::new(ws);
    store.unmute_alerts(&rule);
    store.mute_alerts(rule, 1_100_000).unwrap();
}

#[test]
fn mutes_suppress_detector_kinds() {
    let d = Dir::new("mute-kind");
    let base: String = (0..600).map(|i| line(i, "INF", "steady")).collect();
    let p = d.append("app.log", &base);
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    ws.set_tail_options(TailOptions {
        detect_every_ms: 0,
        ..TailOptions::default()
    });
    ws.mute_alerts(AlertTarget::Kind(AnomalyKind::ErrorSpike), super::wall_ms() + 60_000)
        .unwrap();
    d.append("app.log", &line(600, "INF", "steady"));
    ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    let burst: String = (0..40).map(|_| line(601, "ERR", "Database down")).collect();
    d.append("app.log", &burst);
    let r = ws.refresh_tail(std::slice::from_ref(&p)).unwrap();
    assert!(r.alerts.iter().all(|a| a.kind != "errorSpike"), "{:?}", r.alerts);
    // The muted spike did not start a cooldown: lifted, the next burst is raised.
    ws.unmute_alerts(&AlertTarget::Kind(AnomalyKind::ErrorSpike));
    let burst: String = (0..80).map(|_| line(602, "ERR", "Database down")).collect();
    d.append("app.log", &burst);
    let r = ws.refresh_tail(&[p]).unwrap();
    let spike = r.alerts.iter().find(|a| a.kind == "errorSpike").expect("error spike");
    assert!(spike.id.starts_with("errorSpike:"), "{}", spike.id);
}

#[test]
fn alert_ids_are_stable_per_kind_who_and_cooldown_bucket() {
    let d = Dir::new("ids");
    let profile = RULE.replace("cooldown = \"1ms\"", "cooldown = \"1h\"");
    let (mut ws, p, _) = workspace(&d, &profile);
    d.append("app.log", &line(1, "ERR", "Payment failed"));
    let r = ws.refresh_tail(&[p]).unwrap();
    let a = r.alerts.iter().find(|a| a.kind == "rule").unwrap();
    let bucket = a.from.div_euclid(3_600_000);
    assert_eq!(a.id, format!("rule:payment errors:{bucket}"));
    assert_eq!(serde_json::to_value(a).unwrap()["id"], a.id.as_str());
}

#[test]
fn reports_list_files_whose_state_changed() {
    let d = Dir::new("states");
    let a = d.append("a.log", &line(0, "INF", "a"));
    d.append("b.log", &line(0, "INF", "b"));
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    // Appending keeps the state: nothing listed.
    d.append("a.log", &line(1, "INF", "a"));
    let r = ws.refresh_tail(std::slice::from_ref(&a)).unwrap();
    assert!(r.file_states.is_empty(), "{:?}", r.file_states);
    // Deleted: listed once with its new state.
    std::fs::remove_file(&a).unwrap();
    let r = ws.refresh_tail(std::slice::from_ref(&a)).unwrap();
    assert_eq!(r.file_states.len(), 1);
    assert_eq!(r.file_states[0].state, TailFileState::Deleted);
    assert_eq!(Some(r.file_states[0].source_id), r.deleted.first().copied());
    let v = serde_json::to_value(&r).unwrap();
    assert_eq!(v["fileStates"][0]["state"]["kind"], "deleted");
    assert!(!r.is_empty());
    assert!(ws.refresh_tail(std::slice::from_ref(&a)).unwrap().file_states.is_empty());
    // A new file is listed as tailing.
    let c = d.append("c.log", &line(2, "INF", "c"));
    let r = ws.refresh_tail(&[c]).unwrap();
    assert_eq!(r.file_states.len(), 1);
    assert_eq!(r.file_states[0].state, TailFileState::Tailing);
    assert_eq!(r.added, [r.file_states[0].source_id]);
}

#[test]
fn detect_every_is_clamped() {
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.set_tail_options(TailOptions {
        detect_every_ms: u64::MAX,
        ..TailOptions::default()
    });
    assert_eq!(ws.tail_options().detect_every_ms, MAX_DETECT_EVERY_MS);
    ws.set_tail_options(TailOptions {
        detect_every_ms: 0,
        ..TailOptions::default()
    });
    assert_eq!(ws.tail_options().detect_every_ms, 0);
}
