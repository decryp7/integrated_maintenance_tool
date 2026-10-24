//! The runner and the passes against the fake ffmpeg/ffprobe (`src/bin/fake_ffmpeg.rs`): the happy path, the
//! self-test, and every failure the runner must contain (hang, memory hog, bad exit, bad output, no frames, Cancel).

use debrief_video::analyze::Pass;
use debrief_video::error::VideoError;
use debrief_video::run::{self, Caps, Run, Tools};
use debrief_video::runtime::self_test;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

const FAKE: &str = env!("CARGO_BIN_EXE_debrief-video-fake-ffmpeg");

/// The tests copy executables and start processes; one at a time keeps a fork from inheriting another test's
/// half-written copy (a flaky "exec failed").
fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

/// A runtime folder holding the fake as `ffmpeg` and `ffprobe`.
fn runtime(dir: &Path, mode: &str) -> Tools {
    let rt = dir.join("runtime");
    std::fs::create_dir_all(&rt).unwrap();
    let tools = Tools::in_dir(&rt);
    // A new file each time: rewriting a running-signed executable in place makes macOS kill it (SIGKILL).
    for exe in [&tools.ffmpeg, &tools.ffprobe] {
        let _ = std::fs::remove_file(exe);
        std::fs::copy(FAKE, exe).unwrap();
    }
    let _ = std::fs::remove_file(rt.join("fake-mode"));
    if !mode.is_empty() {
        std::fs::write(rt.join("fake-mode"), mode).unwrap();
    }
    tools
}

fn video(dir: &Path) -> PathBuf {
    let p = dir.join("recording.mp4");
    let mut text = String::from("FAKEVIDEO duration_ms=60000 created=2026-10-14T10:40:00Z\n");
    for (ms, score) in [(0, 0.0), (4000, 0.4), (9000, 0.2), (30_000, 0.7), (59_000, 0.1)] {
        text.push_str(&format!("{ms} {score}\n"));
    }
    std::fs::write(&p, text).unwrap();
    p
}

fn work(dir: &Path) -> PathBuf {
    let w = dir.join("work");
    std::fs::create_dir_all(&w).unwrap();
    w
}

#[test]
fn the_fake_passes_the_self_test() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let tools = runtime(d.path(), "");
    let cancel = AtomicBool::new(false);
    let r = self_test(&tools, &d.path().join("scratch"), &cancel).unwrap();
    assert_eq!(r.version.as_deref(), Some("n9.0.2-fake"));
    assert!(!r.gpl);
    assert!(!d.path().join("scratch").exists(), "the scratch folder is removed");
}

#[test]
fn a_video_is_probed_analysed_and_extracted() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let tools = runtime(d.path(), "");
    let v = video(d.path());
    let w = work(d.path());
    let cancel = AtomicBool::new(false);
    let pass = Pass {
        tools: &tools,
        input: &v,
        work: &w,
        memory_bytes: 512 << 20,
        cancel: &cancel,
    };
    let info = pass.probe(false).unwrap();
    assert_eq!((info.duration_ms, info.width, info.height), (60_000, 1920, 1080));
    assert!(info.creation_ms.is_some());
    let mut seen = Vec::new();
    let c = pass.candidates(&info, &mut |p| seen.push(p)).unwrap();
    assert_eq!(c.iter().map(|c| c.ms).collect::<Vec<_>>(), [0, 4000, 9000, 30_000, 59_000]);
    let got = pass.extract(&info, &c[1..4], &mut |_| {}).unwrap();
    assert_eq!(got.iter().map(|e| e.video_ms).collect::<Vec<_>>(), [4000, 9000, 30_000]);
    assert!(got.iter().all(|e| e.path.is_file()));
    let one = pass.extract_at(&info, 10_000).unwrap();
    assert_eq!(one.video_ms, 30_000, "the first frame at or after the time");
    assert!(matches!(pass.extract_at(&info, 70_000), Err(VideoError::Invalid(_))));
}

#[test]
fn a_hanging_process_is_stopped_at_its_timeout() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let tools = runtime(d.path(), "hang");
    let v = video(d.path());
    let cancel = AtomicBool::new(false);
    let started = Instant::now();
    let e = run::run(
        &tools,
        Run {
            exe: &tools.ffmpeg,
            args: vec!["-i".into(), v.to_string_lossy().into_owned()],
            work: &work(d.path()),
            input: Some(&v),
            caps: Caps {
                memory_bytes: 512 << 20,
                timeout: Duration::from_millis(600),
            },
            cancel: &cancel,
            label: "ffmpeg",
            on_tick: None,
        },
    )
    .unwrap_err();
    assert!(matches!(e, VideoError::Timeout(_)), "{e}");
    assert!(started.elapsed() < Duration::from_secs(20));
}

