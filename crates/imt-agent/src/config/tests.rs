use super::*;

const EXAMPLE: &str = include_str!("../../imt-agent.example.toml");

fn base() -> PathBuf {
    if cfg!(windows) {
        PathBuf::from("C:\\agent")
    } else {
        PathBuf::from("/agent")
    }
}

fn issues(text: &str) -> Vec<Issue> {
    let file = AgentConfigFile::parse(text).expect("parses");
    file.validate(&base()).expect_err("should be invalid")
}

fn has(issues: &[Issue], path: &str, needle: &str) -> bool {
    issues.iter().any(|i| i.path == path && i.message.contains(needle))
}

#[test]
fn the_documented_example_is_valid() {
    let file = AgentConfigFile::parse(EXAMPLE).unwrap();
    let cfg = file.validate(&base()).unwrap();
    assert_eq!(cfg.output_dir, base().join("runs"));
    assert_eq!(cfg.state_dir, base().join("runs").join(".imt-agent"));
    assert_eq!(cfg.log_dir, cfg.state_dir.join("logs"));
    assert_eq!(cfg.min_free_bytes, 1024 << 20);
    assert_eq!(cfg.tick, Duration::from_secs(15));
    assert_eq!(cfg.logman_timeout, Duration::from_secs(120), "the tools plugin's default");
    assert_eq!(cfg.retention.max_runs, Some(50));
    assert_eq!(cfg.retention.max_age, Some(Duration::from_secs(30 * 86_400)));
    assert_eq!(cfg.schedules.len(), 4);
    assert!(!cfg.schedule("hourly-sample").unwrap().enabled);
    let nightly = cfg.schedule("nightly").unwrap();
    assert!(matches!(nightly.when, When::Timer(Timer::Cron(_))));
    assert_eq!(
        nightly.actions.iter().map(Action::kind).collect::<Vec<_>>(),
        ["collect", "copyLogs"]
    );
    let Action::Collect(plan) = &nightly.actions[0] else { panic!() };
    assert_eq!(plan.name, "nightly");
    assert_eq!(plan.event_log_days, 1);
    let spike = cfg.schedule("memory-spike").unwrap();
    let When::Trigger(TriggerSpec::Process(p)) = &spike.when else {
        panic!()
    };
    assert_eq!(
        (p.metric, p.above, p.sustain),
        (Metric::PrivateBytes, 1500.0, Duration::from_secs(30))
    );
    assert_eq!(spike.actions[1].kind(), "perfmon");
    let crash = cfg.schedule("on-crash").unwrap();
    assert!(matches!(&crash.when, When::Trigger(TriggerSpec::Log(l)) if l.path == Path::new("/var/log/myapp/app.log") || cfg!(windows)));
    // Round trip through TOML and JSON (the UI's form) keeps everything.
    let again = AgentConfigFile::parse(&file.to_toml().unwrap()).unwrap();
    assert_eq!(again, file);
    let json = serde_json::to_string(&file).unwrap();
    assert!(json.contains("\"outputDir\":\"runs\""));
    assert_eq!(serde_json::from_str::<AgentConfigFile>(&json).unwrap(), file);
}

#[test]
fn starter_document_validates() {
    // An absolute path on this platform ("/tmp/runs" is relative to the current drive on Windows).
    let runs = if cfg!(windows) { "D:\\runs" } else { "/tmp/runs" };
    let file = AgentConfigFile::starter(runs);
    let cfg = file.validate(&base()).unwrap();
    assert_eq!(cfg.output_dir, PathBuf::from(runs));
    if cfg!(windows) {
        // A rooted path without a drive is on the drive of the config file's folder.
        let cfg = AgentConfigFile::starter("/tmp/runs").validate(&base()).unwrap();
        assert_eq!(cfg.output_dir, PathBuf::from("C:\\tmp\\runs"));
    }
    assert_eq!(cfg.schedules[0].name, "nightly");
}

