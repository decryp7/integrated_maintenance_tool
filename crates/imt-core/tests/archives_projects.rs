//! Compressed input, the context view, and saving/restoring workspace state.

use imt_core::api::{ContextRequest, WorkspaceState};
use imt_core::project::ProjectFile;
use imt_core::{LogStore, Profile, Workspace};
use std::io::Write;
use std::path::{Path, PathBuf};

struct Dir(PathBuf);
impl Dir {
    fn new(tag: &str) -> Dir {
        let p = std::env::temp_dir().join(format!("imt-ap-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        Dir(std::fs::canonicalize(p).unwrap())
    }
    fn write(&self, name: &str, bytes: &[u8]) -> PathBuf {
        let p = self.0.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, bytes).unwrap();
        p
    }
}
impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn log(machine: &str, n: usize) -> String {
    (0..n)
        .map(|i| format!("2026-10-14 10:00:{i:02}.000 +00:00 [INF] {machine} event {i} {{\"MachineName\": \"{machine}\"}}\n"))
        .collect()
}

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    let opts = zip::write::SimpleFileOptions::default();
    for (name, data) in files {
        w.start_file(*name, opts).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn names(ws: &Workspace) -> Vec<String> {
    ws.summary().sources.iter().map(|s| s.name.clone()).collect()
}

#[test]
fn reads_gzip_and_zip_transparently() {
    let d = Dir::new("archives");
    d.write("logs/app.log.1.gz", &gzip(log("WEB01", 3).as_bytes()));
    d.write(
        "logs/bundle.zip",
        &zip_of(&[
            ("web02/app.log", log("WEB02", 4).as_bytes()),
            ("worker/w.log", log("WORKER", 2).as_bytes()),
            ("readme.bin", &[0, 1, 2, 3, 0, 0, 0, 1, 2, 3, 4, 5]),
        ]),
    );
    d.write("logs/broken.zip", b"PK\x03\x04not really");
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();

    assert_eq!(
        names(&ws),
        ["logs/app.log.1.gz", "logs/bundle.zip/web02/app.log", "logs/bundle.zip/worker/w.log"]
    );
    assert_eq!(ws.summary().total, 9);
    let unrecognized: Vec<(String, String)> = ws
        .summary()
        .unrecognized
        .iter()
        .map(|u| (u.name.clone(), u.reason.clone()))
        .collect();
    assert!(
        unrecognized
            .iter()
            .any(|(n, r)| n == "logs/bundle.zip/readme.bin" && r == "binary file"),
        "{unrecognized:?}"
    );
    assert!(
        unrecognized
            .iter()
            .any(|(n, r)| n == "logs/broken.zip" && r.contains("invalid zip")),
        "{unrecognized:?}"
    );

    // Opening again adds nothing; a removed archive entry stays out after reload.
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    assert_eq!(ws.summary().total, 9);
    let worker = ws.summary().sources.iter().find(|s| s.name.ends_with("w.log")).unwrap().id;
    ws.remove_source(worker);
    ws.reload().unwrap();
    assert_eq!(names(&ws), ["logs/app.log.1.gz", "logs/bundle.zip/web02/app.log"]);
}

#[test]
fn context_view_reads_plain_compressed_and_archived_sources() {
    let d = Dir::new("context");
    let mut text = log("WEB01", 30);
    text.push_str("System.Exception: boom\n   at X()\n");
    d.write("plain.log", text.as_bytes());
    d.write("gz.log.gz", &gzip(log("GZ", 5).as_bytes()));
    d.write("z.zip", &zip_of(&[("in/zip.log", log("ZIP", 5).as_bytes())]));
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let id_of = |ws: &Workspace, m: &str, i: usize| {
        ws.records()
            .iter()
            .find(|r| r.message == format!("{m} event {i}"))
            .unwrap_or_else(|| panic!("{m} {i}"))
            .id
    };

    let c = ws
        .source_context(&ContextRequest {
            record_id: id_of(&ws, "WEB01", 15),
            before: 2,
            after: 3,
        })
        .unwrap();
    assert_eq!(c.source_name, "plain.log");
    assert_eq!(c.line, 16);
    assert_eq!(c.lines.iter().map(|l| l.line).collect::<Vec<_>>(), [14, 15, 16, 17, 18, 19]);
    assert!(c.lines[2].text.contains("WEB01 event 15"));

    let first = ws
        .source_context(&ContextRequest {
            record_id: id_of(&ws, "WEB01", 0),
            before: 10,
            after: 0,
        })
        .unwrap();
    assert_eq!(first.lines.len(), 1);
    let last = ws
        .source_context(&ContextRequest {
            record_id: id_of(&ws, "WEB01", 29),
            before: 0,
            after: 100,
        })
        .unwrap();
    assert_eq!(last.lines.last().unwrap().text, "   at X()");

    let gz = ws
        .source_context(&ContextRequest {
            record_id: id_of(&ws, "GZ", 2),
            before: 1,
            after: 1,
        })
        .unwrap();
    assert_eq!(gz.lines.len(), 3);
    let z = ws
        .source_context(&ContextRequest {
            record_id: id_of(&ws, "ZIP", 4),
            before: 1,
            after: 5,
        })
        .unwrap();
    assert_eq!(z.source_name, "z.zip/in/zip.log");
    assert_eq!(z.lines.len(), 2);

    let req: ContextRequest = serde_json::from_str(r#"{"recordId": 0}"#).unwrap();
    assert_eq!((req.before, req.after), (10, 10));
    assert!(ws
        .source_context(&ContextRequest {
            record_id: 9_999,
            before: 1,
            after: 1
        })
        .is_err());

    // The file changed or vanished since import.
    let plain_id = id_of(&ws, "WEB01", 1);
    std::fs::remove_file(d.0.join("plain.log")).unwrap();
    assert!(ws
        .source_context(&ContextRequest {
            record_id: plain_id,
            before: 1,
            after: 1
        })
        .unwrap_err()
        .to_string()
        .contains("cannot read"));
    let zip_id = id_of(&ws, "ZIP", 1);
    d.write("z.zip", &zip_of(&[("other.log", b"x")]));
    assert!(ws
        .source_context(&ContextRequest {
            record_id: zip_id,
            before: 1,
            after: 1
        })
        .unwrap_err()
        .to_string()
        .contains("no longer in"));
    let gz_id = id_of(&ws, "GZ", 1);
    d.write("gz.log.gz", &gzip(&[0, 1, 2, 3, 0, 0, 0, 0, 1, 2, 3, 4, 5, 6]));
    assert!(ws
        .source_context(&ContextRequest {
            record_id: gz_id,
            before: 1,
            after: 1
        })
        .unwrap_err()
        .to_string()
        .contains("no longer a text file"));
}

#[test]
fn saves_and_restores_projects() {
    let d = Dir::new("project");
    d.write("logs/a.log", log("A", 3).as_bytes());
    d.write("logs/b.log", log("B", 3).as_bytes());
    d.write("solo.log", log("S", 2).as_bytes());
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.0.join("logs"), d.0.join("solo.log")]).unwrap();
    let b = ws.summary().sources.iter().find(|s| s.name == "b.log").unwrap().id;
    ws.remove_source(b);

    let state = ws.state();
    assert_eq!(state.roots.len(), 2);
    assert_eq!(state.removed, [d.0.join("logs/b.log").to_string_lossy().into_owned()]);
    let file = d.0.join("incident.imtproj");
    let project = ProjectFile {
        version: 1,
        name: "Incident".into(),
        sources: state.roots.clone(),
        removed: state.removed.clone(),
        profile: "name = 'Saved profile'".into(),
        ui: serde_json::json!({ "bookmarks": [1, 2] }),
    };
    project.save(&file).unwrap();

    // Restore into a fresh workspace through the store abstraction.
    let loaded = ProjectFile::load(&file).unwrap();
    let mut store: Box<dyn LogStore> = Box::new(Workspace::new(Profile::default_profile()).unwrap());
    let profile = Profile::from_toml(&loaded.profile).unwrap();
    let missing = store
        .restore(
            profile,
            &WorkspaceState {
                roots: loaded.sources.clone(),
                removed: loaded.removed.clone(),
            },
        )
        .unwrap();
    assert!(missing.is_empty());
    let s = store.summary();
    assert_eq!(s.profile_name, "Saved profile");
    assert_eq!(s.total, 5, "a.log + solo.log, b.log stays removed");
    assert_eq!(
        store.state(),
        WorkspaceState {
            roots: loaded.sources.clone(),
            removed: loaded.removed.clone()
        }
    );

    // Missing paths are reported, not fatal; an invalid profile is.
    std::fs::remove_file(d.0.join("solo.log")).unwrap();
    let missing = store
        .restore(
            Profile::default_profile(),
            &WorkspaceState {
                roots: loaded.sources.clone(),
                removed: vec![],
            },
        )
        .unwrap();
    assert_eq!(missing, [d.0.join("solo.log").to_string_lossy().into_owned()]);
    assert_eq!(store.summary().total, 6);
    let bad = Profile::from_toml("[parse]\nregexes = ['(']").unwrap();
    assert!(store.restore(bad, &WorkspaceState::default()).is_err());
    assert!(Path::new(&file).exists());
}

#[test]
fn refreshes_changed_new_and_deleted_files() {
    let d = Dir::new("refresh");
    let app = d.write("logs/app.log", log("A", 2).as_bytes());
    d.write("logs/skip.gz.tmp", b"x");
    d.write("solo.log", log("S", 1).as_bytes());
    let outside = Dir::new("refresh-outside");
    let stranger = outside.write("x.log", log("X", 1).as_bytes());
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&[d.0.join("logs"), d.0.join("solo.log")]).unwrap();
    assert_eq!(ws.summary().total, 3);

