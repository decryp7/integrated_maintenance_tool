//! `debrief-app --export-diagnostics` end to end: the real binary, Debrief's folders under `DEBRIEF_DATA_DIR` (debug
//! builds), no window. Checks the exit codes, the printed path and the zip.

use std::path::{Path, PathBuf};

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("debrief-export-cli-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn run(root: &Path, args: &[&str]) -> std::process::Output {
    debrief_core::process::quiet_command(env!("CARGO_BIN_EXE_debrief-app"))
        .args(args)
        .env("DEBRIEF_DATA_DIR", root)
        .current_dir(root)
        .output()
        .unwrap()
}

#[test]
fn the_cli_exports_a_report_without_a_window() {
    if !cfg!(debug_assertions) {
        // Release builds ignore DEBRIEF_DATA_DIR by design.
        return;
    }
    let root = temp("e2e");
    let logs = root.join("logs");
    std::fs::create_dir_all(logs.join("children")).unwrap();
    std::fs::create_dir_all(root.join("config")).unwrap();
    std::fs::write(
        logs.join("debrief.2026-10-14.log"),
        "{\"@t\":\"2026-10-14T13:00:05Z\",\"SourceContext\":\"Debrief.Activity\",\"Operation\":\"file.open\",\"Trigger\":\"menu\",\"Window\":\"main\"}\n",
    )
    .unwrap();
    std::fs::write(
        logs.join("children").join("ai-worker.log"),
        "{\"@t\":\"2026-10-14T13:01:00.000Z\",\"@l\":\"Information\",\"@m\":\"Started ai-worker (pid 7, run 1)\",\"Event\":\"start\",\"ChildKind\":\"ai-worker\",\"Pid\":7}\n\
         {\"@t\":\"2026-10-14T13:02:00.000Z\",\"@l\":\"Fatal\",\"@m\":\"Crashed: ai-worker (pid 7) ended with exit code 3\",\"Event\":\"crash\",\"ChildKind\":\"ai-worker\",\"Pid\":7,\"Exit\":\"exit code 3\"}\n",
    )
    .unwrap();
    // A corrupt settings file does not stop the export.
    std::fs::write(root.join("config").join("settings.json"), "{ corrupt").unwrap();

    let out = run(&root, &["--export-diagnostics", "report.zip"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert_eq!(out.status.code(), Some(0), "stdout: {stdout}\nstderr: {stderr}");
    let printed = PathBuf::from(stdout.trim());
    assert_eq!(printed, root.canonicalize().unwrap().join("report.zip"));
    assert!(stderr.contains("settings.json is not valid JSON"), "{stderr}");

    let mut zip = zip::ZipArchive::new(std::fs::File::open(&printed).unwrap()).unwrap();
    let names: Vec<String> = zip.file_names().filter_map(Result::ok).map(|n| n.into_owned()).collect();
    for want in [
        "manifest.json",
        "processes.json",
        "system-info.json",
        "steps-to-reproduce.txt",
        "logs/children/ai-worker.log",
    ] {
        assert!(names.iter().any(|n| n == want), "{want} missing from {names:?}");
    }
    let mut text = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("processes.json").unwrap(), &mut text).unwrap();
    assert!(text.contains("\"crashed\"") && text.contains("exit code 3"), "{text}");
    let mut steps = String::new();
    std::io::Read::read_to_string(&mut zip.by_name("steps-to-reproduce.txt").unwrap(), &mut steps).unwrap();
    assert!(steps.starts_with("1. Opened files"), "{steps}");

    // No overwrite without --force; bad destinations fail with exit code 1.
    let again = run(&root, &["--export-diagnostics", "report.zip"]);
    assert_eq!(again.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&again.stderr).contains("--force"));
    assert_eq!(
        run(&root, &["--export-diagnostics", "report.zip", "--force"]).status.code(),
        Some(0)
    );
    assert_eq!(run(&root, &["--export-diagnostics", "nope/report.zip"]).status.code(), Some(1));
    assert_eq!(run(&root, &["--export-diagnostics", "report.txt"]).status.code(), Some(1));
    let _ = std::fs::remove_dir_all(root);
}
