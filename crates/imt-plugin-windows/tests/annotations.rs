//! The Windows annotators end to end: records with the EVTX importer's field
//! shape (Provider, EventId, Computer, param1/param2) are written as CLEF, so
//! the workspace imports them with a built-in importer and the plugin's
//! annotators run after the import and again after a live-tail append.

use imt_core::api::Severity;
use imt_core::plugin::annotate::AnnotationsRequest;
use imt_core::{LogStore, PluginRegistry, Profile, Workspace};
use std::io::Write;
use std::path::PathBuf;

struct Dir(PathBuf);
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn line(t: &str, provider: &str, id: u32, extra: &str) -> String {
    format!(
        "{{\"@t\":\"2026-10-14T{t}Z\",\"@mt\":\"{provider} {id}\",\"Provider\":\"{provider}\",\"EventId\":{id},\"Computer\":\"WEB01\"{extra}}}\n"
    )
}

fn ms(t: &str) -> i64 {
    chrono::DateTime::parse_from_rfc3339(&format!("2026-10-14T{t}Z"))
        .unwrap()
        .timestamp_millis()
}

#[test]
fn reboots_and_services_become_spans_and_follow_appends() {
    let dir = Dir(std::env::temp_dir().join(format!("imt-win-annot-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let file = dir.0.join("system.clef");
    let mut text = String::new();
    text += &line("10:00:00", "App", 1, "");
    text += &line("10:05:00", "EventLog", 6006, "");
    text += &line("10:05:30", "Microsoft-Windows-Kernel-General", 13, "");
    text += &line("10:08:00", "Microsoft-Windows-Kernel-General", 12, "");
    text += &line("10:08:05", "EventLog", 6005, "");
    text += &line(
        "10:09:00",
        "Service Control Manager",
        7036,
        ",\"param1\":\"Print Spooler\",\"param2\":\"running\"",
    );
    text += &line("10:30:00", "Service Control Manager", 7034, ",\"param1\":\"Print Spooler\"");
    text += &line(
        "10:40:00",
        "Service Control Manager",
        7036,
        ",\"param1\":\"W3SVC\",\"param2\":\"running\"",
    );
    text += &line("11:00:00", "App", 2, "");
    std::fs::write(&file, &text).unwrap();

    let mut registry = PluginRegistry::new();
    registry.install(&imt_plugin_windows::WindowsPlugin).unwrap();
    let mut ws = Workspace::with_registry(Profile::default_profile(), registry).unwrap();
    ws.open(std::slice::from_ref(&dir.0)).unwrap();

    let all = ws.annotations(&AnnotationsRequest::default());
    let reboot = all.iter().find(|a| a.annotator == "windows.reboots").expect("a reboot span");
    assert_eq!((reboot.from, reboot.to), (ms("10:05:00"), ms("10:08:00")));
    assert_eq!(reboot.kind, "net.decryptology.windows/downtime");
    assert_eq!(reboot.severity, Severity::Info);
    assert_eq!(reboot.on.as_ref().unwrap().value, "WEB01");
    assert!(reboot.instance.is_none());
    let spooler = all.iter().find(|a| a.label.starts_with("Print Spooler")).unwrap();
    assert_eq!(
        (spooler.from, spooler.to, spooler.severity),
        (ms("10:09:00"), ms("10:30:00"), Severity::Warning)
    );
    let w3 = all.iter().find(|a| a.label.starts_with("W3SVC")).unwrap();
    assert_eq!(
        (w3.from, w3.to),
        (ms("10:40:00"), ms("11:00:00")),
        "open spans end at the newest event"
    );
    assert_eq!(all.len(), 3);
    // Time filtering.
    let early = ws.annotations(&AnnotationsRequest {
        to: Some(ms("10:06:00")),
        ..Default::default()
    });
    assert_eq!(early.len(), 1);

    // A crash and reboot are appended: the tail refresh re-runs the annotators.
    let mut f = std::fs::OpenOptions::new().append(true).open(&file).unwrap();
    f.write_all(line("11:30:00", "EventLog", 6005, "").as_bytes()).unwrap();
    f.write_all(line("11:30:10", "EventLog", 6008, "").as_bytes()).unwrap();
    f.write_all(line("11:30:11", "Microsoft-Windows-Kernel-Power", 41, "").as_bytes())
        .unwrap();
    drop(f);
    let report = ws.refresh_tail(std::slice::from_ref(&file)).unwrap();
    assert_eq!(report.appended, 3);
    let all = ws.annotations(&AnnotationsRequest::default());
    let crash = all
        .iter()
        .filter(|a| a.annotator == "windows.reboots")
        .find(|a| a.to == ms("11:30:00"))
        .expect("a downtime span after the append");
    assert_eq!(crash.from, ms("11:00:00"), "from the last event before the boot");
    assert_eq!(crash.severity, Severity::Critical);
    assert!(crash.label.starts_with("Unexpected shutdown"), "{}", crash.label);

    // The marker kinds are in the summary with their owner.
    let s = ws.summary();
    let k = s
        .marker_kinds
        .iter()
        .find(|k| k.kind == "net.decryptology.windows/kernel-power")
        .unwrap();
    assert_eq!(k.owner, "net.decryptology.windows");
    assert!(s.marker_kinds.iter().any(|k| k.kind == "error" && k.owner == "builtin"));

    // Clearing forgets them; the store trait exposes them too.
    let store: &dyn LogStore = &ws;
    assert!(!store.annotations(&AnnotationsRequest::default()).is_empty());
    ws.clear();
    assert!(ws.annotations(&AnnotationsRequest::default()).is_empty());
}