    // Appended lines, a rotated file, a file outside the workspace, a directory.
    std::fs::write(&app, log("A", 5)).unwrap();
    let rotated = d.write("logs/app.log.1", log("R", 3).as_bytes());
    let n = ws
        .refresh_files(&[app.clone(), rotated.clone(), stranger, d.0.join("logs")])
        .unwrap();
    assert_eq!(n, 2);
    assert_eq!(ws.summary().total, 5 + 3 + 1);
    assert_eq!(ws.summary().sources.len(), 3);

    // Removed sources stay out; deleted files disappear; excluded names are ignored.
    let rid = ws.summary().sources.iter().find(|s| s.name == "app.log.1").unwrap().id;
    ws.remove_source(rid);
    assert_eq!(ws.refresh_files(std::slice::from_ref(&rotated)).unwrap(), 0);
    std::fs::remove_file(&app).unwrap();
    let hidden = d.write("logs/.hidden.log", log("H", 1).as_bytes());
    ws.refresh_files(&[app, hidden]).unwrap();
    let left: Vec<String> = ws.summary().sources.iter().map(|s| s.name.clone()).collect();
    assert!(left.len() == 1 && left[0].ends_with("/solo.log"), "{left:?}");

    // An explicitly opened file refreshes too.
    std::fs::write(d.0.join("solo.log"), log("S", 4)).unwrap();
    let mut store: Box<dyn LogStore> = Box::new(ws);
    assert_eq!(store.refresh_files(&[d.0.join("solo.log")]).unwrap(), 1);
    assert_eq!(store.summary().total, 4);
}

