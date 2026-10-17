use chrono::{Local, TimeZone};
use imt_collect::*;
use std::cell::RefCell;
use std::fs::{self, File};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::Duration;

#[derive(Clone)]
enum Behavior {
    /// Write the output file the tool is asked for (content given) and exit 0.
    Write(Vec<u8>),
    /// Exit non-zero without producing a file.
    Fail(String),
    Timeout,
    Missing,
    IoError,
    Stdout(Vec<u8>),
}

struct Fake {
    behaviors: Vec<(&'static str, Behavior)>,
    calls: RefCell<Vec<(String, Vec<String>)>>,
}

impl Fake {
    fn new(b: Vec<(&'static str, Behavior)>) -> Fake {
        Fake {
            behaviors: b,
            calls: RefCell::new(vec![]),
        }
    }
}

fn output_arg(program: &str, args: &[String]) -> Option<String> {
    match program {
        "wevtutil" | "reg" => args.get(2).cloned(),
        "msinfo32" => args.get(1).cloned(),
        "typeperf" => args.iter().position(|a| a == "-o").and_then(|i| args.get(i + 1)).cloned(),
        _ => None,
    }
}

impl CommandRunner for Fake {
    fn run(&self, program: &str, args: &[String], _t: Duration) -> Result<CommandOutput, CommandError> {
        self.calls.borrow_mut().push((program.to_string(), args.to_vec()));
        let b = self
            .behaviors
            .iter()
            .find(|(p, _)| *p == program)
            .map(|(_, b)| b.clone())
            .unwrap_or(Behavior::Missing);
        match b {
            Behavior::Write(data) => {
                if let Some(f) = output_arg(program, args) {
                    fs::write(f, data).unwrap();
                }
                Ok(CommandOutput {
                    exit_code: Some(0),
                    ..Default::default()
                })
            }
            Behavior::Stdout(data) => Ok(CommandOutput {
                exit_code: Some(0),
                stdout: data,
                stderr: String::new(),
            }),
            Behavior::Fail(msg) => Ok(CommandOutput {
                exit_code: Some(5),
                stdout: vec![],
                stderr: msg,
            }),
            Behavior::Timeout => Err(CommandError::TimedOut(program.into(), 1)),
            Behavior::Missing => Err(CommandError::NotFound(program.into())),
            Behavior::IoError => Err(CommandError::Io(program.into(), "denied".into())),
        }
    }
}

struct Recorder(RefCell<Vec<(String, usize, usize)>>);
impl Progress for Recorder {
    fn step(&self, name: &str, done: usize, total: usize) {
        self.0.borrow_mut().push((name.into(), done, total));
    }
}

fn collector<'a>(plan: CollectPlan, r: &'a dyn CommandRunner, p: &'a dyn Progress, platform: Platform) -> Collector<'a> {
    let mut c = Collector::new(plan, r, p);
    c.platform = platform;
    c.host = "SRV-01".into();
    c.now = Local.with_ymd_and_hms(2026, 3, 4, 5, 6, 7).unwrap();
    c
}

fn plan_with(json: &str) -> CollectPlan {
    CollectPlan::parse(json).unwrap()
}

fn read_zip(path: &Path, name: &str) -> Vec<u8> {
    let mut z = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    let mut buf = Vec::new();
    z.by_name(name).unwrap().read_to_end(&mut buf).unwrap();
    buf
}

fn names(path: &Path) -> Vec<String> {
    let mut z = zip::ZipArchive::new(File::open(path).unwrap()).unwrap();
    (0..z.len()).map(|i| z.by_index(i).unwrap().name().unwrap().to_string()).collect()
}

fn logs_dir() -> tempfile::TempDir {
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("app")).unwrap();
    fs::write(
        d.path().join("app/service.log"),
        "2026-03-01 10:00:00 INFO login bob@example.com password=hunter22 from 10.1.2.3\n",
    )
    .unwrap();
    fs::write(d.path().join("app/other.txt"), "plain\n").unwrap();
    fs::write(d.path().join("app/skip.dat"), "zzz").unwrap();
    d
}

fn plan_for(d: &Path, extra: &str) -> CollectPlan {
    plan_with(&format!(
        r#"{{"name":"acme","environment":true,"eventLogs":["System"],"logs":[{{"path":{:?},"include":["*.log","*.txt"]}}]{extra}}}"#,
        d.join("app").display().to_string()
    ))
}

#[test]
fn bundle_layout_manifest_and_project() {
    let d = logs_dir();
    let out = tempfile::tempdir().unwrap();
    let rec = Recorder(RefCell::new(vec![]));
    let fake = Fake::new(vec![]);
    let c = collector(plan_for(d.path(), ""), &fake, &rec, Platform::Linux);
    let res = c.run(&out.path().join("nested/out")).unwrap();

    assert_eq!(
        res.bundle.file_name().unwrap().to_str().unwrap(),
        "acme-SRV-01-20260304-050607.imtz"
    );
    assert!(res.bundle.exists());
    assert!(!res.bundle.with_extension("imtz.part").exists());
    let n = names(&res.bundle);
    for want in [
        "manifest.json",
        "project.imtproj",
        "system/environment.json",
        "logs/app/service.log",
        "logs/app/other.txt",
    ] {
        assert!(n.contains(&want.to_string()), "{want} missing from {n:?}");
    }
    assert!(!n.iter().any(|x| x.ends_with("skip.dat")));

    // Redacted text, counts in the manifest.
    let log = String::from_utf8(read_zip(&res.bundle, "logs/app/service.log")).unwrap();
    assert!(
        log.contains("[email]") && log.contains("password=[REDACTED]") && log.contains("[ipv4]"),
        "{log}"
    );
    assert!(!log.contains("hunter22"));
    let m: Manifest = serde_json::from_slice(&read_zip(&res.bundle, "manifest.json")).unwrap();
    assert_eq!(m.schema, MANIFEST_SCHEMA);
    assert_eq!(m.host, "SRV-01");
    assert_eq!(m.platform, "linux");
    assert_eq!(m.redaction["email"], 1);
    assert_eq!(m.redaction["password"], 1);
    assert_eq!(m.redaction["ipv4"], 1);
    let item = m.items.iter().find(|i| i.path == "logs/app/service.log").unwrap();
    assert!(item.redacted && item.redactions == 3 && item.kind == ItemKind::Log);
    assert!(item.original_bytes > 0 && item.bytes > 0);
    assert!(m
        .skipped
        .iter()
        .any(|s| s.kind == ItemKind::EventLog && s.reason.contains("only available on Windows")));
    assert!(m.errors.is_empty(), "{:?}", m.errors);
    assert_eq!(m.plan.name, "acme");
    assert!(m.tool_versions.contains_key("imt-collect"));
    assert!(m.steps.iter().any(|s| s.name == "environment"));
    assert_eq!(m.total_bytes, m.items.iter().map(|i| i.bytes).sum::<u64>());
    assert_eq!(m.max_bundle_bytes, 500 << 20);

    // Environment snapshot content.
    let env: serde_json::Value = serde_json::from_slice(&read_zip(&res.bundle, "system/environment.json")).unwrap();
    assert!(env["memory"]["totalBytes"].as_u64().unwrap() > 0);
    assert!(env["cpu"]["logicalCores"].as_u64().unwrap() > 0);
    assert_eq!(env["imtVersion"], env!("CARGO_PKG_VERSION"));
    assert!(env["environment"].is_object() && env["disks"].is_array());

    // Progress ends at done == total.
    let steps = rec.0.borrow();
    assert_eq!(steps.last().unwrap().1, steps.last().unwrap().2);
    assert_eq!(steps.first().unwrap().0, "environment");

    // The project file loads and points at the folders.
    let ex = tempfile::tempdir().unwrap();
    fs::write(ex.path().join("project.imtproj"), read_zip(&res.bundle, "project.imtproj")).unwrap();
    let p = imt_core::project::ProjectFile::load(&ex.path().join("project.imtproj")).unwrap();
    assert_eq!(p.name, "acme");
    let base = fs::canonicalize(ex.path()).unwrap();
    assert_eq!(
        p.sources,
        vec![base.join("logs").display().to_string(), base.join("system").display().to_string()]
    );
    assert_eq!(p.profile, imt_core::DEFAULT_PROFILE_TOML);
    assert_eq!(p.ui, serde_json::json!({}));
}

#[test]
fn bundle_opens_in_workspace() {
    let d = logs_dir();
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::new(vec![]);
    let c = collector(plan_for(d.path(), ""), &fake, &NoProgress, Platform::Linux);
    let res = c.run(out.path()).unwrap();
    let mut ws = imt_core::Workspace::new(imt_core::Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&res.bundle)).unwrap();
    let s = ws.summary();
    assert!(
        s.sources.iter().any(|x| x.path.contains("logs/app/service.log")),
        "{:?}",
        s.sources.iter().map(|x| &x.path).collect::<Vec<_>>()
    );
    assert!(s.total >= 1);
}

