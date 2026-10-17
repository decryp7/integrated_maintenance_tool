//! Command-line front end: `imt-collect [--plan plan.json|plan.toml] [--out dir] [--print-default-plan]`.

use crate::{CollectPlan, Collector, Progress, SystemRunner};
use std::io::Write;
use std::path::PathBuf;

struct StderrProgress;

impl Progress for StderrProgress {
    fn step(&self, name: &str, done: usize, total: usize) {
        eprintln!("[{done}/{total}] {name}");
    }
}

const USAGE: &str = "usage: imt-collect [--plan plan.json|plan.toml] [--out dir] [--print-default-plan]";

/// Runs the CLI, writing normal output to `out`. The error carries the exit code and message.
pub fn run(args: &[String], out: &mut dyn Write) -> Result<(), (i32, String)> {
    let mut plan_path: Option<PathBuf> = None;
    let mut out_dir = PathBuf::from(".");
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "--print-default-plan" => {
                let _ = writeln!(out, "{}", CollectPlan::default_json());
                return Ok(());
            }
            "--plan" => plan_path = Some(it.next().ok_or((2, format!("--plan needs a file\n{USAGE}")))?.into()),
            "--out" => out_dir = it.next().ok_or((2, format!("--out needs a folder\n{USAGE}")))?.into(),
            "-h" | "--help" => {
                let _ = writeln!(out, "{USAGE}");
                return Ok(());
            }
            other => return Err((2, format!("unknown argument '{other}'\n{USAGE}"))),
        }
    }
    let plan = match plan_path {
        Some(p) => {
            let text = std::fs::read_to_string(&p).map_err(|e| (1, format!("cannot read {}: {e}", p.display())))?;
            CollectPlan::parse(&text).map_err(|e| (1, format!("{}: {e}", p.display())))?
        }
        None => CollectPlan::default(),
    };
    let collector = Collector::new(plan, &SystemRunner, &StderrProgress);
    let result = collector.run(&out_dir).map_err(|e| (1, format!("{e:#}")))?;
    let _ = writeln!(out, "{}", result.bundle.display());
    Ok(())
}
