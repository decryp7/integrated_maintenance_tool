use super::*;
use crate::plugin::{Manifest, Plugin, PluginRegistry};
use serde_json::json;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// A tool whose ops exercise every outcome.
struct Echo {
    id: &'static str,
    platforms: Vec<Platform>,
    stops: AtomicUsize,
}

fn echo(id: &'static str) -> Echo {
    Echo {
        id,
        platforms: Vec::new(),
        stops: AtomicUsize::new(0),
    }
}

impl Tool for Echo {
    fn id(&self) -> &str {
        self.id
    }

    fn describe(&self) -> ToolInfo {
        let mut info = ToolInfo::new(self.id, "Echo")
            .describe("Repeats its arguments.")
            .op(OpSpec::new("echo", "Echo").args(Schema::Any))
            .op(OpSpec::new("named", "Named").args(Schema::object().field("name", Schema::text(5))))
            .op(OpSpec::new("fail", "Fail"))
            .op(OpSpec::new("panic", "Panic"))
            .op(OpSpec::new("slow", "Slow").budget(Some(Duration::from_millis(50))))
            .op(OpSpec::new("unbounded", "Unbounded").budget(None))
            .op(OpSpec::new("start", "Start"));
        info.platforms = self.platforms.clone();
        info
    }

    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value> {
        match op {
            "echo" | "named" | "unbounded" => Ok(json!({ "op": op, "args": args, "dir": ctx.data_dir() })),
            "fail" => anyhow::bail!("logman failed: nope"),
            "panic" => panic!("tool bug"),
            "slow" => {
                std::thread::sleep(Duration::from_millis(400));
                Ok(Value::Null)
            }
            _ => {
                ctx.task_started(ToolTask {
                    id: "job".into(),
                    label: "A job".into(),
                    kind: "other".into(),
                    stoppable: true,
                });
                ctx.task_progress("job", Some(0.5));
                Ok(json!("started"))
            }
        }
    }

    fn stop(&self, task: &str, ctx: &ToolCtx) -> Result<()> {
        if task == "boom" {
            panic!("stop bug");
        }
        self.stops.fetch_add(1, Ordering::SeqCst);
        ctx.task_finished(task);
        Ok(())
    }
}

/// Uses the default `stop`.
struct Plain;

impl Tool for Plain {
    fn id(&self) -> &str {
        "plain"
    }
    fn describe(&self) -> ToolInfo {
        ToolInfo::new("plain", "Plain").op(OpSpec::new("go", "Go"))
    }
    fn invoke(&self, _: &str, _: Value, _: &ToolCtx) -> Result<Value> {
        Ok(Value::Null)
    }
}

struct ToolPlugin {
    id: &'static str,
    tools: Vec<Arc<dyn Tool>>,
    manifest: Option<Manifest>,
}

impl Plugin for ToolPlugin {
    fn id(&self) -> &'static str {
        self.id
    }
    fn name(&self) -> &'static str {
        "tools"
    }
    fn version(&self) -> &'static str {
        "1.0.0"
    }
    fn manifest(&self) -> Manifest {
        self.manifest.clone().unwrap_or_else(|| Manifest::new(self.id, "1.0.0"))
    }
    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        for t in &self.tools {
            registry.add_tool(t.clone())?;
        }
        Ok(())
    }
}

fn plugin(id: &'static str, tools: Vec<Arc<dyn Tool>>) -> ToolPlugin {
    ToolPlugin { id, tools, manifest: None }
}

#[derive(Default)]
struct Recorder(Mutex<Vec<String>>);

impl TaskSink for Recorder {
    fn started(&self, tool: &str, task: &ToolTask) {
        self.0
            .lock()
            .unwrap()
            .push(format!("started {tool}/{} {} ({})", task.id, task.label, task.kind));
    }
    fn progress(&self, tool: &str, task: &str, fraction: Option<f32>) {
        self.0.lock().unwrap().push(format!("progress {tool}/{task} {fraction:?}"));
    }
    fn finished(&self, tool: &str, task: &str) {
        self.0.lock().unwrap().push(format!("finished {tool}/{task}"));
    }
}

fn ctx() -> ToolCtx {
    ToolCtx::new(Some(PathBuf::from("/data")), Arc::new(NoTasks))
}

fn host_with(tool: Arc<dyn Tool>) -> (PluginRegistry, ToolHost) {
    let mut r = PluginRegistry::empty();
    r.install(&plugin("t.tools", vec![tool])).unwrap();
    let host = r.tools();
    (r, host)
}

