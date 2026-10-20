//! Performance Monitor as tools: templates (anywhere) and collector sets (Windows).

use crate::ids::{PERFMON_SETS, PERFMON_TEMPLATES};
use crate::perfmon_control::{output_files, PerfmonController};
use crate::perfmon_template::{guidance, presets, CollectorTemplate};
use crate::settings::Knobs;
use crate::Runner;
use anyhow::{anyhow, bail, Result};
use imt_core::overwrite::check_overwrite;
use imt_core::plugin::tools::{OpSpec, Platform, Schema, Tool, ToolCtx, ToolFailure, ToolInfo, ToolTask};
use serde_json::{json, Value};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

const U32_MAX: i64 = u32::MAX as i64;

/// The shape of a [`CollectorTemplate`]; values are checked by `CollectorTemplate::validate`.
fn template_schema() -> Schema {
    let ids: Vec<String> = presets().into_iter().map(|p| p.id).collect();
    Schema::object()
        .optional("name", Schema::text(200))
        .optional("processes", Schema::list(Schema::text(260), 256))
        .optional("presets", Schema::list(Schema::one_of(ids), 16))
        .optional("extraCounters", Schema::list(Schema::text(1024), 512))
        .optional("sampleIntervalSecs", Schema::int(0, U32_MAX))
        .optional("format", Schema::one_of(["blg", "csv", "tsv"]))
        .optional("outputDir", Schema::text(4096))
        .optional("maxSizeMb", Schema::int(0, U32_MAX))
        .optional("durationSecs", Schema::int(0, U32_MAX))
}

fn parse_template(args: &Value) -> Result<CollectorTemplate> {
    serde_json::from_value(args.get("template").cloned().unwrap_or(Value::Null)).map_err(|e| anyhow!("invalid template: {e}"))
}

fn text<'a>(args: &'a Value, key: &str) -> &'a str {
    args.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// Counter presets and Data Collector Set XML; pure, so available everywhere.
pub(crate) struct TemplatesTool;

impl Tool for TemplatesTool {
    fn id(&self) -> &str {
        PERFMON_TEMPLATES
    }

    fn describe(&self) -> ToolInfo {
        ToolInfo::new(PERFMON_TEMPLATES, "Performance Monitor templates")
            .describe("Counter presets with guidance, and Data Collector Set XML with the logman commands to install it.")
            .op(OpSpec::new("presets", "Counter presets").budget(Some(Duration::from_secs(5))))
            .op(OpSpec::new("generate", "Generate the XML").args(
                Schema::object()
                    .field("template", template_schema())
                    .optional("xmlPath", Schema::text(4096))
                    .optional("overwrite", Schema::Bool),
            ))
    }

    fn invoke(&self, op: &str, args: Value, _ctx: &ToolCtx) -> Result<Value> {
        match op {
            "presets" => Ok(Value::Array(
                presets()
                    .into_iter()
                    .map(|p| {
                        let counters: Vec<Value> = p.counters.iter().map(|c| json!({ "path": c, "guidance": guidance(c) })).collect();
                        json!({ "id": p.id, "name": p.name, "description": p.description, "counters": counters })
                    })
                    .collect(),
            )),
            "generate" => {
                let template = parse_template(&args)?;
                let xml = template.to_xml()?;
                let path = match args.get("xmlPath").and_then(Value::as_str) {
                    Some(p) => p.to_string(),
                    None => format!("{}.xml", template.name),
                };
                // Written only into an existing folder (the save dialog's choice), never over a
                // file unless the user confirmed it (`overwrite`).
                if let Some(dir) = Path::new(&path).parent().filter(|d| !d.as_os_str().is_empty()) {
                    if dir.exists() {
                        check_overwrite(Path::new(&path), args.get("overwrite").and_then(Value::as_bool).unwrap_or(false))?;
                        std::fs::write(&path, &xml)?;
                    }
                }
                Ok(json!({ "xml": xml, "path": path, "commands": template.logman_commands(&path) }))
            }
            other => bail!("unknown operation \"{other}\""),
        }
    }
}

