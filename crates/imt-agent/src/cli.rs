//! The `imt-agent` command line.

use crate::agent::{Agent, Deps};
use crate::config::{describe, AgentConfig};
use crate::job::Outcome;
use crate::log::AgentLog;
use crate::service::{self, absolute_config};
use std::io::Write;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

pub const USAGE: &str = "usage:
  imt-agent run [--config imt-agent.toml]                    run schedules until stopped (Ctrl+C)
  imt-agent once [--config imt-agent.toml] [--schedule NAME] run every enabled schedule (or NAME) now
  imt-agent validate [--config imt-agent.toml]               check a config file
  imt-agent print-example                                    print a documented example config
  imt-agent service install [--config FILE] [--name NAME]    Windows: install the service (as administrator)
  imt-agent service uninstall [--name NAME]                  Windows: remove the service (as administrator)
  imt-agent service print-unit [--config FILE] [--user USER] Linux: print a systemd unit
  imt-agent service print-plist [--config FILE]              macOS: print a launchd plist
  imt-agent --version";

pub const EXAMPLE: &str = include_str!("../imt-agent.example.toml");

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Run {
        config: PathBuf,
    },
    Once {
        config: PathBuf,
        schedule: Option<String>,
    },
    Validate {
        config: PathBuf,
    },
    PrintExample,
    Install {
        config: PathBuf,
        name: String,
    },
    Uninstall {
        name: String,
    },
    PrintUnit {
        config: PathBuf,
        user: Option<String>,
    },
    PrintPlist {
        config: PathBuf,
    },
    /// Started by the Windows SCM.
    ServiceRun {
        config: PathBuf,
    },
    Help,
    Version,
}

/// Parses the arguments (without the program name).
pub fn parse(args: &[String]) -> Result<Command, String> {
    let mut it = args.iter().map(String::as_str);
    let first = it.next().unwrap_or("help");
    let first = if first == "service" {
        match it.next() {
            Some(sub @ ("install" | "uninstall" | "print-unit" | "print-plist")) => sub,
            Some(other) => return Err(format!("unknown service command \"{other}\"\n{USAGE}")),
            None => return Err(format!("service needs a command\n{USAGE}")),
        }
    } else {
        first
    };
    let (mut config, mut schedule, mut name, mut user) = (PathBuf::from("imt-agent.toml"), None, service::SERVICE_NAME.to_string(), None);
    let allowed: &[&str] = match first {
        "run" | "validate" | "service-run" | "print-unit" | "print-plist" | "install" => &["--config", "--user", "--name"],
        "once" => &["--config", "--schedule"],
        "uninstall" => &["--name"],
        _ => &[],
    };
    while let Some(flag) = it.next() {
        if !allowed.contains(&flag) {
            return Err(format!("unknown argument \"{flag}\" for {first}\n{USAGE}"));
        }
        let value = it.next().ok_or_else(|| format!("{flag} needs a value"))?.to_string();
        match flag {
            "--config" => config = PathBuf::from(value),
            "--schedule" => schedule = Some(value),
            "--name" => name = value,
            _ => user = Some(value),
        }
    }
    Ok(match first {
        "run" => Command::Run { config },
        "once" => Command::Once { config, schedule },
        "validate" => Command::Validate { config },
        "print-example" => Command::PrintExample,
        "install" => Command::Install { config, name },
        "uninstall" => Command::Uninstall { name },
        "print-unit" => Command::PrintUnit { config, user },
        "print-plist" => Command::PrintPlist { config },
        "service-run" => Command::ServiceRun { config },
        "help" | "-h" | "--help" => Command::Help,
        "-V" | "--version" => Command::Version,
        other => return Err(format!("unknown command \"{other}\"\n{USAGE}")),
    })
}

fn load(config: &std::path::Path) -> Result<AgentConfig, (i32, String)> {
    AgentConfig::load(config).map_err(|i| (1, format!("{}:\n{}", config.display(), describe(&i))))
}

fn exe() -> Result<PathBuf, (i32, String)> {
    std::env::current_exe().map_err(|e| (1, format!("cannot find imt-agent itself: {e}")))
}