#[test]
fn schema_accepts_well_formed_arguments() {
    let s = Schema::object()
        .field("name", Schema::text(5))
        .optional("n", Schema::int(1, 10))
        .optional("on", Schema::Bool)
        .optional("format", Schema::one_of(["csv", "blg"]))
        .optional("tags", Schema::list(Schema::text(3), 2))
        .optional("any", Schema::Any);
    s.validate(&json!({ "name": "héllo", "n": 10, "on": true, "format": "blg", "tags": ["a", "b"], "any": [1, {}] }))
        .unwrap();
    // Optional fields may be missing or null.
    s.validate(&json!({ "name": "x", "n": null })).unwrap();
    assert_eq!(Schema::object().open().validate(&json!({ "extra": 1 })), Ok(()));
}

#[test]
fn schema_errors_name_the_location_and_the_rule() {
    let s = Schema::object()
        .field("name", Schema::text(5))
        .optional("n", Schema::int(1, 10))
        .optional("format", Schema::one_of(["csv", "blg"]))
        .optional("tags", Schema::list(Schema::text(3), 2));
    let err = |v: Value| s.validate(&v).unwrap_err().to_string();
    assert_eq!(err(json!({})), "args.name is required");
    assert_eq!(err(json!({ "name": null })), "args.name is required");
    assert_eq!(err(json!({ "name": "toolong" })), "args.name must be at most 5 characters (got 7)");
    assert_eq!(err(json!({ "name": 3 })), "args.name must be text, not a number");
    assert_eq!(err(json!({ "name": "a", "n": 11 })), "args.n must be between 1 and 10 (got 11)");
    assert_eq!(
        err(json!({ "name": "a", "n": 1.5 })),
        "args.n must be a whole number between 1 and 10"
    );
    assert_eq!(
        err(json!({ "name": "a", "format": "tsv" })),
        "args.format \"tsv\" is not one of: csv, blg"
    );
    assert_eq!(err(json!({ "name": "a", "format": 1 })), "args.format must be text, not a number");
    assert_eq!(
        err(json!({ "name": "a", "tags": ["a", "b", "c"] })),
        "args.tags must have at most 2 items (got 3)"
    );
    assert_eq!(
        err(json!({ "name": "a", "tags": ["abcd"] })),
        "args.tags[0] must be at most 3 characters (got 4)"
    );
    assert_eq!(err(json!({ "name": "a", "tags": "a" })), "args.tags must be a list, not text");
    assert_eq!(err(json!({ "name": "a", "who": 1 })), "args.who is not a known argument");
    assert_eq!(err(json!([])), "args must be an object, not a list");
    assert_eq!(err(Value::Null), "args must be an object, not null");
    assert_eq!(
        Schema::Bool.validate(&json!({})).unwrap_err().message,
        "must be true or false, not an object"
    );
    assert_eq!(
        Schema::Bool.validate(&json!("x")).unwrap_err().message,
        "must be true or false, not text"
    );
    assert_eq!(
        Schema::int(0, 1).validate(&json!(true)).unwrap_err().message,
        "must be a whole number, not true/false"
    );
    // `Any` accepts everything, so it never appears in a message; its name still reads well.
    assert_eq!(Schema::Any.kind(), "any value");
    // Field builders only apply to objects.
    assert_eq!(
        Schema::text(1).field("x", Schema::Bool).optional("y", Schema::Bool).open(),
        Schema::text(1)
    );
}

#[test]
fn descriptions_are_checked() {
    let ok = ToolInfo::new("a.b-c_d:e", "T").op(OpSpec::new("go", "Go"));
    assert_eq!(ok.check(), Ok(()));
    assert!(ToolInfo::new("", "T").check().unwrap_err().contains("tool id \"\""));
    assert!(ToolInfo::new("a b", "T").check().unwrap_err().contains("must be 1-64"));
    assert!(ToolInfo::new(&"x".repeat(65), "T").check().is_err());
    assert!(ToolInfo::new("t", "T").check().unwrap_err().contains("has no operations"));
    let dup = ToolInfo::new("t", "T").op(OpSpec::new("go", "Go")).op(OpSpec::new("go", "Again"));
    assert!(dup.check().unwrap_err().contains("operation \"go\" is declared twice"));
    let bad_op = ToolInfo::new("t", "T").op(OpSpec::new("go now", "Go"));
    assert!(bad_op.check().unwrap_err().contains("operation name \"go now\""));
}