/// Installing and controlling collector sets with `logman` (Windows).
pub(crate) struct SetsTool {
    runner: Runner,
    windows: bool,
    knobs: Arc<Knobs>,
}

/// Id of a started collector set in the background task list.
fn task_id(name: &str) -> String {
    format!("perfmon:{name}")
}

impl SetsTool {
    pub fn new(runner: Runner, windows: bool, knobs: Arc<Knobs>) -> SetsTool {
        SetsTool { runner, windows, knobs }
    }

    fn controller(&self) -> PerfmonController {
        PerfmonController::with_platform(self.runner.clone(), self.windows).timeout(self.knobs.logman_timeout())
    }

    fn status(&self, c: &PerfmonController, name: &str) -> Result<Value> {
        Ok(serde_json::to_value(c.query(name)?)?)
    }
}

/// logman refusing for missing rights is a typed denial (the shell offers to restart elevated).
fn typed(e: anyhow::Error) -> anyhow::Error {
    match e.downcast_ref::<crate::perfmon_control::PerfmonError>() {
        Some(crate::perfmon_control::PerfmonError::AccessDenied) => anyhow::Error::new(ToolFailure::denied(e.to_string())),
        _ => e,
    }
}

impl Tool for SetsTool {
    fn id(&self) -> &str {
        PERFMON_SETS
    }

    fn describe(&self) -> ToolInfo {
        let logman = Some(Duration::from_secs(15 * 60));
        let name = || Schema::object().field("name", Schema::text(256));
        ToolInfo::new(PERFMON_SETS, "Performance Monitor collector sets")
            .describe("Creates, starts, stops and deletes Data Collector Sets with logman, and finds the logs they wrote.")
            .only_on(&[Platform::Windows])
            .needs_elevation()
            .op(OpSpec::new("install", "Install a collector set")
                .args(Schema::object().field("template", template_schema()))
                .budget(logman))
            .op(OpSpec::new("control", "Start, stop, delete or query a collector set")
                .args(name().field("action", Schema::one_of(["start", "stop", "delete", "query"])))
                .budget(logman))
            .op(OpSpec::new("outputs", "Log files of a collector set").args(name()).budget(logman))
    }

    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value> {
        let run = || -> Result<Value> {
            let c = self.controller();
            match op {
                "install" => {
                    let template = parse_template(&args)?;
                    let xml = template.to_xml()?;
                    let file = std::env::temp_dir().join(format!("imt-{}.xml", template.name));
                    std::fs::write(&file, xml)?;
                    let imported = c.import(&template.name, &file, true);
                    let _ = std::fs::remove_file(&file);
                    imported?;
                    self.status(&c, &template.name)
                }
                "control" => {
                    let name = text(&args, "name");
                    match text(&args, "action") {
                        "start" => {
                            c.start(name)?;
                            ctx.task_started(ToolTask {
                                id: task_id(name),
                                label: format!("PerfMon collection \u{201c}{name}\u{201d}"),
                                kind: "perfmon".into(),
                                stoppable: true,
                            });
                        }
                        "stop" => {
                            c.stop(name)?;
                            ctx.task_finished(&task_id(name));
                        }
                        "delete" => {
                            c.delete(name)?;
                            ctx.task_finished(&task_id(name));
                            return Ok(Value::Null);
                        }
                        _ => {}
                    }
                    self.status(&c, name)
                }
                "outputs" => {
                    let status = c.query(text(&args, "name"))?;
                    Ok(json!(output_files(&status)
                        .into_iter()
                        .map(|p| p.to_string_lossy().into_owned())
                        .collect::<Vec<_>>()))
                }
                other => bail!("unknown operation \"{other}\""),
            }
        };
        run().map_err(typed)
    }

