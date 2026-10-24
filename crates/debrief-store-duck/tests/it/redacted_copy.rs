//! The redacted project copy (ADR 0041): the produced `.debriefz` is searched, byte for byte and entry by entry, for the
//! invented secrets that were in the logs; the copy still opens and holds the redacted events; sources that cannot be
//! redacted are left out and listed; a store without rules or with nothing copyable refuses with what to do.

use crate::common;

use common::{store_with, TempDir};
use debrief_core::package::ProjectManifest;
use debrief_core::ports::{PageQuery, Projects, Query, Redaction, Search, SearchRequest, StoreError};
use debrief_core::redact::copy::RedactedExportRequest;
use debrief_core::redact::{Detector, Mode, RedactCfg, RedactRuleCfg};
use debrief_core::{PluginRegistry, Profile};
use debrief_store_duck::DuckStore;
use std::io::Read;
use std::path::Path;

const SECRETS: &[&str] = &[
    "maria.example@contoso-invented.test",
    "10.77.8.9",
    "abc123SECRETtokenXYZ789",
    "hunter2-invented",
];

fn write_logs(dir: &Path) {
    std::fs::create_dir_all(dir.join("maria.example@contoso-invented.test")).unwrap();
    std::fs::write(
        dir.join("maria.example@contoso-invented.test").join("app.log"),
        "2026-10-14 10:00:00 INFO login maria.example@contoso-invented.test from 10.77.8.9\n\
         2026-10-14 10:00:05 WARN call failed Authorization: Bearer abc123SECRETtokenXYZ789\n\
         2026-10-14 10:00:09 ERROR boom\n   at Service.Run() password hunter2-invented\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("table.csv"),
        "Time,Level,Message\n2026-10-14 10:00:00,INFO,mail maria.example@contoso-invented.test\n",
    )
    .unwrap();
}

fn cfg() -> RedactCfg {
    RedactCfg {
        detectors: vec![Detector::Email, Detector::Ipv4, Detector::BearerToken],
        mode: Mode::Mask,
        fields: vec!["*".into()],
        rule: vec![RedactRuleCfg {
            name: "invented-password".into(),
            regex: "hunter2-invented".into(),
            replace: None,
        }],
        ..RedactCfg::default()
    }
}