/// Runs a command, writing results to `out`. Errors carry the exit code.
pub fn run(args: &[String], out: &mut dyn Write, stop: Arc<AtomicBool>) -> Result<(), (i32, String)> {
    let cmd = parse(args).map_err(|e| (2, e))?;
    let w = |out: &mut dyn Write, text: &str| {
        let _ = writeln!(out, "{text}");
    };
    match cmd {
        Command::Help => w(out, USAGE),
        Command::Version => w(out, concat!("imt-agent ", env!("CARGO_PKG_VERSION"))),
        Command::PrintExample => w(out, EXAMPLE.trim_end()),
        Command::Validate { config } => {
            let cfg = load(&config)?;
            let enabled = cfg.schedules.iter().filter(|s| s.enabled).count();
            w(
                out,
                &format!(
                    "{} is valid: {} schedule(s), {enabled} enabled; runs go to {}",
                    config.display(),
                    cfg.schedules.len(),
                    cfg.output_dir.display()
                ),
            );
        }
        Command::Run { config } => service::run_service(&config, stop, true).map_err(|e| (1, e))?,
        Command::Once { config, schedule } => {
            let cfg = load(&config)?;
            let log = AgentLog::open(&cfg.log_dir, &cfg.log, true).map_err(|e| (1, format!("cannot open the log: {e:#}")))?;
            let deps = Deps::system(Arc::new(log), &cfg);
            let mut agent = Agent::new(cfg, deps, stop);
            let results = agent.run_once(schedule.as_deref()).map_err(|e| (2, e))?;
            for (name, outcome) in &results {
                match outcome {
                    Outcome::Ran { id, status } => w(out, &format!("{name}: {status:?} ({id})")),
                    Outcome::Skipped(why) => w(out, &format!("{name}: skipped — {why}")),
                }
            }
            if Agent::exit_code(&results) != 0 {
                return Err((1, "some runs did not succeed; see the run manifests and the agent log".into()));
            }
        }
        Command::PrintUnit { config, user } => {
            let unit = service::systemd::unit_file(&service::systemd::UnitOptions {
                exe: exe()?,
                config: absolute_config(&config).map_err(|e| (1, e))?,
                user,
            })
            .map_err(|e| (1, e))?;
            let _ = write!(out, "{unit}");
        }
        Command::PrintPlist { config } => {
            let cfg = load(&config)?;
            let text = service::launchd::plist(&service::launchd::PlistOptions {
                exe: exe()?,
                config: absolute_config(&config).map_err(|e| (1, e))?,
                log_dir: cfg.log_dir,
            })
            .map_err(|e| (1, e))?;
            let _ = write!(out, "{text}");
        }
        Command::Install { config, name } => {
            let config = absolute_config(&config).map_err(|e| (1, e))?;
            load(&config)?;
            install(&name, &config).map_err(|e| (1, e))?;
            w(out, &format!("Installed the {name} service; start it with: sc start {name}"));
        }
        Command::Uninstall { name } => {
            uninstall(&name).map_err(|e| (1, e))?;
            w(out, &format!("Removed the {name} service"));
        }
        Command::ServiceRun { config } => service_run(config).map_err(|e| (1, e))?,
    }
    Ok(())
}

#[cfg(not(windows))]
const NOT_WINDOWS: &str = "Windows services can only be installed on Windows. On Linux use `imt-agent service print-unit` \
(then systemctl enable --now imt-agent); on macOS use `imt-agent service print-plist` (then launchctl bootstrap system <plist>)";

