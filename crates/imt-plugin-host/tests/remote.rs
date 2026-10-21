//! A plugin process supervised by the host, against the fake plugin: imports stream back with credits, a crash,
//! a panic or a hang is a failure attributed to the plugin (and quarantines it when it repeats), typed errors are
//! not, settings are restored after a restart, annotators read the events through callbacks, path arguments need
//! a grant, and the normal process refuses the operations of the elevated one.

use imt_core::guard::Budgets;
use imt_core::plugin::annotate::AnnotationsRequest;
use imt_core::plugin::tools::{NoTasks, PathAccess, PathKind, PathPolicy, TaskSink, ToolCtx, ToolError, ToolTask};
use imt_core::plugin::PluginStatus;
use imt_core::{PluginRegistry, Profile, Workspace};
use imt_ipc::IpcError;
use imt_plugin_api::protocol::PluginRequest;
use imt_plugin_host::Builtins;
use serde_json::json;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const ID: &str = "test.fake";

fn builtins() -> Builtins {
    let b = Builtins::load(Path::new(env!("CARGO_BIN_EXE_imt-plugin-host-fake")));
    assert!(b.problem().is_none(), "{:?}", b.problem());
    b
}

fn registry(b: &Builtins) -> PluginRegistry {
    let mut r = PluginRegistry::new();
    for p in b.plugins() {
        r.install_shared(p).unwrap();
    }
    r.resolve();
    r
}

fn workspace(b: &Builtins) -> Workspace {
    Workspace::with_registry(Profile::default_profile(), registry(b)).unwrap()
}

/// A file whose first line makes the fake importer pick it.
fn file(dir: &Path, name: &str, body: &str) -> PathBuf {
    let p = dir.join(name);
    std::fs::write(&p, format!("#fake\n{body}\n")).unwrap();
    p
}

fn pid(r: &PluginRegistry) -> u64 {
    r.tools()
        .invoke("fake.tool", "pid", json!({}), &ToolCtx::new(None, Arc::new(NoTasks)))
        .unwrap()
        .as_u64()
        .unwrap()
}

#[test]
fn imports_stream_back_and_annotators_read_through_the_host() {
    let b = builtins();
    let dir = tempfile::tempdir().unwrap();
    let mut ws = workspace(&b);
    ws.open(&[file(dir.path(), "a.log", "hello\nworld")]).unwrap();
    assert_eq!(ws.records().len(), 2);
    assert_eq!(ws.summary().sources[0].importer, "fake");
    // Identity fields and problems cross the contract (ADR 0020).
    assert_eq!(ws.registry().importers().find("fake").unwrap().identity_fields(), ["Host", "App"]);
    let mut ws = workspace(&b);
    ws.open(&[file(
        dir.path(),
        "p.log",
        "one
?bad
?worse
two",
    )])
    .unwrap();
    assert_eq!(ws.records().len(), 2);
    let problems = &ws.summary().sources[0].problems;
    let p = problems
        .iter()
        .find(|p| p.kind == imt_plugin_api::problems::ProblemKind::LineNotParsed)
        .expect("the plugin's problems");
    assert_eq!((p.count, p.sample_lines.clone()), (2, vec![3, 4]));
    // More records than one item holds, over several credits.
    let mut ws = workspace(&b);
    ws.open(&[file(dir.path(), "many.log", "many 20000")]).unwrap();
    assert_eq!(ws.records().len(), 20_000);
    let notes = ws.annotations(&AnnotationsRequest::default());
    let seen = notes.iter().find(|a| a.kind == "test.fake/seen").expect("the annotator ran");
    assert!(seen.label.starts_with("1000 events"), "{}", seen.label);
}

#[test]
fn crashes_and_panics_count_against_the_plugin_and_restart_its_process() {
    let b = builtins();
    let dir = tempfile::tempdir().unwrap();
    let mut ws = workspace(&b);
    let r_pid = |ws: &Workspace| pid(ws.registry());
    let first = r_pid(&ws);
    // A typed failure is just an error.
    let _ = ws.open(&[file(dir.path(), "fail.log", "fail")]);
    let info = ws.summary().plugins.into_iter().find(|p| p.id == ID).unwrap();
    assert_eq!(info.failures, 0, "{info:?}");
    // A crash fails the import and counts; the next call runs in a fresh process.
    let _ = ws.open(&[file(dir.path(), "crash.log", "crash")]);
    let info = ws.summary().plugins.into_iter().find(|p| p.id == ID).unwrap();
    assert_eq!(info.failures, 1, "{info:?}");
    assert!(info.last_error.unwrap_or_default().contains("crashed"));
    let second = r_pid(&ws);
    assert_ne!(first, second, "restarted");
    // A panic inside the plugin's code is contained in its process, counted, and the process restarted.
    let _ = ws.open(&[file(dir.path(), "panic.log", "panic")]);
    let info = ws.summary().plugins.into_iter().find(|p| p.id == ID).unwrap();
    assert_eq!(info.failures, 2, "{info:?}");
    assert_ne!(r_pid(&ws), second);
    // A third failure quarantines the plugin.
    let _ = ws.open(&[file(dir.path(), "crash2.log", "crash")]);
    let info = ws.summary().plugins.into_iter().find(|p| p.id == ID).unwrap();
    assert_eq!(info.status, PluginStatus::Quarantined, "{info:?}");
}