#[test]
fn registration_records_tools_and_refuses_bad_or_duplicate_ones() {
    let mut r = PluginRegistry::empty();
    r.install(&plugin("t.a", vec![Arc::new(echo("echo")), Arc::new(Plain)])).unwrap();
    let info = &r.plugins()[0];
    assert_eq!(info.contributes, ["tool:echo", "tool:plain"]);
    // The same tool id from another plugin is refused (that plugin's registration fails).
    let err = r.install(&plugin("t.b", vec![Arc::new(echo("echo"))])).unwrap_err().to_string();
    assert!(err.contains("a tool with id \"echo\" is already registered by \"t.a\""), "{err}");
    // A tool whose description names another id, or is invalid.
    struct Liar;
    impl Tool for Liar {
        fn id(&self) -> &str {
            "liar"
        }
        fn describe(&self) -> ToolInfo {
            ToolInfo::new("truth", "Liar").op(OpSpec::new("go", "Go"))
        }
        fn invoke(&self, _: &str, _: Value, _: &ToolCtx) -> Result<Value> {
            Ok(Value::Null)
        }
    }
    let err = r.install(&plugin("t.c", vec![Arc::new(Liar)])).unwrap_err().to_string();
    assert!(err.contains("tool \"liar\" describes itself as \"truth\""), "{err}");
    let err = r.install(&plugin("t.d", vec![Arc::new(echo("bad id"))])).unwrap_err().to_string();
    assert!(err.contains("tool id \"bad id\""), "{err}");
    let host = r.tools();
    assert!(!host.is_empty());
    assert_eq!(
        format!("{host:?}"),
        format!("ToolHost {{ tools: [\"echo\", \"plain\"], platform: {:?} }}", Platform::current())
    );
}

#[test]
fn invoke_validates_and_dispatches() {
    let (_r, host) = host_with(Arc::new(echo("echo")));
    let out = host.invoke("echo", "named", json!({ "name": "abc" }), &ctx()).unwrap();
    assert_eq!(out, json!({ "op": "named", "args": { "name": "abc" }, "dir": "/data" }));
    // No arguments at all is an empty object.
    assert_eq!(host.invoke("echo", "echo", Value::Null, &ctx()).unwrap()["args"], json!({}));
    assert_eq!(host.invoke("echo", "unbounded", json!({}), &ctx()).unwrap()["op"], "unbounded");

    let e = host.invoke("nope", "x", json!({}), &ctx()).unwrap_err();
    assert_eq!(e, ToolError::UnknownTool("nope".into()));
    assert_eq!(e.to_string(), "no installed plugin provides the tool \"nope\"");
    let e = host.invoke("echo", "dance", json!({}), &ctx()).unwrap_err();
    assert!(matches!(&e, ToolError::UnknownOp { op, .. } if op == "dance"));
    assert_eq!(
        e.to_string(),
        "tool \"echo\" has no operation \"dance\" (it has: echo, named, fail, panic, slow, unbounded, start)"
    );
    let e = host.invoke("echo", "named", json!({ "name": 1 }), &ctx()).unwrap_err();
    assert_eq!(e.to_string(), "echo named: args.name must be text, not a number");
    assert!(matches!(e, ToolError::InvalidArgs { .. }));
    let huge = json!({ "x": "y".repeat(MAX_ARGS_BYTES) });
    let e = host.invoke("echo", "echo", huge, &ctx()).unwrap_err().to_string();
    assert!(e.starts_with("echo echo: args are too large"), "{e}");

    // An ordinary error passes through and does not count against the plugin.
    let e = host.invoke("echo", "fail", json!({}), &ctx()).unwrap_err();
    assert_eq!(e, ToolError::Failed("logman failed: nope".into()));
    assert_eq!(_r.plugins()[0].failures, 0);
}

#[test]
fn platform_gating_lists_the_reason_instead_of_hiding() {
    let mut windows_only = echo("perfmon.sets");
    windows_only.platforms = vec![Platform::Windows];
    let (_r, host) = host_with(Arc::new(windows_only));
    let mac = host.clone().on_platform(Platform::Macos);
    let listed = mac.list();
    assert_eq!(
        listed[0].availability,
        Availability::Unsupported {
            reason: "Echo is only available on Windows (this is macOS)".into()
        }
    );
    let e = mac.invoke("perfmon.sets", "echo", json!({}), &ctx()).unwrap_err();
    assert_eq!(
        e,
        ToolError::Unsupported("Echo is only available on Windows (this is macOS)".into())
    );
    assert_eq!(e.to_string(), "Echo is only available on Windows (this is macOS)");
    let win = host.on_platform(Platform::Windows);
    assert_eq!(win.list()[0].availability, Availability::Available);
    assert!(win.invoke("perfmon.sets", "echo", json!({}), &ctx()).is_ok());
    let mut two = ToolInfo::new("t", "Two").only_on(&[Platform::Windows, Platform::Linux]);
    two.ops.push(OpSpec::new("go", "Go"));
    assert_eq!(
        two.unsupported_on(Platform::Macos).unwrap(),
        "Two is only available on Windows and Linux (this is macOS)"
    );
    assert_eq!(Platform::Linux.label(), "Linux");
    assert_eq!(Platform::current() == Platform::Windows, cfg!(windows));
}