#[test]
fn imports_report_progress_and_can_be_cancelled() {
    let d = Dir::new("progress");
    for i in 0..20 {
        d.write(&format!("f{i:02}.log"), log("P", 2).as_bytes());
    }
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    let progress = ws.progress();
    assert!(!progress.snapshot().running);
    ws.open(std::slice::from_ref(&d.0)).unwrap();
    let snap = progress.snapshot();
    assert_eq!((snap.running, snap.done, snap.total, snap.cancelled), (false, 20, 20, false));

    // Cancel while importing: an importer pulls the brake on the first file it reads.
    struct Brake(std::sync::Mutex<Option<std::sync::Arc<imt_core::workspace::ImportProgress>>>);
    impl imt_core::importers::Importer for Brake {
        fn id(&self) -> &'static str {
            "brake"
        }
        fn sniff(&self, _: &imt_core::importers::ImportCtx, head: &str) -> u8 {
            if head.starts_with("BRAKE") {
                100
            } else {
                0
            }
        }
        fn import(&self, _: &imt_core::importers::ImportCtx, _: &str) -> anyhow::Result<imt_core::importers::Imported> {
            if let Some(p) = self.0.lock().unwrap().as_ref() {
                p.cancel();
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
            let mut r = imt_core::model::Record::new(1);
            r.message = "x".into();
            Ok(imt_core::importers::Imported {
                records: vec![r],
                skipped: 0,
            })
        }
    }
    let brake = std::sync::Arc::new(Brake(std::sync::Mutex::new(None)));
    let mut importers = imt_core::importers::Importers::new();
    struct Shared(std::sync::Arc<Brake>);
    impl imt_core::importers::Importer for Shared {
        fn id(&self) -> &'static str {
            self.0.id()
        }
        fn sniff(&self, c: &imt_core::importers::ImportCtx, h: &str) -> u8 {
            self.0.sniff(c, h)
        }
        fn import(&self, c: &imt_core::importers::ImportCtx, t: &str) -> anyhow::Result<imt_core::importers::Imported> {
            self.0.import(c, t)
        }
    }
    importers.register(Box::new(Shared(brake.clone())));
    let mut ws = Workspace::with_importers(Profile::default_profile(), importers).unwrap();
    *brake.0.lock().unwrap() = Some(ws.progress());
    let d2 = Dir::new("progress2");
    for i in 0..200 {
        d2.write(&format!("g{i:03}.log"), b"BRAKE\n");
    }
    ws.open(std::slice::from_ref(&d2.0)).unwrap();
    let snap = ws.progress().snapshot();
    assert!(snap.cancelled && !snap.running && snap.done == 200);
    let imported = ws.summary().total;
    assert!(
        (1..200).contains(&imported),
        "cancel stopped the import early ({imported} of 200)"
    );
    // Opening again imports the files that were skipped.
    *brake.0.lock().unwrap() = None;
    ws.open(std::slice::from_ref(&d2.0)).unwrap();
    assert_eq!(ws.summary().total, 200);
}