#[test]
fn cancel_stops_a_running_process() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let tools = runtime(d.path(), "hang");
    let v = video(d.path());
    let cancel = AtomicBool::new(false);
    let mut ticks = 0;
    let mut tick = || {
        ticks += 1;
        if ticks == 3 {
            cancel.store(true, Ordering::SeqCst);
        }
    };
    let e = run::run(
        &tools,
        Run {
            exe: &tools.ffmpeg,
            args: vec!["-i".into(), v.to_string_lossy().into_owned()],
            work: &work(d.path()),
            input: Some(&v),
            caps: Caps {
                memory_bytes: 512 << 20,
                timeout: Duration::from_secs(60),
            },
            cancel: &cancel,
            label: "ffmpeg",
            on_tick: Some(&mut tick),
        },
    )
    .unwrap_err();
    assert_eq!(e, VideoError::Cancelled);
}

#[test]
fn a_process_over_its_memory_cap_is_stopped() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let tools = runtime(d.path(), "memory");
    let v = video(d.path());
    let cancel = AtomicBool::new(false);
    let e = run::run(
        &tools,
        Run {
            exe: &tools.ffmpeg,
            args: vec!["-i".into(), v.to_string_lossy().into_owned()],
            work: &work(d.path()),
            input: Some(&v),
            caps: Caps {
                memory_bytes: 256 << 20,
                timeout: Duration::from_secs(30),
            },
            cancel: &cancel,
            label: "ffmpeg",
            on_tick: None,
        },
    )
    .unwrap_err();
    assert!(matches!(e, VideoError::OverMemory(_)), "{e}");
}

#[test]
fn a_failing_process_reports_its_last_error_lines() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let tools = runtime(d.path(), "badexit");
    let v = video(d.path());
    let w = work(d.path());
    let cancel = AtomicBool::new(false);
    let pass = Pass {
        tools: &tools,
        input: &v,
        work: &w,
        memory_bytes: 512 << 20,
        cancel: &cancel,
    };
    let e = pass.probe(false).unwrap_err();
    assert!(matches!(e, VideoError::Tool(_)), "{e}");
    // The self-test names the step that failed.
    let e = self_test(&tools, &d.path().join("scratch"), &cancel).unwrap_err();
    assert!(matches!(e, VideoError::SelfTest { .. }), "{e}");
}

#[test]
fn unusable_output_is_a_clear_error_not_a_panic() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let v = video(d.path());
    let w = work(d.path());
    let cancel = AtomicBool::new(false);
    let tools = runtime(d.path(), "badprobe");
    let pass = Pass {
        tools: &tools,
        input: &v,
        work: &w,
        memory_bytes: 512 << 20,
        cancel: &cancel,
    };
    assert!(matches!(pass.probe(false), Err(VideoError::Tool(m)) if m.contains("not valid JSON")));
    let tools = runtime(d.path(), "noframes");
    let pass = Pass {
        tools: &tools,
        input: &v,
        work: &w,
        memory_bytes: 512 << 20,
        cancel: &cancel,
    };
    let info = pass.probe(false).unwrap();
    assert!(matches!(pass.candidates(&info, &mut |_| {}), Err(VideoError::Tool(m)) if m.contains("no frames")));
    let t = [debrief_video::select::Candidate { ms: 0, score: 0.0 }];
    assert!(matches!(pass.extract(&info, &t, &mut |_| {}), Err(VideoError::Tool(_))));
    assert!(matches!(pass.extract_at(&info, 0), Err(VideoError::Tool(_))));
    // A file that is no video at all.
    let junk = d.path().join("junk.mp4");
    std::fs::write(&junk, b"not a video").unwrap();
    let tools = runtime(d.path(), "");
    let pass = Pass {
        tools: &tools,
        input: &junk,
        work: &w,
        memory_bytes: 512 << 20,
        cancel: &cancel,
    };
    assert!(matches!(pass.probe(false), Err(VideoError::Tool(_))));
}

#[test]
fn a_missing_runtime_says_what_to_do() {
    let _serial = serial();
    let d = tempfile::tempdir().unwrap();
    let tools = Tools::in_dir(&d.path().join("nothing"));
    let e = tools.present().unwrap_err();
    assert!(matches!(e, VideoError::NoRuntime(_)));
}