#[test]
fn windows_steps_with_fake_runner() {
    let out = tempfile::tempdir().unwrap();
    let rec = Recorder(RefCell::new(vec![]));
    let fake = Fake::new(vec![
        ("wevtutil", Behavior::Write(vec![0xFF, 0x00, 0x01, 0x02, 0x00])),
        ("msinfo32", Behavior::Write(b"<nfo>user mail a@b.com</nfo>".to_vec())),
        (
            "reg",
            Behavior::Write(b"Windows Registry\r\n\"Password\"=\"hunter22hunter\"".to_vec()),
        ),
        ("typeperf", Behavior::Write(b"\"t\",\"cpu\"\n\"1\",\"2\"\n".to_vec())),
    ]);
    let plan = plan_with(
        r#"{"name":"w","environment":false,"eventLogs":["System","Application"],"eventLogDays":2,"msinfo":true,
            "registry":["HKLM\\SOFTWARE\\Contoso"],"counters":true,"counterPaths":["\\Memory\\Available MBytes"]}"#,
    );
    let c = collector(plan, &fake, &rec, Platform::Windows);
    let res = c.run(out.path()).unwrap();
    let n = names(&res.bundle);
    for want in [
        "eventlogs/System.evtx",
        "eventlogs/Application.evtx",
        "system/msinfo.nfo",
        "system/registry-HKLM-SOFTWARE-Contoso.reg",
        "system/counters.csv",
    ] {
        assert!(n.contains(&want.to_string()), "{want} in {n:?}");
    }
    let calls = fake.calls.borrow();
    let w = calls.iter().find(|(p, _)| p == "wevtutil").unwrap();
    assert_eq!(w.1[0], "epl");
    assert_eq!(w.1[1], "System");
    assert_eq!(w.1[3], "/q:*[System[TimeCreated[timediff(@SystemTime) <= 172800000]]]");
    let r = calls.iter().find(|(p, _)| p == "reg").unwrap();
    assert_eq!(
        (r.1[0].as_str(), r.1[1].as_str(), r.1[3].as_str()),
        ("export", "HKLM\\SOFTWARE\\Contoso", "/y")
    );
    let t = calls.iter().find(|(p, _)| p == "typeperf").unwrap();
    assert_eq!(&t.1[..3], &["-sc".to_string(), "5".into(), "\\Memory\\Available MBytes".into()]);

    let m = res.manifest;
    let evtx = m.items.iter().find(|i| i.path == "eventlogs/System.evtx").unwrap();
    assert!(!evtx.redacted);
    assert_eq!(evtx.note.as_deref(), Some("binary file, not redacted"));
    let reg = m.items.iter().find(|i| i.kind == ItemKind::Registry).unwrap();
    assert!(reg.redacted);
    assert!(m.commands.iter().all(|c| c.status == CommandStatus::Ok));
    assert!(m.errors.is_empty());
    let reg_text = String::from_utf8(read_zip(&res.bundle, &reg.path)).unwrap();
    assert!(!reg_text.contains("hunter22"));
    // Only event logs, system folders exist; project sources reflect that.
    let p: serde_json::Value = serde_json::from_slice(&read_zip(&res.bundle, "project.imtproj")).unwrap();
    assert_eq!(p["sources"], serde_json::json!(["eventlogs", "system"]));
}