#[test]
fn syntax_unknown_keys_and_versions_are_refused() {
    assert!(AgentConfigFile::parse("version = ").unwrap_err()[0]
        .message
        .contains("not a valid imt-agent.toml"));
    assert!(AgentConfigFile::parse("version = 1\nbogus = 2").unwrap_err()[0]
        .message
        .contains("unknown field"));
    let newer = AgentConfigFile::parse("version = 9").unwrap_err();
    assert_eq!(newer[0].path, "version");
    assert!(newer[0].message.contains("not supported"));
    assert!(AgentConfigFile::parse("version = 0").is_err());
    // validate() re-checks the version for documents built in code (the UI's JSON).
    let mut f = AgentConfigFile::starter("x");
    f.version = 2;
    assert!(f.validate(&base()).is_err());
}

#[test]
fn every_problem_is_reported_with_its_path() {
    let text = r#"
version = 1
[agent]
outputDir = ""
tickSecs = 0
logmanTimeoutSecs = 1
minFreeMb = 99999999999
[agent.log]
maxFileMb = 0
keepFiles = 0
maxAgeDays = 0
[retention]
maxRuns = 0
maxTotalMb = 0
maxAgeDays = 0

[[schedules]]
name = "../evil"
cron = "0 2 * * *"
every = "1h"

[[schedules]]
name = "a"
cron = "99 * * * *"
  [[schedules.actions]]
  kind = "monitor"
  targets = []
  duration = "2d"

[[schedules]]
name = "A"
every = "10s"
  [[schedules.actions]]
  kind = "collect"
  planFile = "plan.toml"
  plan = { name = "x" }
  [[schedules.actions]]
  kind = "collect"
  plan = { name = "" }
  [[schedules.actions]]
  kind = "copyLogs"
  from = " "
  include = ["a[", "*.log"]
  maxTotalMb = 0
  [[schedules.actions]]
  kind = "perfmon"
  collector = "-x"
  duration = "5m"
  [[schedules.actions]]
  kind = "perfmon"
  collector = "Set"
  duration = "soon"
  [[schedules.actions]]
  kind = "monitor"
  targets = ["svc"]
  duration = "0s"
"#;
    let found = issues(text);
    for (path, needle) in [
        ("agent.outputDir", "required"),
        ("agent.tickSecs", "between 1 and 3600"),
        ("agent.logmanTimeoutSecs", "between 5 and 600"),
        ("agent.minFreeMb", "between"),
        ("agent.log.maxFileMb", "between"),
        ("agent.log.keepFiles", "between"),
        ("agent.log.maxAgeDays", "between"),
        ("retention.maxRuns", "between"),
        ("retention.maxTotalMb", "between"),
        ("retention.maxAgeDays", "between"),
        ("schedules[0].name", "letters"),
        ("schedules[0]", "exactly one"),
        ("schedules[0].actions", "at least one"),
        ("schedules[1].cron", "invalid cron"),
        ("schedules[1].actions[0].duration", "between 1s and 24h"),
        ("schedules[1].actions[0].targets", "at least one"),
        ("schedules[2].name", "already called"),
        ("schedules[2].every", "between 1m and 31d"),
        ("schedules[2].actions[0]", "not both"),
        ("schedules[2].actions[1].plan", "name must not be empty"),
        ("schedules[2].actions[2].from", "required"),
        ("schedules[2].actions[2].include[0]", "invalid glob"),
        ("schedules[2].actions[2].maxTotalMb", "between"),
        ("schedules[2].actions[3].collector", "Data Collector Set"),
        ("schedules[2].actions[4].duration", "not a duration"),
        ("schedules[2].actions[5].duration", "between 1s and 24h"),
    ] {
        assert!(has(&found, path, needle), "missing {path} ~ {needle} in {found:#?}");
    }
    assert!(describe(&found).contains("agent.outputDir: a path is required"));
}

