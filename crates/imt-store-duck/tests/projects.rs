//! Diagnostics with quick fixes, classified profile changes, source context and self-contained projects
//! on the DuckDB store.

mod common;

use common::{samples, TempDir};
use imt_core::api::ContextRequest;
use imt_core::ingest::ProblemKind;
use imt_core::package::ProjectManifest;
use imt_core::ports::{Diagnostics, Filter, Ingest, PageQuery, Profiles, Query, QuickFix, StoreError};
use imt_core::{PluginRegistry, Profile};
use imt_store_duck::DuckStore;
use std::path::{Path, PathBuf};

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for e in std::fs::read_dir(from).unwrap() {
        let e = e.unwrap();
        let target = to.join(e.file_name());
        if e.file_type().unwrap().is_dir() {
            copy_dir(&e.path(), &target);
        } else {
            std::fs::copy(e.path(), target).unwrap();
        }
    }
}

/// A folder with a CSV whose timestamps the default profile cannot read, a file nobody reads and a
/// copy of the Serilog text samples.
fn logs(dir: &TempDir) -> PathBuf {
    let logs = dir.0.join("logs");
    copy_dir(&samples().join("serilog-text"), &logs.join("serilog"));
    std::fs::write(
        logs.join("ctime.csv"),
        "Time,Level,Message\nSat Nov 08 00:04:37 2025,INFO,started\nSat Nov 08 00:04:38 2025,WARN,slow\nSat Nov 08 00:04:39 2025,ERROR,failed\nSat Nov 08 00:04:40 2025,INFO,done\n",
    )
    .unwrap();
    std::fs::write(logs.join("blob.bin"), [0u8, 159, 146, 150, 0, 0, 1, 2, 3, 255, 0, 7]).unwrap();
    logs
}

fn import(dir: &Path, logs: &Path) -> DuckStore {
    let store = DuckStore::create_project(dir, "incident", Profile::default_profile(), PluginRegistry::new()).unwrap();
    store.open(&[logs.to_path_buf()]).unwrap().wait().unwrap();
    store
}

