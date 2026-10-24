//! The whole pipeline against a real FFmpeg, when one is installed (`ffmpeg` on the PATH, e.g. Homebrew's): a clip
//! generated with FFmpeg's `testsrc`, analysed through the sandboxed runner exactly as the app does. Skipped (with a
//! note) where there is none. No user footage is ever used.

use debrief_video::analyze::Pass;
use debrief_video::run::Tools;
use debrief_video::runtime::{custom_tools, self_test};
use debrief_video::select::{select, SelectOptions};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;

fn find_ffmpeg() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path)
        .map(|d| d.join("ffmpeg"))
        .find(|p| p.is_file() && p.with_file_name("ffprobe").is_file())
}

#[allow(clippy::disallowed_methods)] // a test generating its clip; no console window matters here
fn make_clip(ffmpeg: &Path, out: &Path) -> bool {
    // 6 s: two seconds of colour bars, two of a test pattern, two of a gradient, so there are scene changes.
    Command::new(ffmpeg)
        .args([
            "-v",
            "error",
            "-y",
            "-f",
            "lavfi",
            "-i",
            "smptebars=size=320x240:rate=10:duration=2",
        ])
        .args(["-f", "lavfi", "-i", "testsrc=size=320x240:rate=10:duration=2"])
        .args(["-f", "lavfi", "-i", "gradients=size=320x240:rate=10:duration=2:speed=0.0"])
        .args(["-filter_complex", "[0:v][1:v][2:v]concat=n=3:v=1:a=0", "-pix_fmt", "yuv420p"])
        .arg(out)
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

#[test]
fn a_generated_video_is_analysed_by_the_real_ffmpeg() {
    let Some(ffmpeg) = find_ffmpeg() else {
        eprintln!("no ffmpeg on the PATH: skipping the real-FFmpeg test");
        return;
    };
    let dir = tempfile::tempdir().unwrap();
    let clip = dir.path().join("generated.mp4");
    if !make_clip(&ffmpeg, &clip) {
        eprintln!("this ffmpeg cannot generate the test clip: skipping");
        return;
    }
    let tools: Tools = custom_tools(&ffmpeg).unwrap();
    let cancel = AtomicBool::new(false);
    let report = self_test(&tools, &dir.path().join("scratch"), &cancel).expect("the real ffmpeg passes the self-test");
    assert!(report.version.is_some());
    let work = dir.path().join("work");
    std::fs::create_dir_all(&work).unwrap();
    let pass = Pass {
        tools: &tools,
        input: &clip,
        work: &work,
        memory_bytes: 2 << 30,
        cancel: &cancel,
    };
    let info = pass.probe(false).unwrap();
    assert!((5_500..=6_500).contains(&info.duration_ms), "{info:?}");
    assert_eq!((info.width, info.height), (320, 240));
    let candidates = pass.candidates(&info, &mut |_| {}).unwrap();
    assert!(candidates.len() >= 3, "{candidates:?}");
    let kept = select(candidates, SelectOptions::default());
    let targets = kept.kept;
    let frames = pass.extract(&info, &targets, &mut |_| {}).unwrap();
    assert!(frames.len() >= 3);
    assert!(frames.windows(2).all(|w| w[0].video_ms < w[1].video_ms));
    for f in &frames {
        let bytes = std::fs::read(&f.path).unwrap();
        assert!(debrief_ai::vision::sanitize(&bytes, 1_000_000, &Default::default()).is_ok());
    }
}
