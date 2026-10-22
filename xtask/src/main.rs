//! Repository chores for Debrief developers, run as `cargo xtask <task>` (alias in `.cargo/config.toml`).
//!
//! - `gen-samples`: regenerates every committed sample: the logs from `samples/generate.py`, then the
//!   profiler captures and the crash dump, which Rust tests write when `DEBRIEF_REGENERATE_SAMPLES=1`.
//! - `libduckdb`: fetches the pinned prebuilt libduckdb the store worker links and points Cargo at it
//!   (see [`libduckdb`]).

mod libduckdb;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

/// One command the task runs, kept as data so the plan can be tested without running anything.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Step {
    program: String,
    args: Vec<String>,
    env: Vec<(String, String)>,
}

impl Step {
    fn new(program: &str, args: &[&str]) -> Self {
        Step {
            program: program.to_owned(),
            args: args.iter().map(|a| (*a).to_owned()).collect(),
            env: Vec::new(),
        }
    }

    fn with_env(mut self, key: &str, value: &str) -> Self {
        self.env.push((key.to_owned(), value.to_owned()));
        self
    }

    fn display(&self) -> String {
        let env: String = self.env.iter().map(|(k, v)| format!("{k}={v} ")).collect();
        format!("{env}{} {}", self.program, self.args.join(" "))
    }
}

/// Python launchers to try, in order. `python3` comes first (Linux, macOS); on Windows it may be the
/// Microsoft Store stub, which fails `--version`, so `python` and the `py -3` launcher follow.
const PYTHONS: &[&[&str]] = &[&["python3"], &["python"], &["py", "-3"]];

/// The Rust tests that write samples, as (crate, extra cargo args, test target).
const RUST_GENERATORS: &[(&str, &[&str], &str)] = &[
    ("debrief-plugin-profiling", &["--no-default-features"], "gen_json_samples"),
    ("debrief-plugin-profiling", &["--no-default-features"], "gen_nettrace"),
    ("debrief-plugin-profiling", &["--no-default-features"], "gen_pprof"),
    ("debrief-plugin-dumps", &[], "samples"),
    ("debrief-plugin-windows", &[], "samples"),
];

/// The full `gen-samples` plan, given the Python launcher to use (program and leading arguments).
fn gen_samples_plan(python: &[&str], root: &Path) -> Vec<Step> {
    let script = root.join("samples").join("generate.py");
    let script = script.to_string_lossy();
    let mut py_args: Vec<&str> = python[1..].to_vec();
    py_args.push(&script);
    let mut steps = vec![Step::new(python[0], &py_args)];
    for (krate, extra, test) in RUST_GENERATORS {
        let mut args = vec!["test", "-p", krate];
        args.extend_from_slice(extra);
        // Only the ignored `regenerate_*` tests write files; the rest of the target runs in the normal test suite.
        args.extend_from_slice(&["--test", test, "--", "--ignored", "regenerate_"]);
        steps.push(Step::new("cargo", &args).with_env("DEBRIEF_REGENERATE_SAMPLES", "1"));
    }
    steps
}

/// The first launcher in `candidates` for which `works` holds.
fn pick_python<'a>(candidates: &[&'a [&'a str]], works: impl Fn(&[&str]) -> bool) -> Option<&'a [&'a str]> {
    candidates.iter().copied().find(|c| works(c))
}

/// A child process sharing this console: xtask is a terminal tool, so the app's `quiet_command` (no console
/// window) is not wanted here.
#[allow(clippy::disallowed_methods)]
fn command(program: &str) -> Command {
    Command::new(program)
}

fn python_works(launcher: &[&str]) -> bool {
    command(launcher[0])
        .args(&launcher[1..])
        .arg("--version")
        .output()
        .is_ok_and(|o| o.status.success())
}

/// The repository root: the parent of this crate's directory.
fn repo_root() -> PathBuf {
    let dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    dir.parent().unwrap_or(dir).to_path_buf()
}

fn run(step: &Step, root: &Path) -> Result<(), String> {
    eprintln!("xtask: {}", step.display());
    let status = command(&step.program)
        .args(&step.args)
        .envs(step.env.iter().map(|(k, v)| (k, v)))
        .current_dir(root)
        .status()
        .map_err(|e| format!("could not start `{}`: {e}", step.program))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{}` failed ({status})", step.display()))
    }
}

fn gen_samples() -> Result<(), String> {
    let root = repo_root();
    let python = pick_python(PYTHONS, python_works)
        .ok_or("no working Python 3 found (tried python3, python, py -3); install Python 3 and put it on PATH")?;
    for step in gen_samples_plan(python, &root) {
        run(&step, &root)?;
    }
    eprintln!("xtask: samples regenerated; review and commit the changes under samples/ (and fuzz seeds)");
    Ok(())
}

const USAGE: &str = "usage: cargo xtask <task>\n\ntasks:\n  gen-samples   regenerate every committed sample (samples/generate.py and the Rust sample generators)\n  libduckdb     fetch the pinned prebuilt libduckdb and point Cargo at it (.cargo/libduckdb.toml)\n  help         show this message";

fn main() -> ExitCode {
    let task = std::env::args().nth(1);
    let result = match task.as_deref() {
        Some("gen-samples") => gen_samples(),
        Some("libduckdb") => libduckdb::run(&repo_root()),
        Some("help" | "-h" | "--help") | None => {
            println!("{USAGE}");
            Ok(())
        }
        Some(other) => Err(format!("unknown task `{other}`\n\n{USAGE}")),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("xtask: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn picks_the_first_working_python() {
        let picked = pick_python(PYTHONS, |c| c[0] != "python3");
        assert_eq!(picked, Some(&["python"][..]));
        let picked = pick_python(PYTHONS, |c| c[0] == "py");
        assert_eq!(picked, Some(&["py", "-3"][..]));
        assert_eq!(pick_python(PYTHONS, |_| false), None);
    }

    #[test]
    fn plan_runs_the_python_generator_then_every_rust_generator() {
        let root = Path::new("repo");
        let plan = gen_samples_plan(&["py", "-3"], root);
        assert_eq!(plan.len(), 1 + RUST_GENERATORS.len());
        assert_eq!(plan[0].program, "py");
        assert_eq!(plan[0].args[0], "-3");
        assert!(plan[0].args[1].ends_with("generate.py"));
        assert!(plan[0].env.is_empty());
        for step in &plan[1..] {
            assert_eq!(step.program, "cargo");
            assert_eq!(step.env, vec![("DEBRIEF_REGENERATE_SAMPLES".to_owned(), "1".to_owned())]);
            assert!(step.args.ends_with(&["--".into(), "--ignored".into(), "regenerate_".into()]));
        }
        assert!(plan[1]
            .display()
            .starts_with("DEBRIEF_REGENERATE_SAMPLES=1 cargo test -p debrief-plugin-profiling --no-default-features"));
        assert_eq!(plan.last().map(|s| s.args[2].as_str()), Some("debrief-plugin-windows"));
    }

    #[test]
    fn repo_root_holds_the_sample_generator() {
        assert!(repo_root().join("samples").join("generate.py").is_file());
    }
}