#[test]
fn problems_come_with_fixes_the_engine_checks() {
    let dir = TempDir::new("diag");
    let logs = logs(&dir);
    let store = import(&dir.0.join("p.imt"), &logs);
    let problems = store.problems().unwrap();
    let ts = problems
        .iter()
        .find(|p| p.kind == ProblemKind::TimestampUnparsed && p.source_name.ends_with("ctime.csv"))
        .unwrap_or_else(|| panic!("{problems:#?}"));
    assert!(ts.count >= 4 && ts.sample_lines.len() <= 20 && ts.path.ends_with("ctime.csv"));
    let fix = ts.fix.clone().expect("a verified timestamp format");
    assert_eq!(fix.timestamp.as_ref().unwrap().format, "%a %b %d %H:%M:%S %Y");
    assert!(fix.profile_override.contains("timestamp_formats"), "{}", fix.profile_override);
    let unknown = problems.iter().find(|p| p.kind == ProblemKind::UnknownFormat).expect("blob.bin");
    assert!(unknown.source.is_none() && unknown.path.ends_with("blob.bin") && unknown.detail.is_some());
    // The fix repairs the stored events; a useless override repairs none.
    let quick = QuickFix {
        profile_override: fix.profile_override.clone(),
        sources: vec![ts.source.unwrap()],
    };
    let good = store.evaluate_fix(&quick).unwrap();
    assert!(good.checked > 0 && good.fixed == good.checked, "{good:?}");
    let useless = store
        .evaluate_fix(&QuickFix {
            profile_override: "name = \"same\"".into(),
            sources: vec![],
        })
        .unwrap();
    assert_eq!(useless.fixed, 0, "{useless:?}");
    assert!(useless.checked > 0);
    let bad = store.evaluate_fix(&QuickFix {
        profile_override: "[parse".into(),
        sources: vec![],
    });
    assert!(matches!(bad, Err(StoreError::Invalid(_))), "{bad:?}");

    // Applying it is a parsing change: the files are read again and the timestamps are read.
    let before = store.summary().unwrap();
    let profile = store.active_profile().unwrap().with_override(&fix.profile_override).unwrap();
    store.set_profile(profile).unwrap().wait().unwrap();
    let after = store.summary().unwrap();
    assert!(after.untimed < before.untimed, "{} -> {}", before.untimed, after.untimed);
    assert!(!store
        .problems()
        .unwrap()
        .iter()
        .any(|p| p.kind == ProblemKind::TimestampUnparsed && p.source_name.ends_with("ctime.csv")));
    // A presentation change does no data work.
    let renamed = Profile {
        name: "renamed".into(),
        ..store.active_profile().unwrap()
    };
    let report = store.set_profile(renamed).unwrap().wait().unwrap();
    assert_eq!(report.events, 0);
    assert_eq!(store.summary().unwrap().total, after.total);
    assert_eq!(store.active_profile().unwrap().name, "renamed");
    // Reload reads everything again.
    let reloaded = store.reload().unwrap().wait().unwrap();
    assert_eq!(reloaded.events, after.total);
    // Without the files, a parsing change is refused with a message naming them.
    std::fs::remove_file(logs.join("ctime.csv")).unwrap();
    let day_first = store.active_profile().unwrap().with_override("[parse]\nday_first = true").unwrap();
    match store.set_profile(day_first).map(|_| ()) {
        Err(StoreError::Invalid(m)) => assert!(m.contains("ctime.csv") && m.contains("missing"), "{m}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn projects_open_without_their_files_and_degrade_source_context() {
    let dir = TempDir::new("project");
    let logs = logs(&dir);
    let project = dir.0.join("incident.imt");
    let store = import(&project, &logs);
    let page = store
        .page(&PageQuery {
            query: Filter {
                text: "started".into(),
                ..Filter::default()
            },
            limit: 5,
            ..PageQuery::default()
        })
        .unwrap();
    let event = page
        .rows
        .iter()
        .find(|r| r.source_name.contains("serilog"))
        .cloned()
        .unwrap_or_else(|| page.rows[0].clone());
    let req = ContextRequest {
        record_id: u32::try_from(event.id.0).unwrap(),
        before: 3,
        after: 3,
    };
    let live = store.source_context(&req).unwrap();
    assert_eq!(live.note, None);
    assert!(live.lines.iter().any(|l| l.line == event.line), "{live:?}");
    let total = store.summary().unwrap().total;
    let mut manifest = ProjectManifest {
        name: "incident".into(),
        ..ProjectManifest::default()
    };
    let archive = dir.0.join("incident.imtz");
    assert!(store.export_project(&mut manifest, &archive).unwrap() > 0);
    drop(store);
    // The project is shared: the original files are gone.
    std::fs::remove_dir_all(&logs).unwrap();
    let (opened, m) = DuckStore::open_project(&archive, &dir.0.join("opened"), Profile::default_profile(), PluginRegistry::new()).unwrap();
    assert_eq!(m.name, "incident");
    assert_eq!(m.roots.len(), 1);
    assert_eq!(opened.summary().unwrap().total, total);
    let shared = opened.source_context(&req).unwrap();
    assert!(shared.note.as_deref().is_some_and(|n| n.contains("not available")), "{shared:?}");
    let line = shared.lines.iter().find(|l| l.line == event.line).expect("the event's line");
    assert_eq!(line.text, live.lines.iter().find(|l| l.line == event.line).unwrap().text);
    assert!(matches!(
        opened.source_context(&ContextRequest {
            record_id: u32::MAX,
            before: 1,
            after: 1
        }),
        Err(StoreError::NotFound(_))
    ));
    drop(opened);
    // The folder opens as it is; a newer project format is refused; a collector bundle is not a project.
    let (again, _) = DuckStore::open_project(&project, &dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    assert_eq!(again.summary().unwrap().total, total);
    drop(again);
    let toml = project.join("project.toml");
    let text = std::fs::read_to_string(&toml).unwrap().replace("version = 1", "version = 99");
    std::fs::write(&toml, text).unwrap();
    match DuckStore::open_project(&project, &dir.0, Profile::default_profile(), PluginRegistry::new()) {
        Err(StoreError::Invalid(m)) => assert!(m.contains("newer IMT"), "{m}"),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("opened a newer project"),
    }
    let collector = dir.0.join("collected.imtz");
    {
        use std::io::Write;
        let mut z = zip::ZipWriter::new(std::fs::File::create(&collector).unwrap());
        z.start_file("manifest.json", zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(b"{}").unwrap();
        z.finish().unwrap();
    }
    match DuckStore::open_project(&collector, &dir.0.join("c"), Profile::default_profile(), PluginRegistry::new()) {
        Err(StoreError::Invalid(m)) => assert!(m.contains("collector bundle"), "{m}"),
        Err(e) => panic!("{e}"),
        Ok(_) => panic!("opened a collector bundle as a project"),
    }
}
