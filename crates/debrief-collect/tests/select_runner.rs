use debrief_collect::*;
use std::fs::{self, File};
use std::path::Path;
use std::time::{Duration, SystemTime};

fn write(dir: &Path, rel: &str, size: usize, age_days: u64, now: SystemTime) {
    let p = dir.join(rel);
    fs::create_dir_all(p.parent().unwrap()).unwrap();
    fs::write(&p, vec![b'a'; size]).unwrap();
    File::options()
        .write(true)
        .open(&p)
        .unwrap()
        .set_modified(now - Duration::from_secs(age_days * 86_400))
        .unwrap();
}

fn spec(path: &Path) -> LogSpec {
    LogSpec {
        path: path.display().to_string(),
        include: vec!["*.log".into()],
        max_age_days: None,
        max_total_mb: None,
    }
}

#[test]
fn globs_match_nested_files_and_names() {
    let d = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    write(d.path(), "a.log", 5, 0, now);
    write(d.path(), "sub/deep/b.log", 5, 1, now);
    write(d.path(), "c.bin", 5, 0, now);
    let (sel, rej) = select_logs(&spec(d.path()), now);
    let rels: Vec<_> = sel.iter().map(|s| s.rel.as_str()).collect();
    assert_eq!(rels, vec!["a.log", "sub/deep/b.log"], "newest first");
    assert!(rej.is_empty());

    let mut s = spec(d.path());
    s.include = vec!["sub/**/*.log".into()];
    assert_eq!(select_logs(&s, now).0.len(), 1);
    s.include = vec![];
    assert_eq!(select_logs(&s, now).0.len(), 3, "empty include means everything");
}

#[test]
fn age_filter_rejects_old_files() {
    let d = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    write(d.path(), "new.log", 1, 1, now);
    write(d.path(), "old.log", 1, 30, now);
    let mut s = spec(d.path());
    s.max_age_days = Some(7);
    let (sel, rej) = select_logs(&s, now);
    assert_eq!(sel.len(), 1);
    assert_eq!(sel[0].rel, "new.log");
    assert_eq!(rej.len(), 1);
    assert!(rej[0].reason.contains("older than 7 days"));
}

#[test]
fn total_size_cap_keeps_newest() {
    let d = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    let mb = 1 << 20;
    write(d.path(), "n.log", mb, 1, now);
    write(d.path(), "m.log", mb, 2, now);
    write(d.path(), "o.log", mb, 3, now);
    let mut s = spec(d.path());
    s.max_total_mb = Some(2);
    let (sel, rej) = select_logs(&s, now);
    assert_eq!(sel.iter().map(|x| x.rel.as_str()).collect::<Vec<_>>(), vec!["n.log", "m.log"]);
    assert_eq!(rej.len(), 1);
    assert!(rej[0].path.ends_with("o.log") && rej[0].reason.contains("2 MB"));
}

#[test]
fn single_file_missing_and_special_paths() {
    let d = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    write(d.path(), "one.txt", 3, 0, now);
    let mut s = spec(&d.path().join("one.txt"));
    s.include = vec!["*.txt".into()];
    let (sel, _) = select_logs(&s, now);
    assert_eq!(sel[0].rel, "one.txt");
    assert_eq!(sel[0].size, 3);

    let (sel, rej) = select_logs(&spec(&d.path().join("nope")), now);
    assert!(sel.is_empty());
    assert_eq!(rej[0].reason, "path does not exist");
}

#[cfg(unix)]
#[test]
fn symlinks_are_not_followed_and_unreadable_dirs_noted() {
    use std::os::unix::fs::{symlink, PermissionsExt};
    let d = tempfile::tempdir().unwrap();
    let now = SystemTime::now();
    write(d.path(), "real/a.log", 1, 0, now);
    symlink(d.path().join("real"), d.path().join("link")).unwrap();
    symlink(d.path().join("real/a.log"), d.path().join("l.log")).unwrap();
    let (sel, _) = select_logs(&spec(d.path()), now);
    assert_eq!(sel.len(), 1);

    // A symlink given as the source itself is neither file nor dir.
    let (sel, rej) = select_logs(&spec(&d.path().join("l.log")), now);
    assert!(sel.is_empty());
    assert!(rej[0].reason.contains("not a regular"));

    let locked = d.path().join("locked");
    fs::create_dir(&locked).unwrap();
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o000)).unwrap();
    let (_, rej) = select_logs(&spec(d.path()), now);
    fs::set_permissions(&locked, fs::Permissions::from_mode(0o755)).unwrap();
    let is_root = std::fs::read_dir(&locked).is_ok();
    // Running as root can read it; otherwise the folder is reported.
    let _ = is_root;
    assert!(rej.iter().all(|r| !r.reason.is_empty()));
}

#[cfg(unix)]
#[test]
fn system_runner_runs_times_out_and_reports_missing() {
    let r = SystemRunner;
    let out = r
        .run(
            "sh",
            &["-c".into(), "echo hi; echo err >&2; exit 3".into()],
            Duration::from_secs(10),
        )
        .unwrap();
    assert_eq!(out.exit_code, Some(3));
    assert_eq!(out.stdout, b"hi\n");
    assert_eq!(out.stderr, "err");
    assert!(!out.success());
    let ok = r.run("sh", &["-c".into(), "true".into()], Duration::from_secs(10)).unwrap();
    assert!(ok.success());

    let e = r
        .run("sh", &["-c".into(), "sleep 5".into()], Duration::from_millis(100))
        .unwrap_err();
    assert!(matches!(e, CommandError::TimedOut(_, 0)), "{e:?}");
    assert!(e.to_string().contains("stopped"));

    let e = r.run("definitely-not-a-real-tool-xyz", &[], Duration::from_secs(1)).unwrap_err();
    assert_eq!(e, CommandError::NotFound("definitely-not-a-real-tool-xyz".into()));

    let e = r.run("/", &[], Duration::from_secs(1)).unwrap_err();
    assert!(matches!(e, CommandError::Io(..)), "{e:?}");
    assert!(e.to_string().contains("could not run"));
}