#[test]
fn a_panicking_tool_is_contained_and_quarantined() {
    let (r, host) = host_with(Arc::new(echo("echo")));
    for _ in 0..crate::guard::QUARANTINE_AFTER {
        let e = host.invoke("echo", "panic", json!({}), &ctx()).unwrap_err().to_string();
        assert!(
            e.contains("crashed: tool bug") || e.contains("disabled after repeated crashes"),
            "{e}"
        );
    }
    let info = &r.plugins()[0];
    assert_eq!(info.status, crate::plugin::PluginStatus::Quarantined);
    match &host.list()[0].availability {
        Availability::PluginUnavailable { reason } => assert!(reason.contains("disabled after repeated crashes"), "{reason}"),
        other => panic!("{other:?}"),
    }
    let e = host.invoke("echo", "echo", json!({}), &ctx()).unwrap_err().to_string();
    assert!(e.contains("disabled after repeated crashes"), "{e}");
    // A reset makes it available again.
    r.reset_plugin("t.tools");
    assert_eq!(host.list()[0].availability, Availability::Available);
}

#[test]
fn an_op_past_its_budget_is_abandoned_and_marks_the_plugin_hung() {
    let (r, host) = host_with(Arc::new(echo("echo")));
    let started = std::time::Instant::now();
    let e = host.invoke("echo", "slow", json!({}), &ctx()).unwrap_err().to_string();
    assert!(started.elapsed() < Duration::from_millis(390), "abandoned, not awaited");
    assert!(e.contains("(tool echo slow) did not finish within 50 ms"), "{e}");
    assert!(r.plugins()[0].hung);
    match &host.list()[0].availability {
        Availability::PluginUnavailable { reason } => assert!(reason.contains("not responding"), "{reason}"),
        other => panic!("{other:?}"),
    }
    assert!(host.invoke("echo", "echo", json!({}), &ctx()).is_err(), "fails fast while hung");
    // Once the abandoned call returns, the plugin answers again.
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while r.plugins()[0].hung && std::time::Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(host.invoke("echo", "echo", json!({}), &ctx()).is_ok());
    assert_eq!(r.plugins()[0].failures, 1);
}

#[test]
fn an_inactive_plugin_lists_its_tools_with_the_reason() {
    let mut r = PluginRegistry::empty();
    let mut p = plugin("t.needy", vec![Arc::new(Plain)]);
    p.manifest = Some(Manifest::new("t.needy", "1.0.0").requires("t.absent", "^1"));
    r.install(&p).unwrap();
    r.resolve();
    let host = r.tools();
    match &host.list()[0].availability {
        Availability::PluginUnavailable { reason } => assert!(reason.contains("inactive: needs t.absent"), "{reason}"),
        other => panic!("{other:?}"),
    }
    let e = host.invoke("plain", "go", json!({}), &ctx()).unwrap_err().to_string();
    assert!(e.contains("is inactive"), "{e}");
}

#[test]
fn background_tasks_are_reported_with_the_tool_and_stopped_through_the_host() {
    let tool = Arc::new(echo("echo"));
    let (_r, host) = host_with(tool.clone());
    let rec = Arc::new(Recorder::default());
    let ctx = ToolCtx::new(None, rec.clone());
    assert_eq!(host.invoke("echo", "start", json!({}), &ctx).unwrap(), "started");
    host.stop("echo", "job", &ctx).unwrap();
    assert_eq!(
        *rec.0.lock().unwrap(),
        ["started echo/job A job (other)", "progress echo/job Some(0.5)", "finished echo/job"]
    );
    assert_eq!(tool.stops.load(Ordering::SeqCst), 1);
    let e = host.stop("echo", "boom", &ctx).unwrap_err().to_string();
    assert!(e.contains("crashed: stop bug"), "{e}");
    assert!(host.stop("nope", "job", &ctx).is_err());
    let (_r, plain) = host_with(Arc::new(Plain));
    let e = plain.stop("plain", "job", &ctx).unwrap_err().to_string();
    assert_eq!(e, "tool \"plain\" has no background task \"job\" to stop");
    // The null sink accepts everything.
    let quiet = ToolCtx::new(None, Arc::new(NoTasks));
    assert_eq!(host.invoke("echo", "start", json!({}), &quiet).unwrap(), "started");
    host.stop("echo", "job", &quiet).unwrap();
    assert!(quiet.data_dir().is_none());
}

