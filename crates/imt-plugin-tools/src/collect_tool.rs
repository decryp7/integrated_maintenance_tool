//! The diagnostics collector as a tool: a redacted `.imtz` bundle.

use crate::ids::DIAGNOSTICS;
use crate::Runner;
use anyhow::{bail, Result};
use imt_core::plugin::tools::{OpSpec, Schema, Tool, ToolCtx, ToolInfo, ToolTask};
use serde_json::{json, Value};
use std::path::Path;
use std::time::Duration;

/// Id of a running collection in the background task list.
const TASK: &str = "collect";
/// Largest plan accepted (JSON or TOML text).
const MAX_PLAN: usize = 256 * 1024;

pub(crate) struct DiagnosticsTool {
    runner: Runner,
}

impl DiagnosticsTool {
    pub fn new(runner: Runner) -> DiagnosticsTool {
        DiagnosticsTool { runner }
    }

    fn collect(&self, args: &Value, ctx: &ToolCtx) -> Result<Value> {
        let text = |k: &str| args.get(k).and_then(Value::as_str).unwrap_or_default();
        let plan = imt_collect::CollectPlan::parse(text("plan"))?;
        if text("outDir").trim().is_empty() {
            bail!("choose an output folder");
        }
        ctx.task_started(ToolTask {
            id: TASK.into(),
            label: format!("Collecting diagnostics \u{201c}{}\u{201d}", plan.name),
            kind: "collect".into(),
            stoppable: false,
        });
        let progress = Progress(ctx);
        let mut collector = imt_collect::Collector::new(plan, self.runner.as_ref(), &progress);
        // An existing bundle of the same name is replaced only when the user confirmed it.
        collector.overwrite = args.get("overwrite").and_then(Value::as_bool).unwrap_or(false);
        let result = collector.run(Path::new(text("outDir")));
        ctx.task_finished(TASK);
        let result = result?;
        Ok(json!({ "bundle": result.bundle, "manifest": result.manifest }))
    }
}

/// Collector progress as the background task's progress.
struct Progress<'a>(&'a ToolCtx);

impl imt_collect::Progress for Progress<'_> {
    fn step(&self, _name: &str, done: usize, total: usize) {
        let fraction = (total > 0).then(|| done as f32 / total as f32);
        self.0.task_progress(TASK, fraction);
    }
}

impl Tool for DiagnosticsTool {
    fn id(&self) -> &str {
        DIAGNOSTICS
    }