    /// Stops a started collector set (`logman stop`).
    fn stop(&self, task: &str, ctx: &ToolCtx) -> Result<()> {
        let name = task
            .strip_prefix("perfmon:")
            .ok_or_else(|| anyhow!("\"{task}\" is not a collector set started by IMT"))?;
        self.controller().stop(name)?;
        ctx.task_finished(task);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::{fail, ok, FakeRunner};
    use imt_core::plugin::tools::{NoTasks, TaskSink};
    use std::sync::Mutex;

    #[derive(Default)]
    struct Tasks(Mutex<Vec<String>>);
    impl TaskSink for Tasks {
        fn started(&self, tool: &str, t: &ToolTask) {
            self.0
                .lock()
                .unwrap()
                .push(format!("+{tool} {} {} {} {}", t.id, t.label, t.kind, t.stoppable));
        }
        fn progress(&self, _: &str, _: &str, _: Option<f32>) {}
        fn finished(&self, tool: &str, task: &str) {
            self.0.lock().unwrap().push(format!("-{tool} {task}"));
        }
    }

    fn ctx() -> ToolCtx {
        ToolCtx::new(None, Arc::new(NoTasks))
    }

    #[test]
    fn presets_and_generated_xml() {
        let t = TemplatesTool;
        assert!(t.describe().check().is_ok());
        let p = t.invoke("presets", json!({}), &ctx()).unwrap();
        assert!(p.as_array().unwrap().iter().any(|x| x["id"] == "memory"));
        assert!(p[0]["counters"][0]["guidance"].is_string());
        let dir = std::env::temp_dir().join(format!("imt-tools-perfmon-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("t.xml").to_string_lossy().into_owned();
        let args = json!({ "template": { "name": "IMT-test", "processes": ["OrderService"] }, "xmlPath": path });
        t.describe().ops[1].args.validate(&args).unwrap();
        let out = t.invoke("generate", args, &ctx()).unwrap();
        assert!(out["xml"].as_str().unwrap().contains("<DataCollectorSet>"));
        assert!(std::fs::read_to_string(&path).unwrap().contains("OrderService"));
        assert!(out["commands"]
            .as_array()
            .unwrap()
            .iter()
            .any(|c| c.as_str().unwrap().contains("logman import")));
        // Without a path: the name, nothing written.
        let out = t.invoke("generate", json!({ "template": { "name": "Plain" } }), &ctx()).unwrap();
        assert_eq!(out["path"], "Plain.xml");
        let bad = t.invoke("generate", json!({ "template": { "name": "", "sampleIntervalSecs": 0 } }), &ctx());
        assert!(bad.unwrap_err().to_string().contains("collector name"));
        let bad = t.invoke("generate", json!({ "template": { "format": "pdf" } }), &ctx());
        assert!(bad.unwrap_err().to_string().starts_with("invalid template"));
        // UI-shaped templates (nulls for unset options) pass the schema.
        let ui = json!({ "template": { "name": "x", "processes": [], "presets": ["cpu"], "extraCounters": [], "sampleIntervalSecs": 15,
            "format": "csv", "outputDir": "C:\\PerfLogs", "maxSizeMb": null, "durationSecs": null } });
        t.describe().ops[1].args.validate(&ui).unwrap();
        assert!(t.describe().ops[1]
            .args
            .validate(&json!({ "template": { "presets": ["gpu"] } }))
            .is_err());
        assert!(t.invoke("nope", json!({}), &ctx()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn generated_xml_never_replaces_a_file_without_confirmation() {
        use imt_core::overwrite::FILE_EXISTS_SUFFIX;
        let t = TemplatesTool;
        let dir = std::env::temp_dir().join(format!("imt-tools-perfmon-ow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mine.xml");
        std::fs::write(&path, "keep me").unwrap();
        let args = |overwrite: Option<bool>| json!({ "template": { "name": "IMT-ow" }, "xmlPath": path, "overwrite": overwrite });
        for refused in [None, Some(false)] {
            t.describe().ops[1].args.validate(&args(refused)).unwrap();
            let e = t.invoke("generate", args(refused), &ctx()).unwrap_err();
            assert_eq!(format!("{e:#}"), format!("{}{FILE_EXISTS_SUFFIX}", path.display()));
            assert_eq!(std::fs::read_to_string(&path).unwrap(), "keep me");
        }
        let out = t.invoke("generate", args(Some(true)), &ctx()).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), out["xml"].as_str().unwrap());
        assert!(t.describe().ops[1]
            .args
            .validate(&json!({ "template": {}, "overwrite": "yes" }))
            .is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    fn sets(fake: &Arc<FakeRunner>, windows: bool) -> SetsTool {
        SetsTool::new(fake.clone(), windows, Arc::new(Knobs::default()))
    }

    #[test]
    fn collector_sets_run_logman_and_report_background_work() {
        let fake = Arc::new(FakeRunner::new(|_, args| match args[0].as_str() {
            "query" if args[1] == "Missing" => fail(1, "", "Data Collector Set was not found."),
            "query" => ok("Name: IMT\nStatus: Running\nRoot Path: C:\\none\n"),
            _ => ok(""),
        }));
        let tool = sets(&fake, true);
        assert!(tool.describe().check().is_ok() && tool.describe().needs_elevation);
        let tasks = Arc::new(Tasks::default());
        let ctx = ToolCtx::new(None, tasks.clone()).for_tool(PERFMON_SETS);
        let installed = tool.invoke("install", json!({ "template": { "name": "Missing" } }), &ctx);
        // Not existing yet: imported, then the status query fails like logman would.
        assert!(installed.unwrap_err().to_string().contains("logman failed"));
        assert!(fake.calls().iter().any(|c| c.starts_with("logman import -n Missing -xml")));
        let status = tool.invoke("install", json!({ "template": { "name": "IMT" } }), &ctx).unwrap();
        assert_eq!(status["status"], "running");
        let control = |action: &str| tool.invoke("control", json!({ "name": "IMT", "action": action }), &ctx);
        assert_eq!(control("start").unwrap()["status"], "running");
        assert_eq!(control("query").unwrap()["name"], "IMT");
        assert_eq!(control("stop").unwrap()["status"], "running");
        control("start").unwrap();
        tool.stop("perfmon:IMT", &ctx).unwrap();
        assert!(control("delete").unwrap().is_null());
        assert_eq!(
            *tasks.0.lock().unwrap(),
            [
                "+perfmon.sets perfmon:IMT PerfMon collection \u{201c}IMT\u{201d} perfmon true",
                "-perfmon.sets perfmon:IMT",
                "+perfmon.sets perfmon:IMT PerfMon collection \u{201c}IMT\u{201d} perfmon true",
                "-perfmon.sets perfmon:IMT",
                "-perfmon.sets perfmon:IMT",
            ]
        );
        assert!(tool
            .invoke("outputs", json!({ "name": "IMT" }), &ctx)
            .unwrap()
            .as_array()
            .unwrap()
            .is_empty());
        assert!(tool.stop("monitor", &ctx).unwrap_err().to_string().contains("not a collector set"));
        assert!(tool.invoke("nope", json!({}), &ctx).is_err());
        // The configured timeout reaches logman.
        assert!(fake.timeouts().iter().all(|t| *t == crate::perfmon_control::DEFAULT_TIMEOUT));
    }

    #[test]
    fn a_failed_start_reports_nothing_and_off_windows_nothing_runs() {
        let fake = Arc::new(FakeRunner::new(|_, _| fail(5, "", "Access is denied.")));
        let tasks = Arc::new(Tasks::default());
        let ctx = ToolCtx::new(None, tasks.clone());
        let e = sets(&fake, true).invoke("control", json!({ "name": "IMT", "action": "start" }), &ctx);
        let e = e.unwrap_err();
        assert!(e.to_string().contains("Access is denied"));
        let typed = e.downcast_ref::<ToolFailure>().expect("a typed denial");
        assert_eq!(typed.reason, imt_core::plugin::tools::FailureReason::Denied);
        assert!(tasks.0.lock().unwrap().is_empty());
        let fake = Arc::new(FakeRunner::new(|_, _| ok("")));
        let e = sets(&fake, false).invoke("outputs", json!({ "name": "IMT" }), &ctx);
        assert!(e.unwrap_err().to_string().contains("only be managed on Windows"));
        assert!(fake.calls().is_empty());
    }
}