#[test]
fn tool_failures_are_recorded_not_fatal() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::new(vec![
        ("wevtutil", Behavior::Fail("Access is denied.".into())),
        ("msinfo32", Behavior::Timeout),
        ("reg", Behavior::Missing),
        ("typeperf", Behavior::Fail(String::new())),
    ]);
    let plan = plan_with(r#"{"environment":false,"eventLogs":["Security"],"msinfo":true,"registry":["HKLM\\X"],"counters":true}"#);
    let c = collector(plan, &fake, &NoProgress, Platform::Windows);
    let res = c.run(out.path()).unwrap();
    let m = &res.manifest;
    assert!(m.items.is_empty());
    let status: Vec<_> = m.commands.iter().map(|c| c.status).collect();
    assert_eq!(
        status,
        vec![
            CommandStatus::Failed,
            CommandStatus::TimedOut,
            CommandStatus::NotFound,
            CommandStatus::Failed
        ]
    );
    assert_eq!(m.commands[0].stderr, "Access is denied.");
    assert_eq!(m.commands[0].exit_code, Some(5));
    assert_eq!(m.errors.len(), 4);
    assert!(m.errors[0].contains("Access is denied."));
    assert!(m.errors[1].contains("did not finish"));
    assert!(m.errors[2].contains("not found"));
    assert!(m.errors[3].contains("Failed"));
    assert_eq!(m.skipped.len(), 4);
    assert!(m.skipped[0].reason.contains("no output file"));
    // Manifest in the bundle agrees; project has no sources.
    let p: serde_json::Value = serde_json::from_slice(&read_zip(&res.bundle, "project.imtproj")).unwrap();
    assert_eq!(p["sources"], serde_json::json!([]));
}