#[test]
fn trigger_problems() {
    let text = r#"
version = 1
[agent]
outputDir = "out"
[[schedules]]
name = "p"
  [schedules.trigger]
  kind = "process"
  process = "a["
  metric = "cpu"
  above = 10
  [[schedules.actions]]
  kind = "collect"
[[schedules]]
name = "q"
  [schedules.trigger]
  kind = "process"
  process = "svc"
  metric = "handles"
  above = -1
  [[schedules.actions]]
  kind = "collect"
[[schedules]]
name = "r"
  [schedules.trigger]
  kind = "log"
  path = "app.log"
  ql = ""
  [[schedules.actions]]
  kind = "collect"
[[schedules]]
name = "s"
  [schedules.trigger]
  kind = "log"
  path = "app.log"
  ql = "level >= "
  pollSecs = 0
  [[schedules.actions]]
  kind = "collect"
[[schedules]]
name = "t"
  [schedules.trigger]
  kind = "process"
  process = "svc"
  metric = "threads"
  above = 5
  sampleSecs = 0
  [[schedules.actions]]
  kind = "collect"
"#;
    let found = issues(text);
    assert!(has(&found, "schedules[0].trigger.process", "bad name pattern"), "{found:#?}");
    assert!(has(&found, "schedules[1].trigger.above", "greater than 0"));
    assert!(has(&found, "schedules[2].trigger.ql", "required"));
    assert!(found.iter().any(|i| i.path == "schedules[3].trigger.ql"));
    assert!(has(&found, "schedules[4].trigger.sampleSecs", "between"));
    assert!(has(&found, "schedules[3].trigger.pollSecs", "between"));
}

#[test]
fn plan_files_and_paths() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("plan.toml"), "name = \"from-file\"\nmsinfo = true\n").unwrap();
    std::fs::write(dir.path().join("bad.toml"), "nope = 1\n").unwrap();
    std::fs::write(dir.path().join("huge.toml"), vec![b'#'; (MAX_CONFIG_BYTES + 1) as usize]).unwrap();
    let text = r#"
version = 1
[agent]
outputDir = "out"
stateDir = "st"
logDir = "lg"
[[schedules]]
name = "a"
every = "1h"
  [[schedules.actions]]
  kind = "collect"
  planFile = "plan.toml"
  [[schedules.actions]]
  kind = "monitor"
  targets = ["1234", "/opt/app/bin/*", "svc*"]
  duration = "1m"
"#;
    let cfg = AgentConfigFile::parse(text).unwrap().validate(dir.path()).unwrap();
    assert_eq!(cfg.state_dir, dir.path().join("st"));
    assert_eq!(cfg.log_dir, dir.path().join("lg"));
    let Action::Collect(plan) = &cfg.schedules[0].actions[0] else {
        panic!()
    };
    assert_eq!(plan.name, "from-file");
    let Action::Monitor { settings, duration } = &cfg.schedules[0].actions[1] else {
        panic!()
    };
    assert_eq!(*duration, Duration::from_secs(60));
    assert_eq!(settings["targets"][0]["pid"], 1234);
    assert_eq!(settings["targets"][1]["path"], "/opt/app/bin/*");
    assert_eq!(settings["targets"][2]["name"], "svc*");
    // The .NET runtime counters are opt-in and reach the monitor's settings.
    assert_eq!(settings["dotnet"], false);
    let with_dotnet = text.replace(
        "duration = \"1m\"",
        "duration = \"1m\"
  dotnet = true",
    );
    let cfg = AgentConfigFile::parse(&with_dotnet).unwrap().validate(dir.path()).unwrap();
    let Action::Monitor { settings, .. } = &cfg.schedules[0].actions[1] else {
        panic!()
    };
    assert_eq!(settings["dotnet"], true);

    for (file, needle) in [
        ("bad.toml", "unknown field"),
        ("huge.toml", "larger than 1 MB"),
        ("missing.toml", "cannot read"),
    ] {
        let t = text.replace("plan.toml", file);
        let found = AgentConfigFile::parse(&t).unwrap().validate(dir.path()).unwrap_err();
        assert!(has(&found, "schedules[0].actions[0].planFile", needle), "{file}: {found:#?}");
    }
    let nul = AgentConfigFile::parse(&text.replace("\"st\"", "\"a\\u0000b\"")).unwrap();
    assert!(has(&nul.validate(dir.path()).unwrap_err(), "agent.stateDir", "NUL"));
}