#[test]
fn revoking_a_permission_restarts_the_plugin_with_fewer() {
    use imt_plugin_api::manifest::Permission;
    let b = builtins();
    let ws = workspace(&b);
    let p = b.process(ID).unwrap();
    assert_eq!(p.granted_now(), [Permission::Network]);
    assert!(p.sandbox().network && p.granted(Permission::Network));
    let before = pid(ws.registry());
    p.set_granted(Vec::new());
    assert!(!p.sandbox().network && !p.granted(Permission::Network));
    let after = pid(ws.registry());
    assert_ne!(before, after, "the process was restarted with the new grants");
    assert!(!p.third_party());
    assert_eq!(
        p.tool_data_dir(Some(Path::new("/app"))).as_deref(),
        Some(Path::new("/app").to_str().unwrap())
    );
}

#[test]
fn a_hang_is_stopped_at_its_deadline() {
    let b = builtins();
    let dir = tempfile::tempdir().unwrap();
    let r = registry(&b);
    r.set_budgets(Budgets {
        import: Some(Duration::from_secs(2)),
        ..Budgets::default()
    });
    let mut ws = Workspace::with_registry(Profile::default_profile(), r).unwrap();
    let started = std::time::Instant::now();
    let _ = ws.open(&[file(dir.path(), "hang.log", "hang")]);
    assert!(started.elapsed() < Duration::from_secs(30), "{:?}", started.elapsed());
    let info = ws.summary().plugins.into_iter().find(|p| p.id == ID).unwrap();
    assert!(info.failures >= 1, "{info:?}");
}

#[test]
fn settings_are_restored_after_a_restart() {
    let b = builtins();
    let r = registry(&b);
    let section = r.plugin_settings().into_iter().find(|s| s.plugin.id == ID).unwrap();
    let valid = imt_core::settings::validate(&section.configurable.settings_section(), &json!({"level": 7})).unwrap();
    section.apply(&valid).unwrap();
    let ctx = ToolCtx::new(None, Arc::new(NoTasks));
    assert_eq!(r.tools().invoke("fake.tool", "settings", json!({}), &ctx).unwrap(), json!(7));
    // Kill the process; the restore hook applies the settings again in the next one.
    b.process(ID).unwrap().shutdown();
    assert_eq!(r.tools().invoke("fake.tool", "settings", json!({}), &ctx).unwrap(), json!(7));
}

struct Under(PathBuf);

impl PathPolicy for Under {
    fn check(&self, path: &str, _: PathKind, _: PathAccess) -> Result<PathBuf, String> {
        if Path::new(path).starts_with(&self.0) {
            Ok(PathBuf::from(path))
        } else {
            Err("not chosen".into())
        }
    }
}

#[test]
fn path_arguments_need_a_grant_and_elevated_operations_stay_out_of_the_normal_process() {
    let b = builtins();
    let r = registry(&b);
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("out.txt");
    let args = json!({"path": target.to_string_lossy()});
    let plain = ToolCtx::new(None, Arc::new(NoTasks));
    assert!(matches!(
        r.tools().invoke("fake.tool", "write", args.clone(), &plain),
        Err(ToolError::PathRefused { .. })
    ));
    assert!(!target.exists());
    // The tool runs locked down (low integrity), so writing a user folder may still be refused by the OS; the
    // host let the granted path through either way.
    let granted = plain.clone().with_paths(Arc::new(Under(dir.path().to_path_buf())));
    match r.tools().invoke("fake.tool", "write", args, &granted) {
        Ok(_) => assert!(target.exists()),
        Err(e) => assert!(!matches!(e, ToolError::PathRefused { .. }), "{e}"),
    }
    // The normal process refuses an elevated operation: only the elevated process serves it.
    let p = b.process(ID).unwrap();
    let req = PluginRequest::InvokeTool {
        tool: "fake.tool".into(),
        op: "admin".into(),
        args: json!({}).into(),
        data_dir: None,
    };
    let refused: Result<Result<imt_ipc::Json, imt_plugin_api::protocol::ToolFail>, IpcError> =
        p.call(&req, Some(Duration::from_secs(10)), false);
    match refused {
        Err(IpcError::Remote(e)) => assert!(e.message.contains("elevated process"), "{e:?}"),
        other => panic!("{other:?}"),
    }
}

