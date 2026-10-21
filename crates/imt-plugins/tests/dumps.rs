//! The dumps plugins (ADR 0023) as IMT runs them: the readers in their locked-down process, reached through
//! `imt_dump::IMPORTERS_POINT` with the file lent by ranges; the capture tool in its own process, its folder
//! argument checked against the grants.

mod common;

use imt_core::plugin::tools::{NoTasks, PathAccess, PathKind, PathPolicy, ToolCtx, ToolError};
use imt_dump::model::Document;
use imt_dump::{DumpImporter, ImportOptions, Limits, Produces};
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::Arc;

fn options() -> ImportOptions {
    ImportOptions {
        limits: Limits::default(),
        file_time_ms: Some(1_791_970_000_000),
    }
}

fn importer(kind: Produces) -> imt_core::plugin::services::Service<dyn DumpImporter> {
    let registry = common::registry(&[common::DUMPS]);
    registry
        .contributions::<dyn DumpImporter>(imt_dump::IMPORTERS_POINT)
        .into_iter()
        .find(|i| i.call(|i| i.produces()).ok() == Some(kind))
        .expect("the dumps plugin contributes an importer of this kind")
}

#[test]
fn the_sample_crash_dump_is_read_in_the_plugin_process() {
    let path = common::samples().join("windows/WEB02/OrderService.exe.4120.dmp");
    let bytes = std::fs::read(&path).unwrap();
    let dumps = importer(Produces::Dump);
    assert_eq!(dumps.call(|i| i.sniff("x.dmp", &bytes[..64])).unwrap(), 100);
    let doc = dumps
        .call(|i| i.import("OrderService.exe.4120.dmp", Arc::new(bytes.clone()), &options()))
        .unwrap()
        .unwrap();
    let Document::Dump(d) = doc else { panic!("not a dump") };
    let e = d.exception.as_ref().unwrap();
    assert_eq!(e.code, 0xE043_4352, "the CLR exception of the sample");
    assert!(d.dotnet.is_some(), "coreclr is loaded");
    let f = imt_dump::findings::findings(1, &d);
    assert_eq!(f[0].kind, imt_dump::api::FindingKind::CrashCause);
    assert!(f.iter().any(|f| f.kind == imt_dump::api::FindingKind::Dotnet));
}

#[test]
fn damaged_files_are_errors_not_crashes() {
    let dumps = importer(Produces::Dump);
    let mut bytes = b"MDMP".to_vec();
    bytes.extend_from_slice(&[0xFF; 60]);
    let r = dumps.call(|i| i.import("bad.dmp", Arc::new(bytes.clone()), &options())).unwrap();
    assert!(r.is_err());
    // The process is still fine afterwards.
    let ok = std::fs::read(common::samples().join("windows/WEB02/OrderService.exe.4120.dmp")).unwrap();
    assert!(dumps.call(|i| i.import("x.dmp", Arc::new(ok.clone()), &options())).unwrap().is_ok());
    let umdh = importer(Produces::Umdh);
    assert_eq!(umdh.call(|i| i.sniff("x.txt", b"plain text")).unwrap(), 0);
    assert!(umdh
        .call(|i| i.import("x.txt", Arc::new(b"not umdh".to_vec()), &options()))
        .unwrap()
        .is_err());
}

#[test]
fn umdh_snapshots_are_read_in_the_plugin_process() {
    let dir = common::samples().join("dumps/umdh");
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
    files.sort();
    let umdh = importer(Produces::Umdh);
    let mut logs = Vec::new();
    for f in &files {
        let bytes = std::fs::read(f).unwrap();
        assert!(
            umdh.call(|i| i.sniff("x", &bytes[..bytes.len().min(65536)])).unwrap() >= 50,
            "{}",
            f.display()
        );
        match umdh.call(|i| i.import("x", Arc::new(bytes.clone()), &options())).unwrap().unwrap() {
            Document::Umdh(u) => logs.push(u),
            other => panic!("{other:?}"),
        }
    }
    let snapshots: Vec<_> = logs.iter().filter(|l| l.kind == imt_dump::model::UmdhKind::Snapshot).collect();
    assert!(snapshots.len() >= 3);
    let series: Vec<(u32, &imt_dump::model::UmdhLog)> = snapshots.iter().enumerate().map(|(i, l)| (i as u32 + 1, *l)).collect();
    let f = imt_dump::umdh::findings(&series).unwrap();
    assert!(
        f.iter().any(|f| f.kind == imt_dump::api::FindingKind::SteadyGrowth),
        "the samples leak steadily: {f:#?}"
    );
}

/// Grants every path under one folder (the shell's grants, for these tests).
struct Under(PathBuf);

impl PathPolicy for Under {
    fn check(&self, path: &str, _: PathKind, _: PathAccess) -> Result<PathBuf, String> {
        let p = Path::new(path);
        if p.starts_with(&self.0) {
            Ok(p.to_path_buf())
        } else {
            Err(format!("\"{path}\" was not chosen"))
        }
    }
}

#[test]
fn the_capture_tool_needs_a_granted_folder() {
    let registry = common::registry(&[common::DUMP_CAPTURE]);
    let tools = registry.tools();
    assert!(tools.list().iter().any(|t| t.info.id == "dumps.capture"));
    let dir = tempfile::tempdir().unwrap();
    let args = json!({ "pid": std::process::id(), "kind": "mini", "folder": dir.path().to_string_lossy() });
    let refused = tools.invoke("dumps.capture", "create", args.clone(), &ToolCtx::new(None, Arc::new(NoTasks)));
    if cfg!(windows) {
        assert!(matches!(refused, Err(ToolError::PathRefused { .. })), "{refused:?}");
    } else {
        // Creating a dump is Windows-only (ADR 0023): elsewhere the tool says so before it looks at the folder.
        assert!(
            matches!(&refused, Err(ToolError::Unsupported(m)) if m.contains("only available on Windows")),
            "{refused:?}"
        );
    }
    let granted = ToolCtx::new(None, Arc::new(NoTasks)).with_paths(Arc::new(Under(dir.path().to_path_buf())));
    let made = tools.invoke("dumps.capture", "create", args, &granted);
    if cfg!(windows) {
        let made = made.unwrap();
        let path = made["file"].as_str().unwrap();
        assert!(Path::new(path).starts_with(dir.path()) && Path::new(path).is_file());
        let list = tools.invoke("dumps.capture", "processes", json!({}), &granted).unwrap();
        assert!(list.as_array().unwrap().iter().any(|p| p["pid"] == json!(std::process::id())));
    } else {
        assert!(made.is_err());
    }
}