#[test]
fn load_from_disk() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("imt-agent.toml");
    std::fs::write(&path, EXAMPLE).unwrap();
    let cfg = AgentConfig::load(&path).unwrap();
    assert_eq!(cfg.output_dir, std::fs::canonicalize(dir.path()).unwrap().join("runs"));
    assert!(AgentConfig::load(&dir.path().join("none.toml")).unwrap_err()[0]
        .message
        .contains("cannot read"));
    std::fs::write(&path, vec![b'#'; (MAX_CONFIG_BYTES + 1) as usize]).unwrap();
    assert!(AgentConfigFile::load(&path).unwrap_err()[0].message.contains("larger than"));
}

#[test]
fn limits_on_counts() {
    let mut f = AgentConfigFile::starter("/out");
    let one = f.schedules[0].clone();
    f.schedules = (0..MAX_SCHEDULES + 1)
        .map(|i| ScheduleFile {
            name: format!("s{i}"),
            ..one.clone()
        })
        .collect();
    f.schedules[0].actions = vec![one.actions[0].clone(); MAX_ACTIONS + 1];
    let found = f.validate(&base()).unwrap_err();
    assert!(has(&found, "schedules", "at most 64"));
    assert!(has(&found, "schedules[0].actions", "at most 16"));
}

#[test]
fn small_helpers() {
    assert!(valid_name("nightly run-1.a_b"));
    assert!(!valid_name(".hidden"));
    assert!(!valid_name(""));
    assert!(!valid_name(&"x".repeat(65)));
    assert_eq!(Issue::new("", "m").to_string(), "m");
    assert_eq!(Metric::PrivateBytes.unit(), "MB");
    assert_eq!(Metric::Cpu.unit(), "%");
    assert_eq!(Metric::Handles.unit(), "");
    assert_eq!(
        Action::Perfmon {
            collector: "x".into(),
            duration: Duration::from_secs(1)
        }
        .kind(),
        "perfmon"
    );
}

#[test]
fn saving_over_the_documented_example_keeps_its_comments() {
    let mut file = AgentConfigFile::parse(EXAMPLE).unwrap();
    // Unchanged: byte for byte the same file.
    assert_eq!(file.to_toml_over(EXAMPLE).unwrap(), EXAMPLE);
    file.agent.tick_secs = 30;
    file.agent.logman_timeout_secs = 300;
    file.schedules.remove(1);
    file.schedules[0].name = "nightly-2".into();
    let saved = file.to_toml_over(EXAMPLE).unwrap();
    assert_eq!(AgentConfigFile::parse(&saved).unwrap(), file, "{saved}");
    for comment in EXAMPLE.lines().filter(|l| l.trim_start().starts_with('#')).take(12) {
        assert!(saved.contains(comment), "lost {comment:?} in\n{saved}");
    }
    assert!(
        saved.contains("tickSecs = 30") && saved.contains("logmanTimeoutSecs = 300"),
        "{saved}"
    );
    // A file that is TOML but not a valid config is updated with every value.
    let odd = "# keep\nversion = 1\nunknownKey = 3\n";
    let saved = file.to_toml_over(odd).unwrap();
    assert!(saved.starts_with("# keep\n"), "{saved}");
    assert_eq!(AgentConfigFile::parse(&saved).unwrap(), file);
    // Not TOML any more: an error, the caller decides.
    assert!(file.to_toml_over("version = ").is_err());
}