#[test]
fn host_registered_tools_are_contained_too() {
    let mut r = PluginRegistry::empty();
    r.add_tool(Arc::new(echo("echo"))).unwrap();
    let host = r.tools();
    assert_eq!(host.list()[0].plugin, crate::plugin::services::HOST);
    assert_eq!(host.list()[0].availability, Availability::Available);
    let e = host.invoke("echo", "panic", json!({}), &ctx()).unwrap_err().to_string();
    assert!(e.contains("crashed: tool bug"), "{e}");
    assert!(host.invoke("echo", "slow", json!({}), &ctx()).is_ok(), "no guard, no budget");
    host.stop("echo", "job", &ctx()).unwrap();
}

#[test]
fn entries_serialise_for_the_ui() {
    let mut info = ToolInfo::new("perfmon.sets", "PerfMon")
        .only_on(&[Platform::Windows])
        .needs_elevation();
    info = info.op(OpSpec::new("control", "Control").args(
        Schema::object()
            .field("action", Schema::one_of(["start"]))
            .optional("n", Schema::int(0, 1))
            .optional("l", Schema::list(Schema::Bool, 3))
            .optional("t", Schema::text(9))
            .optional("a", Schema::Any),
    ));
    info = info.op(OpSpec::new("forever", "Forever").budget(None));
    let entry = ToolEntry {
        info,
        plugin: "net.decryptology.tools".into(),
        availability: Availability::Unsupported { reason: "no".into() },
    };
    let v = serde_json::to_value(&entry).unwrap();
    assert_eq!(v["id"], "perfmon.sets");
    assert_eq!(v["platforms"], json!(["windows"]));
    assert_eq!(v["needsElevation"], true);
    assert_eq!(v["plugin"], "net.decryptology.tools");
    assert_eq!(v["availability"], json!({ "state": "unsupported", "reason": "no" }));
    assert_eq!(v["ops"][0]["budgetMs"], 30_000);
    assert_eq!(v["ops"][1]["budgetMs"], Value::Null);
    let args = &v["ops"][0]["args"];
    assert_eq!(args["type"], "object");
    assert_eq!(args["additional"], false);
    assert_eq!(
        args["fields"][0],
        json!({ "name": "action", "schema": { "type": "enum", "values": ["start"] }, "required": true })
    );
    assert_eq!(args["fields"][1]["schema"], json!({ "type": "int", "min": 0, "max": 1 }));
    assert_eq!(
        args["fields"][2]["schema"],
        json!({ "type": "list", "items": { "type": "bool" }, "maxItems": 3 })
    );
    assert_eq!(args["fields"][3]["schema"], json!({ "type": "text", "maxLen": 9 }));
    assert_eq!(args["fields"][4]["schema"], json!({ "type": "any" }));
    let a = serde_json::to_value(Availability::PluginUnavailable { reason: "q".into() }).unwrap();
    assert_eq!(a, json!({ "state": "pluginUnavailable", "reason": "q" }));
    assert_eq!(
        serde_json::to_value(Availability::Available).unwrap(),
        json!({ "state": "available" })
    );
}

#[test]
fn failures_keep_the_reason_a_tool_or_the_os_gave() {
    let denied = anyhow::Error::new(ToolFailure::denied("run IMT elevated")).context("logman create");
    assert_eq!(failed(denied), ToolError::Denied("logman create: run IMT elevated".into()));
    let missing = anyhow::Error::new(ToolFailure::missing("wevtutil is not installed"));
    assert_eq!(failed(missing), ToolError::Missing("wevtutil is not installed".into()));
    let os = anyhow::Error::new(std::io::Error::new(std::io::ErrorKind::PermissionDenied, "no"));
    assert!(matches!(failed(os), ToolError::Denied(_)));
    assert_eq!(failed(anyhow::anyhow!("boom")), ToolError::Failed("boom".into()));
    assert_eq!(ToolError::Missing("m".into()).to_string(), "m");
    assert_eq!(ToolFailure::denied("d").to_string(), "d");
}