#[test]
fn the_normal_process_runs_only_declared_elevated_operations_through_the_host() {
    // The elevated role as an ordinary process (the administrator prompt cannot run in tests).
    let exe = Path::new(env!("CARGO_BIN_EXE_imt-plugin-host-fake"));
    let manifest = imt_plugin_host::builtins::load_manifests(exe)
        .unwrap()
        .into_iter()
        .find(|m| &*m.id == ID)
        .unwrap();
    let mut spec = imt_plugin_host::PluginSpec::new(exe, ID);
    spec.elevate = false;
    let process = imt_plugin_host::PluginProcess::new(manifest, spec);
    let mut r = PluginRegistry::new();
    r.install_shared(Arc::new(imt_plugin_host::RemotePlugin::new(process.clone())))
        .unwrap();
    r.resolve();
    let data = tempfile::tempdir().unwrap();
    let ctx = ToolCtx::new(Some(data.path().to_path_buf()), Arc::new(NoTasks));
    let normal = pid(&r);
    let out = r.tools().invoke("fake.tool", "via-admin", json!({}), &ctx).unwrap();
    // It ran in the other (elevated) process, with the host's data folder, and cannot elevate further.
    assert_ne!(out["pid"].as_u64().unwrap(), normal, "{out}");
    assert_eq!(out["dataDir"], json!(data.path().to_string_lossy()));
    assert_eq!(out["canElevate"], json!(false));
    // Only operations the manifest declares elevated: the host refuses anything else.
    let e = r
        .tools()
        .invoke("fake.tool", "via-admin", json!({"op": "pid"}), &ctx)
        .unwrap_err()
        .to_string();
    assert!(e.contains("not an elevated operation"), "{e}");
    process.shutdown();
}

/// Task reports with the time they arrived.
#[derive(Default)]
struct Reports(Mutex<Vec<(String, Instant)>>);

impl TaskSink for Reports {
    fn started(&self, tool: &str, task: &ToolTask) {
        self.0.lock().unwrap().push((format!("started {tool} {}", task.id), Instant::now()));
    }
    fn progress(&self, tool: &str, task: &str, fraction: Option<f32>) {
        self.0
            .lock()
            .unwrap()
            .push((format!("progress {tool} {task} {fraction:?}"), Instant::now()));
    }
    fn finished(&self, tool: &str, task: &str) {
        self.0.lock().unwrap().push((format!("finished {tool} {task}"), Instant::now()));
    }
}

#[test]
fn an_elevated_operation_reports_progress_while_it_runs_not_with_its_answer() {
    // The elevated role as an ordinary process (the administrator prompt cannot run in tests).
    let exe = Path::new(env!("CARGO_BIN_EXE_imt-plugin-host-fake"));
    let manifest = imt_plugin_host::builtins::load_manifests(exe)
        .unwrap()
        .into_iter()
        .find(|m| &*m.id == ID)
        .unwrap();
    let mut spec = imt_plugin_host::PluginSpec::new(exe, ID);
    spec.elevate = false;
    let process = imt_plugin_host::PluginProcess::new(manifest, spec);
    let mut r = PluginRegistry::new();
    r.install_shared(Arc::new(imt_plugin_host::RemotePlugin::new(process.clone())))
        .unwrap();
    r.resolve();
    let reports = Arc::new(Reports::default());
    let ctx = ToolCtx::new(None, reports.clone());
    for op in [json!({"op": "admin-steps"}), json!(null)] {
        // Through the normal process (`ToolCtx::elevated`), and called directly in the elevated process.
        reports.0.lock().unwrap().clear();
        let out = if op.is_null() {
            r.tools().invoke("fake.tool", "admin-steps", json!({}), &ctx).unwrap()
        } else {
            r.tools().invoke("fake.tool", "via-admin", op, &ctx).unwrap()
        };
        let answered = Instant::now();
        assert_eq!(out, json!("done"));
        let got = reports.0.lock().unwrap().clone();
        let names: Vec<&str> = got.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            names,
            [
                "started fake.tool steps",
                "progress fake.tool steps Some(0.5)",
                "progress fake.tool steps Some(1.0)",
                "finished fake.tool steps"
            ]
        );
        // The first progress arrived while the operation was still working (it pauses 600 ms before answering).
        let early = answered.duration_since(got[1].1);
        assert!(early >= Duration::from_millis(400), "progress arrived {early:?} before the answer");
    }
    process.shutdown();
}