fn request(out: &Path, cfg: RedactCfg) -> RedactedExportRequest {
    RedactedExportRequest {
        cfg,
        manifest: ProjectManifest {
            name: "shared".into(),
            // The original's profile spells a secret in its own redaction rule; the copy must not carry it.
            profile_overrides: "[redact]\n[[redact.rule]]\nname='x'\nregex='hunter2-invented'\n".into(),
            roots: vec!["/home/maria.example@contoso-invented.test/logs".into()],
            ..ProjectManifest::default()
        },
        out: out.to_path_buf(),
        ui_json: Some(r#"{"version":1,"ui":{"note":"<email> was here"},"reports":[]}"#.into()),
    }
}

fn every_entry(debriefz: &Path) -> Vec<(String, Vec<u8>)> {
    let mut zip = zip::ZipArchive::new(std::fs::File::open(debriefz).unwrap()).unwrap();
    (0..zip.len())
        .map(|i| {
            let mut f = zip.by_index(i).unwrap();
            let mut bytes = Vec::new();
            f.read_to_end(&mut bytes).unwrap();
            (f.name().unwrap().to_string(), bytes)
        })
        .collect()
}

fn contains(hay: &[u8], needle: &str) -> bool {
    hay.windows(needle.len()).any(|w| w == needle.as_bytes())
}

fn found(store: &impl Search, text: &str) -> u64 {
    store
        .search(&SearchRequest {
            text: text.into(),
            limit: 10,
        })
        .unwrap()
        .total
}

#[test]
fn the_copy_holds_no_original_secret_anywhere_and_still_opens() {
    let logs = TempDir::new("redcopy-logs");
    write_logs(&logs.0);
    let dir = TempDir::new("redcopy-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    store.wait_for_index(std::time::Duration::from_secs(60)).unwrap();
    // The original does hold them (so the searches below mean something).
    assert!(found(&store, "login") >= 1, "the index is not ready");
    assert!(found(&store, "hunter2") >= 1);
    let out_dir = TempDir::new("redcopy-out");
    let out = out_dir.0.join("shared.debriefz");

    let report = store.export_redacted_project(&request(&out, cfg())).unwrap();
    assert_eq!((report.sources, report.events), (1, 3));
    assert_eq!(report.excluded.len(), 1);
    assert!(report.excluded[0].name.ends_with("table.csv") && report.excluded[0].reason.contains("csv"));
    let hit = |rule: &str| report.hits.iter().find(|h| h.rule == rule).map_or(0, |h| h.count);
    assert!(
        hit("email") >= 2 && hit("ipv4") >= 1 && hit("invented-password") >= 1,
        "{:?}",
        report.hits
    );
    assert!(report.bytes > 0 && out.exists());

    // The archive file as written, and every entry decompressed.
    let raw = std::fs::read(&out).unwrap();
    let entries = every_entry(&out);
    for secret in SECRETS {
        assert!(!contains(&raw, secret), "{secret} in the archive bytes");
        for (name, bytes) in &entries {
            assert!(!contains(bytes, secret), "{secret} in {name}");
            assert!(!name.contains("contoso-invented"), "{secret} in the entry name {name}");
        }
    }
    // Nor the path of the work folder inside the original's data folder (it can name the user).
    let tmp_name = dir.0.file_name().unwrap().to_string_lossy().into_owned();
    assert!(
        !contains(&raw, &tmp_name) && entries.iter().all(|(_, b)| !contains(b, &tmp_name)),
        "the data folder's path is in the copy"
    );
    assert!(entries.iter().all(|(n, _)| !n.contains("redacted-") && !n.starts_with("tmp/")));

    // It opens, holds the redacted events and keeps the UI state it was given.
    let open_root = TempDir::new("redcopy-open");
    let (copy, manifest) = DuckStore::open_project(&out, &open_root.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    assert_eq!(manifest.name, "shared");
    assert!(manifest.roots.is_empty() && !manifest.profile_overrides.contains("hunter2"));
    let rows = copy
        .page(&PageQuery {
            limit: 10,
            ..PageQuery::default()
        })
        .unwrap()
        .rows;
    assert_eq!(rows.len(), 3);
    copy.wait_for_index(std::time::Duration::from_secs(60)).unwrap();
    assert!(
        rows.iter().any(|r| r.message.contains("<email>")),
        "{:?}",
        rows.iter().map(|r| &r.message).collect::<Vec<_>>()
    );
    assert!(rows.iter().all(|r| SECRETS.iter().all(|s| !r.message.contains(s))));
    assert!(found(&copy, "login") >= 1 && found(&copy, "hunter2") == 0 && found(&copy, "contoso") == 0);
    assert!(std::fs::read_to_string(Projects::store_dir(&copy).join("ui.json"))
        .unwrap()
        .contains("<email> was here"));
    // The original is untouched and left no work files behind.
    assert!(found(&store, "login") >= 1, "the index is not ready");
    assert!(found(&store, "hunter2") >= 1);
    assert!(std::fs::read_dir(dir.0.join("tmp")).map_or(true, |d| d
        .flatten()
        .all(|e| !e.file_name().to_string_lossy().starts_with("redacted-"))));
}

#[test]
fn nothing_to_redact_with_or_nothing_copyable_is_refused_with_what_to_do() {
    let logs = TempDir::new("redcopy-refuse-logs");
    std::fs::write(logs.0.join("t.csv"), "Time,Level,Message\n2026-10-14 10:00:00,INFO,x\n").unwrap();
    let dir = TempDir::new("redcopy-refuse");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    let out = dir.0.join("o.debriefz");
    let e = store.export_redacted_project(&request(&out, RedactCfg::default())).unwrap_err();
    assert!(matches!(e, StoreError::Invalid(ref m) if m.contains("redact")), "{e}");
    let e = store.export_redacted_project(&request(&out, cfg())).unwrap_err();
    assert!(
        matches!(e, StoreError::Invalid(ref m) if m.contains("nothing can be copied")),
        "{e}"
    );
    assert!(!out.exists());
    let bad = RedactCfg {
        rule: vec![RedactRuleCfg {
            name: "bad".into(),
            regex: "(".into(),
            replace: None,
        }],
        ..cfg()
    };
    assert!(matches!(
        store.export_redacted_project(&request(&out, bad)),
        Err(StoreError::Invalid(_))
    ));
}

#[test]
fn a_cancelled_copy_reports_its_progress_stops_and_leaves_no_file() {
    let logs = TempDir::new("redcopy-cancel-logs");
    write_logs(&logs.0);
    let dir = TempDir::new("redcopy-cancel-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    let out_dir = TempDir::new("redcopy-cancel-out");
    let out = out_dir.0.join("shared.debriefz");
    let control = debrief_core::ql::QueryControl::new();
    control.cancel();
    let err = control.scope(|| store.export_redacted_project(&request(&out, cfg()))).unwrap_err();
    assert!(matches!(err, StoreError::Cancelled), "{err:?}");
    assert!(!out.exists());
    // Progress: an uncancelled run moves through redacting, importing the copy, finishing and writing the archive, in that
    // order, and ends with every byte of the archive's files written.
    use debrief_core::ql::QueryStage::{Finishing, Importing, Matching, Writing};
    let control = debrief_core::ql::QueryControl::new();
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let watcher = {
        let (c, seen, stop) = (control.clone(), seen.clone(), stop.clone());
        std::thread::spawn(move || {
            while !stop.load(std::sync::atomic::Ordering::Relaxed) {
                let p = c.progress();
                let mut s = seen.lock().unwrap();
                if s.last().map(|(st, _, _)| *st) != Some(p.stage) {
                    s.push((p.stage, p.done, p.total));
                }
                drop(s);
                std::thread::sleep(std::time::Duration::from_micros(200));
            }
        })
    };
    control.scope(|| store.export_redacted_project(&request(&out, cfg()))).unwrap();
    stop.store(true, std::sync::atomic::Ordering::Relaxed);
    watcher.join().unwrap();
    let p = control.progress();
    assert!(p.stage == Writing && p.total > 0 && p.done == p.total, "{p:?}");
    let order = |s| [Matching, Importing, Finishing, Writing].iter().position(|x| *x == s).unwrap();
    let seen = seen.lock().unwrap();
    assert!(
        seen.windows(2).all(|w| order(w[0].0) < order(w[1].0)),
        "stages out of order: {seen:?}"
    );
}