#[test]
fn windows_only_steps_are_skipped_elsewhere() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::new(vec![]);
    let plan = plan_with(
        r#"{"environment":false,"eventLogs":["System"],"msinfo":true,"registry":["HKLM\\X"],"counters":true,"macosUnifiedLog":true,"linuxJournal":true}"#,
    );
    let c = collector(plan, &fake, &NoProgress, Platform::Other);
    let res = c.run(out.path()).unwrap();
    assert!(fake.calls.borrow().is_empty());
    let kinds: Vec<_> = res.manifest.skipped.iter().map(|s| s.kind).collect();
    assert_eq!(
        kinds,
        vec![
            ItemKind::EventLog,
            ItemKind::Msinfo,
            ItemKind::Registry,
            ItemKind::Counters,
            ItemKind::UnifiedLog,
            ItemKind::Journal
        ]
    );
    assert!(res.manifest.skipped.iter().all(|s| s.reason.contains("only available on")));
}

#[test]
fn unix_extras_use_stdout() {
    let out = tempfile::tempdir().unwrap();
    let plan = plan_with(r#"{"environment":false,"eventLogs":[],"macosUnifiedLog":true,"linuxJournal":true,"recentLogHours":3}"#);

    let fake = Fake::new(vec![("log", Behavior::Stdout(br#"[{"m":"user a@b.com"}]"#.to_vec()))]);
    let res = collector(plan.clone(), &fake, &NoProgress, Platform::MacOs)
        .run(out.path())
        .unwrap();
    assert_eq!(fake.calls.borrow()[0].1, vec!["show", "--last", "3h", "--style", "json"]);
    assert!(names(&res.bundle).contains(&"system/unified-log.json".to_string()));
    assert!(String::from_utf8(read_zip(&res.bundle, "system/unified-log.json"))
        .unwrap()
        .contains("[email]"));
    assert!(res.manifest.skipped.iter().any(|s| s.kind == ItemKind::Journal));

    let fake = Fake::new(vec![("journalctl", Behavior::Stdout(b"{\"MESSAGE\":\"hi\"}\n".to_vec()))]);
    let res = collector(plan.clone(), &fake, &NoProgress, Platform::Linux)
        .run(out.path())
        .unwrap();
    assert_eq!(fake.calls.borrow()[0].1, vec!["-o", "json", "--since", "-3h"]);
    assert!(names(&res.bundle).contains(&"system/journal.json".to_string()));

    let fake = Fake::new(vec![("journalctl", Behavior::Fail("boom".into()))]);
    let res = collector(plan, &fake, &NoProgress, Platform::Linux).run(out.path()).unwrap();
    assert!(res
        .manifest
        .skipped
        .iter()
        .any(|s| s.kind == ItemKind::Journal && s.reason.contains("failed")));
    assert!(res.manifest.errors[0].contains("boom"));
}

#[test]
fn redaction_disabled_copies_bytes_untouched() {
    let d = logs_dir();
    let out = tempfile::tempdir().unwrap();
    let mut plan = plan_for(d.path(), r#","redact":{"builtIn":false}"#);
    plan.environment = false;
    let res = collector(plan, &Fake::new(vec![]), &NoProgress, Platform::Linux)
        .run(out.path())
        .unwrap();
    let log = String::from_utf8(read_zip(&res.bundle, "logs/app/service.log")).unwrap();
    assert!(log.contains("hunter22"));
    let it = res.manifest.items.iter().find(|i| i.path.ends_with("service.log")).unwrap();
    assert!(!it.redacted);
    assert_eq!(it.note.as_deref(), Some("redaction disabled"));
    assert!(res.manifest.redaction.is_empty());
}

#[test]
fn max_bundle_mb_drops_oldest_logs_first() {
    let d = tempfile::tempdir().unwrap();
    let mb = 1 << 20;
    let now = std::time::SystemTime::now();
    for (name, age) in [("new.log", 1u64), ("mid.log", 2), ("old.log", 3)] {
        let p = d.path().join(name);
        fs::write(&p, vec![b'x'; mb * 6 / 10]).unwrap();
        File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(now - Duration::from_secs(age * 86_400))
            .unwrap();
    }
    let out = tempfile::tempdir().unwrap();
    let plan = plan_with(&format!(
        r#"{{"environment":false,"eventLogs":[],"maxBundleMb":1,"logs":[{{"path":{:?}}}]}}"#,
        d.path().display().to_string()
    ));
    let res = collector(plan, &Fake::new(vec![]), &NoProgress, Platform::Linux)
        .run(out.path())
        .unwrap();
    let logs: Vec<_> = res.manifest.items.iter().map(|i| i.path.clone()).collect();
    assert_eq!(logs.len(), 1, "{logs:?}");
    assert!(logs[0].ends_with("new.log"));
    let skipped: Vec<_> = res.manifest.skipped.iter().map(|s| s.what.clone()).collect();
    assert_eq!(skipped.len(), 2);
    assert!(skipped[0].ends_with("mid.log") && skipped[1].ends_with("old.log"));
    assert!(res.manifest.skipped.iter().all(|s| s.reason.contains("maxBundleMb")));
    assert!(res.manifest.total_bytes <= 1 << 20);
}

#[test]
fn non_log_items_also_respect_the_budget() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::new(vec![("wevtutil", Behavior::Write(vec![0u8; 2 << 20]))]);
    let plan = plan_with(r#"{"environment":false,"eventLogs":["System"],"maxBundleMb":1}"#);
    let res = collector(plan, &fake, &NoProgress, Platform::Windows).run(out.path()).unwrap();
    assert!(res.manifest.items.is_empty());
    assert!(res.manifest.skipped[0].reason.contains("maxBundleMb"));
}

#[test]
fn name_collisions_labels_and_odd_inputs() {
    let d = tempfile::tempdir().unwrap();
    for sub in ["a/app", "b/app"] {
        fs::create_dir_all(d.path().join(sub)).unwrap();
        fs::write(d.path().join(sub).join("x.log"), "hello\n").unwrap();
    }
    fs::write(d.path().join("a/single.log"), "one\n").unwrap();
    fs::create_dir_all(d.path().join("a/app/we!rd")).unwrap();
    fs::write(d.path().join("a/app/we!rd/q.log"), "q\n").unwrap();
    let out = tempfile::tempdir().unwrap();
    let plan = plan_with(&format!(
        r#"{{"environment":false,"eventLogs":[],"logs":[{{"path":{:?}}},{{"path":{:?}}},{{"path":{:?}}},{{"path":{:?}}},{{"path":{:?}}}]}}"#,
        d.path().join("a/app").display().to_string(),
        d.path().join("b/app").display().to_string(),
        d.path().join("a/single.log").display().to_string(),
        d.path().join("a/single.log").display().to_string(),
        d.path().join("missing").display().to_string(),
    ));
    let res = collector(plan, &Fake::new(vec![]), &NoProgress, Platform::Linux)
        .run(out.path())
        .unwrap();
    let n = names(&res.bundle);
    assert!(n.contains(&"logs/app/x.log".to_string()));
    assert!(n.contains(&"logs/app-2/x.log".to_string()));
    assert!(n.contains(&"logs/a/single.log".to_string()));
    assert!(n.contains(&"logs/a-2/single.log".to_string()), "{n:?}");
    assert!(n.iter().all(|x| !x.contains("!/")));
    assert!(res.manifest.skipped.iter().any(|s| s.reason == "path does not exist"));
}

#[test]
fn duplicate_entry_names_get_a_suffix() {
    // Two plan entries for the same folder produce the same bundle path twice.
    let d = tempfile::tempdir().unwrap();
    fs::create_dir_all(d.path().join("app")).unwrap();
    fs::write(d.path().join("app/x.log"), "a\n").unwrap();
    fs::write(d.path().join("app/noext"), "a\n").unwrap();
    let out = tempfile::tempdir().unwrap();
    let p = d.path().join("app").display().to_string();
    let plan = plan_with(&format!(
        r#"{{"environment":false,"eventLogs":[],"logs":[{{"path":{p:?},"include":[]}}]}}"#
    ));
    let fake = Fake::new(vec![]);
    let mut c = collector(plan, &fake, &NoProgress, Platform::Linux);
    c.plan.logs.push(c.plan.logs[0].clone());
    let res = c.run(out.path()).unwrap();
    let n = names(&res.bundle);
    assert!(
        n.contains(&"logs/app/x.log".to_string()) && n.contains(&"logs/app-2/x.log".to_string()),
        "{n:?}"
    );
    assert!(n.contains(&"logs/app/noext".to_string()));
}

#[test]
fn run_rejects_invalid_plan_and_unwritable_output() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::new(vec![]);
    let plan = CollectPlan {
        max_bundle_mb: 0,
        ..Default::default()
    };
    let c = collector(plan, &fake, &NoProgress, Platform::Linux);
    assert!(c.run(out.path()).unwrap_err().to_string().contains("maxBundleMb"));

    let mut plan = CollectPlan::default();
    plan.redact.patterns.push(CustomPattern {
        name: "x".into(),
        regex: "(".into(),
        replace: String::new(),
    });
    plan.environment = false;
    let c = collector(plan, &fake, &NoProgress, Platform::Linux);
    assert!(c.run(out.path()).is_err());

    // The output "folder" is a file.
    let file = out.path().join("f");
    fs::write(&file, "x").unwrap();
    let c = collector(CollectPlan::default(), &fake, &NoProgress, Platform::Linux);
    assert!(c.run(&file).unwrap_err().to_string().contains("cannot create"));

    // Output folder cannot hold the bundle.
    let c = collector(CollectPlan::default(), &fake, &NoProgress, Platform::Linux);
    let clash = out.path().join(c.bundle_file_name().replace(".imtz", ".imtz.part"));
    fs::create_dir(&clash).unwrap();
    assert!(c.run(out.path()).unwrap_err().to_string().contains("cannot create"));
}

#[test]
fn platform_labels() {
    assert_eq!(
        Platform::current().label(),
        if cfg!(target_os = "macos") {
            "macos"
        } else if cfg!(windows) {
            "windows"
        } else {
            "linux"
        }
    );
    for (p, l) in [
        (Platform::Windows, "windows"),
        (Platform::MacOs, "macos"),
        (Platform::Linux, "linux"),
        (Platform::Other, "other"),
    ] {
        assert_eq!(p.label(), l);
    }
}

#[test]
fn cli_prints_plan_and_bundle() {
    let d = logs_dir();
    let out = tempfile::tempdir().unwrap();
    let run = |args: &[&str]| -> (Result<(), (i32, String)>, String) {
        let mut buf = Vec::new();
        let args: Vec<String> = args.iter().map(|s| s.to_string()).collect();
        let r = imt_collect::cli::run(&args, &mut buf);
        (r, String::from_utf8(buf).unwrap())
    };
    let (r, o) = run(&["--print-default-plan"]);
    assert!(r.is_ok());
    assert!(CollectPlan::parse(&o).is_ok());
    let (r, o) = run(&["--help"]);
    assert!(r.is_ok() && o.contains("usage"));

    let plan_path = out.path().join("plan.json");
    fs::write(&plan_path, plan_for(d.path(), "").tap_json()).unwrap();
    let bundle_dir = out.path().join("b");
    let (r, o) = run(&["--plan", plan_path.to_str().unwrap(), "--out", bundle_dir.to_str().unwrap()]);
    assert!(r.is_ok());
    let printed = PathBuf::from(o.trim());
    assert!(printed.exists() && printed.extension().unwrap() == "imtz");

    // No plan: default plan (environment only).
    let (r, o) = run(&["--out", bundle_dir.to_str().unwrap()]);
    assert!(r.is_ok() && o.trim().ends_with(".imtz"));

    let (e, _) = run(&["--bogus"]);
    assert_eq!(e.unwrap_err().0, 2);
    assert_eq!(run(&["--plan"]).0.unwrap_err().0, 2);
    assert_eq!(run(&["--out"]).0.unwrap_err().0, 2);
    let (e, _) = run(&["--plan", out.path().join("none.json").to_str().unwrap()]);
    assert!(e.unwrap_err().1.contains("cannot read"));
    fs::write(&plan_path, "{bad").unwrap();
    let (e, _) = run(&["--plan", plan_path.to_str().unwrap()]);
    assert_eq!(e.as_ref().unwrap_err().0, 1);
    assert!(e.unwrap_err().1.contains("not valid JSON"));
    let file = out.path().join("afile");
    fs::write(&file, "x").unwrap();
    let (e, _) = run(&["--out", file.to_str().unwrap()]);
    assert_eq!(e.unwrap_err().0, 1);
}

trait TapJson {
    fn tap_json(&self) -> String;
}
impl TapJson for CollectPlan {
    fn tap_json(&self) -> String {
        serde_json::to_string(self).unwrap()
    }
}

#[test]
fn io_errors_from_runner_are_failures() {
    let out = tempfile::tempdir().unwrap();
    let fake = Fake::new(vec![("journalctl", Behavior::IoError)]);
    let plan = plan_with(r#"{"environment":false,"eventLogs":[],"linuxJournal":true}"#);
    let res = collector(plan, &fake, &NoProgress, Platform::Linux).run(out.path()).unwrap();
    assert_eq!(res.manifest.commands[0].status, CommandStatus::Failed);
    assert!(res.manifest.errors[0].contains("denied"));
}

#[cfg(unix)]
#[test]
fn colliding_sanitised_names_unreadable_and_huge_files() {
    use std::os::unix::fs::PermissionsExt;
    let d = tempfile::tempdir().unwrap();
    fs::write(d.path().join("a:b.log"), "one\n").unwrap();
    fs::write(d.path().join("a_b.log"), "two\n").unwrap();
    fs::write(d.path().join("a_b-2.log"), "three\n").unwrap();
    let locked = d.path().join("locked.log");
    fs::write(&locked, "secret\n").unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let huge = d.path().join("huge.log");
    File::create(&huge).unwrap().set_len(300 << 20).unwrap();
    let out = tempfile::tempdir().unwrap();
    let plan = plan_with(&format!(
        r#"{{"environment":false,"eventLogs":[],"maxBundleMb":1000,"logs":[{{"path":{:?}}}]}}"#,
        d.path().display().to_string()
    ));
    let fake = Fake::new(vec![]);
    let res = collector(plan, &fake, &NoProgress, Platform::Linux).run(out.path()).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o644)).unwrap();
    let n = names(&res.bundle);
    let base = d.path().file_name().unwrap().to_string_lossy().replace('.', "_");
    let _ = base;
    let logs: Vec<_> = n.iter().filter(|x| x.starts_with("logs/")).collect();
    assert_eq!(logs.iter().filter(|x| x.contains("a_b")).count(), 3, "{logs:?}");
    assert!(res.manifest.skipped.iter().any(|s| s.reason.contains("too large")));
    let unreadable = res.manifest.skipped.iter().any(|s| s.reason == "cannot be read");
    // Root can read mode-000 files; everyone else gets a manifest error.
    assert_eq!(unreadable, !res.manifest.errors.is_empty());
}

#[test]
fn binary_runs_end_to_end() {
    let exe = env!("CARGO_BIN_EXE_imt-collect");
    let out = std::process::Command::new(exe).arg("--print-default-plan").output().unwrap();
    assert!(out.status.success());
    assert!(CollectPlan::parse(&String::from_utf8_lossy(&out.stdout)).is_ok());
    let bad = std::process::Command::new(exe).arg("--nope").output().unwrap();
    assert_eq!(bad.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&bad.stderr).contains("imt-collect:"));
}