#[cfg(windows)]
fn install(name: &str, config: &std::path::Path) -> Result<(), String> {
    service::windows::install(name, config)
}
#[cfg(not(windows))]
fn install(_name: &str, _config: &std::path::Path) -> Result<(), String> {
    Err(NOT_WINDOWS.into())
}
#[cfg(windows)]
fn uninstall(name: &str) -> Result<(), String> {
    service::windows::uninstall(name)
}
#[cfg(not(windows))]
fn uninstall(_name: &str) -> Result<(), String> {
    Err(NOT_WINDOWS.into())
}
#[cfg(windows)]
fn service_run(config: PathBuf) -> Result<(), String> {
    service::windows::dispatch(config)
}
#[cfg(not(windows))]
fn service_run(_config: PathBuf) -> Result<(), String> {
    Err("service-run is used by the Windows Service Control Manager; use `imt-agent run` here".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::AgentConfigFile;

    fn v(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| s.to_string()).collect()
    }

    fn exec(args: &[&str]) -> (Result<(), (i32, String)>, String) {
        let mut out = Vec::new();
        let r = run(&v(args), &mut out, Arc::new(AtomicBool::new(true)));
        (r, String::from_utf8(out).unwrap())
    }

    #[test]
    fn parses_commands() {
        assert_eq!(parse(&v(&[])).unwrap(), Command::Help);
        assert_eq!(
            parse(&v(&["run", "--config", "a.toml"])).unwrap(),
            Command::Run { config: "a.toml".into() }
        );
        assert_eq!(
            parse(&v(&["once", "--schedule", "nightly"])).unwrap(),
            Command::Once {
                config: "imt-agent.toml".into(),
                schedule: Some("nightly".into())
            }
        );
        assert_eq!(
            parse(&v(&["service", "install", "--config", "c", "--name", "n"])).unwrap(),
            Command::Install {
                config: "c".into(),
                name: "n".into()
            }
        );
        assert_eq!(
            parse(&v(&["service", "uninstall"])).unwrap(),
            Command::Uninstall { name: "imt-agent".into() }
        );
        assert_eq!(
            parse(&v(&["service", "print-unit", "--user", "imt"])).unwrap(),
            Command::PrintUnit {
                config: "imt-agent.toml".into(),
                user: Some("imt".into())
            }
        );
        assert_eq!(
            parse(&v(&["service", "print-plist"])).unwrap(),
            Command::PrintPlist {
                config: "imt-agent.toml".into()
            }
        );
        assert_eq!(
            parse(&v(&["service-run", "--config", "c"])).unwrap(),
            Command::ServiceRun { config: "c".into() }
        );
        assert_eq!(parse(&v(&["--version"])).unwrap(), Command::Version);
        assert_eq!(parse(&v(&["print-example"])).unwrap(), Command::PrintExample);
        for bad in [
            v(&["bogus"]),
            v(&["service"]),
            v(&["service", "start"]),
            v(&["run", "--schedule", "x"]),
            v(&["once", "--config"]),
            v(&["service", "uninstall", "--config", "x"]),
        ] {
            assert!(parse(&bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn help_version_example() {
        assert!(exec(&["--help"]).1.contains("imt-agent run"));
        assert!(exec(&["-V"]).1.starts_with("imt-agent "));
        let (r, text) = exec(&["print-example"]);
        r.unwrap();
        assert!(AgentConfigFile::parse(&text).is_ok());
        assert_eq!(exec(&["nope"]).0.unwrap_err().0, 2);
    }

    #[test]
    fn validate_once_and_run() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("imt-agent.toml");
        let c = cfg.to_str().unwrap();
        let (r, _) = exec(&["validate", "--config", c]);
        assert!(r.unwrap_err().1.contains("cannot read"));
        std::fs::write(&cfg, "version = 1\n[agent]\noutputDir = \"runs\"\nminFreeMb = 0\n[[schedules]]\nname = \"copy\"\nevery = \"1h\"\n  [[schedules.actions]]\n  kind = \"copyLogs\"\n  from = \"src\"\n").unwrap();
        let (r, text) = exec(&["validate", "--config", c]);
        r.unwrap();
        assert!(text.contains("is valid: 1 schedule(s), 1 enabled"), "{text}");
        // `once` with a stop already requested runs nothing and succeeds.
        let (r, text) = exec(&["once", "--config", c]);
        r.unwrap();
        assert!(text.is_empty());
        let (r, _) = exec(&["once", "--config", c, "--schedule", "nope"]);
        assert_eq!(r.unwrap_err().0, 2);
        // A real run: the source folder is missing, so the run fails (exit 1) but is recorded.
        let mut out = Vec::new();
        let r = run(&v(&["once", "--config", c]), &mut out, Arc::new(AtomicBool::new(false)));
        let text = String::from_utf8(out).unwrap();
        assert!(text.starts_with("copy: Failed ("), "{text}");
        assert_eq!(r.unwrap_err().0, 1);
        assert_eq!(crate::run::list_runs(&dir.path().join("runs")).len(), 1);
        // `run` with stop set returns at once.
        exec(&["run", "--config", c]).0.unwrap();
        // The log folder cannot be opened for `once`.
        std::fs::write(dir.path().join("file"), "x").unwrap();
        std::fs::write(&cfg, "version = 1\n[agent]\noutputDir = \"runs\"\nlogDir = \"file/x\"\n").unwrap();
        assert!(exec(&["once", "--config", c]).0.unwrap_err().1.contains("cannot open the log"));
    }

    #[test]
    fn service_files_and_install_off_windows() {
        let dir = tempfile::tempdir().unwrap();
        let cfg = dir.path().join("imt-agent.toml");
        std::fs::write(&cfg, "version = 1\n[agent]\noutputDir = \"runs\"\n").unwrap();
        let c = cfg.to_str().unwrap();
        let (r, unit) = exec(&["service", "print-unit", "--config", c, "--user", "imt"]);
        r.unwrap();
        assert!(unit.contains("run --config") && unit.contains("User=imt"));
        assert!(exec(&["service", "print-unit", "--config", c, "--user", "a b"]).0.is_err());
        assert!(exec(&["service", "print-unit", "--config", "missing.toml"]).0.is_err());
        let (r, plist) = exec(&["service", "print-plist", "--config", c]);
        r.unwrap();
        assert!(plist.contains("net.decryptology.imt-agent"));
        assert!(exec(&["service", "print-plist", "--config", "missing.toml"]).0.is_err());
        if !cfg!(windows) {
            assert!(exec(&["service", "install", "--config", c])
                .0
                .unwrap_err()
                .1
                .contains("only be installed on Windows"));
            assert!(exec(&["service", "install", "--config", "missing.toml"]).0.is_err());
            assert!(exec(&["service", "uninstall"]).0.unwrap_err().1.contains("print-unit"));
            assert!(exec(&["service-run", "--config", c]).0.unwrap_err().1.contains("imt-agent run"));
        }
    }
}