    fn describe(&self) -> ToolInfo {
        ToolInfo::new(DIAGNOSTICS, "Diagnostics collector")
            .describe("Gathers logs, event logs, system information and counters into one redacted bundle (.imtz).")
            .op(OpSpec::new("default-plan", "The default plan").budget(Some(Duration::from_secs(5))))
            .op(OpSpec::new("collect", "Collect a bundle")
                .args(
                    Schema::object()
                        .field("plan", Schema::text(MAX_PLAN))
                        .field("outDir", Schema::text(4096))
                        .optional("overwrite", Schema::Bool),
                )
                // Each external program has its own timeout; the whole run gets an hour.
                .budget(Some(Duration::from_secs(60 * 60))))
    }

    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value> {
        match op {
            "default-plan" => Ok(json!(imt_collect::CollectPlan::default_json())),
            "collect" => self.collect(&args, ctx),
            other => bail!("unknown operation \"{other}\""),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fake::FakeRunner;
    use imt_collect::CommandError;
    use imt_core::plugin::tools::TaskSink;
    use std::sync::{Arc, Mutex};

    #[derive(Default)]
    struct Tasks(Mutex<Vec<String>>);
    impl TaskSink for Tasks {
        fn started(&self, _: &str, t: &ToolTask) {
            self.0.lock().unwrap().push(format!("+{} {}", t.id, t.stoppable));
        }
        fn progress(&self, _: &str, task: &str, f: Option<f32>) {
            self.0.lock().unwrap().push(format!("~{task} {f:?}"));
        }
        fn finished(&self, _: &str, task: &str) {
            self.0.lock().unwrap().push(format!("-{task}"));
        }
    }

    #[test]
    fn collects_a_bundle_with_fake_tools_and_reports_progress() {
        let root = std::env::temp_dir().join(format!("imt-tools-collect-{}", std::process::id()));
        let logs = root.join("logs");
        std::fs::create_dir_all(&logs).unwrap();
        std::fs::write(logs.join("app.log"), "2026-10-14 10:00:00 [INF] mail me at a@b.example\n").unwrap();
        // Every external program is "missing": the steps are skipped, the logs still collected.
        let fake = Arc::new(FakeRunner::new(|p, _| Err(CommandError::NotFound(p.into()))));
        let tool = DiagnosticsTool::new(fake.clone());
        assert!(tool.describe().check().is_ok());
        let tasks = Arc::new(Tasks::default());
        let ctx = ToolCtx::new(None, tasks.clone());
        let plan = json!({ "name": "case", "logs": [{ "path": logs }], "eventLogs": [], "environment": false }).to_string();
        let out = tool
            .invoke("collect", json!({ "plan": plan, "outDir": root.join("out") }), &ctx)
            .unwrap();
        assert!(out["bundle"].as_str().unwrap().ends_with(".imtz"));
        assert!(Path::new(out["bundle"].as_str().unwrap()).exists());
        assert_eq!(out["manifest"]["name"], "case");
        let seen = tasks.0.lock().unwrap().clone();
        assert_eq!(seen.first().unwrap(), "+collect false");
        assert_eq!(seen.last().unwrap(), "-collect");
        assert!(seen.iter().any(|s| s.starts_with("~collect Some(")), "{seen:?}");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn an_existing_bundle_is_replaced_only_when_confirmed() {
        use imt_core::overwrite::FILE_EXISTS_SUFFIX;
        let root = std::env::temp_dir().join(format!("imt-tools-collect-ow-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let fake = Arc::new(FakeRunner::new(|p, _| Err(CommandError::NotFound(p.into()))));
        let tool = DiagnosticsTool::new(fake);
        let tasks = Arc::new(Tasks::default());
        let ctx = ToolCtx::new(None, tasks.clone());
        let plan = json!({ "name": "same", "eventLogs": [], "environment": false }).to_string();
        let args = |overwrite: bool| json!({ "plan": plan, "outDir": root, "overwrite": overwrite });
        tool.describe().ops[1].args.validate(&args(true)).unwrap();
        // The bundle name ends in a one-second timestamp: take the first bundle's name and make
        // sure a file exists for every second of the next run.
        let first = tool.invoke("collect", args(false), &ctx).unwrap();
        let bundle = first["bundle"].as_str().unwrap().to_string();
        let prefix = &bundle[..bundle.len() - "yyyyMMdd-HHmmss.imtz".len()];
        let now = chrono::Local::now();
        for s in 0..10 {
            let at = now + chrono::Duration::seconds(s);
            let _ = std::fs::write(format!("{prefix}{}.imtz", at.format("%Y%m%d-%H%M%S")), "mine");
        }
        let refused = format!("{:#}", tool.invoke("collect", args(false), &ctx).unwrap_err());
        assert!(refused.starts_with(prefix) && refused.ends_with(FILE_EXISTS_SUFFIX), "{refused}");
        assert_eq!(tasks.0.lock().unwrap().last().unwrap(), "-collect");
        let again = tool.invoke("collect", args(true), &ctx).unwrap();
        assert_ne!(std::fs::read(again["bundle"].as_str().unwrap()).unwrap(), b"mine", "replaced");
        let _ = std::fs::remove_dir_all(root);
    }

    #[test]
    fn default_plan_and_errors() {
        let tool = DiagnosticsTool::new(Arc::new(FakeRunner::new(|p, _| Err(CommandError::NotFound(p.into())))));
        let tasks = Arc::new(Tasks::default());
        let ctx = ToolCtx::new(None, tasks.clone());
        let plan = tool.invoke("default-plan", json!({}), &ctx).unwrap();
        assert!(plan.as_str().unwrap().contains("redact"));
        assert!(tool.invoke("collect", json!({ "plan": "{ nope", "outDir": "/x" }), &ctx).is_err());
        let e = tool.invoke("collect", json!({ "plan": "{}", "outDir": " " }), &ctx).unwrap_err();
        assert_eq!(e.to_string(), "choose an output folder");
        // A collection that fails still ends its background task.
        let file = std::env::temp_dir().join(format!("imt-tools-collect-file-{}", std::process::id()));
        std::fs::write(&file, "not a folder").unwrap();
        assert!(tool
            .invoke("collect", json!({ "plan": "{}", "outDir": file.join("sub") }), &ctx)
            .is_err());
        assert_eq!(tasks.0.lock().unwrap().last().unwrap(), "-collect");
        let _ = std::fs::remove_file(file);
        assert!(tool.invoke("nope", json!({}), &ctx).is_err());
        let p = Progress(&ctx);
        imt_collect::Progress::step(&p, "x", 0, 0);
        assert_eq!(tasks.0.lock().unwrap().last().unwrap(), "~collect None");
    }
}
