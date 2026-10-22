//! The check every FFmpeg runtime passes before Debrief uses it (ADR 0033 §1): a downloaded build in its staging folder,
//! an update, or a user-provided `ffmpeg`. It runs the real pipeline — sandboxed, as every analysis does — on the
//! generated clip ([`super::testclip`]): `-version`, ffprobe counting the frames, the candidates pass and the extraction
//! of every frame. Six frames at 0, 0.2, … 1.0 s must come back, each a readable PNG.

use super::testclip;
use crate::analyze::Pass;
use crate::error::{Result, VideoError};
use crate::parse;
use crate::run::{self, Caps, Run, Tools};
use crate::select::Candidate;
use serde::Serialize;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Memory a self-test process may use.
const SELF_TEST_MEMORY: u64 = 512 << 20;

/// What a passed self-test found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct SelfTestReport {
    /// The version FFmpeg reports.
    pub version: Option<String>,
    /// The build includes GPL components (a user-provided FFmpeg may; the pinned ones never do).
    pub gpl: bool,
}

fn failed(step: &str, message: impl Into<String>) -> VideoError {
    VideoError::SelfTest {
        step: step.into(),
        message: message.into(),
    }
}

fn step<T>(name: &str, r: Result<T>) -> Result<T> {
    r.map_err(|e| match e {
        VideoError::Cancelled => VideoError::Cancelled,
        other => failed(name, other.to_string()),
    })
}

/// Runs the self-test of `tools` in `scratch` (a folder of its own, emptied first and removed after).
pub fn self_test(tools: &Tools, scratch: &Path, cancel: &AtomicBool) -> Result<SelfTestReport> {
    step("looking for ffmpeg and ffprobe", tools.present())?;
    if scratch.exists() {
        std::fs::remove_dir_all(scratch)?;
    }
    std::fs::create_dir_all(scratch)?;
    let result = (|| {
        let clip = scratch.join("clip.y4m");
        std::fs::write(&clip, testclip::y4m())?;
        let work = scratch.join("work");
        std::fs::create_dir_all(&work)?;
        // `-version` prints to stdout, which the runner does not capture; decoding one frame without `-hide_banner`
        // puts the banner (version and configuration) into FFmpeg's report.
        let real_clip = std::fs::canonicalize(&clip)?;
        let ran = step(
            "asking ffmpeg for its version",
            run::run(
                tools,
                Run {
                    exe: &tools.ffmpeg,
                    args: vec![
                        "-nostdin".into(),
                        "-i".into(),
                        real_clip.to_string_lossy().into_owned(),
                        "-frames:v".into(),
                        "1".into(),
                        "-f".into(),
                        "null".into(),
                        "-".into(),
                    ],
                    work: &work,
                    input: Some(&clip),
                    caps: Caps {
                        memory_bytes: SELF_TEST_MEMORY,
                        timeout: Duration::from_secs(20),
                    },
                    cancel,
                    label: "ffmpeg",
                    on_tick: None,
                },
            ),
        )?;
        let report = SelfTestReport {
            version: parse::version(&ran.report),
            gpl: parse::is_gpl(&ran.report),
        };
        let pass = Pass {
            tools,
            input: &clip,
            work: &work,
            memory_bytes: SELF_TEST_MEMORY,
            cancel,
        };
        let info = step("probing the test clip", pass.probe(true))?;
        if info.frames != Some(testclip::FRAMES as u64) || info.duration_ms.abs_diff(testclip::duration_ms()) > 50 {
            return Err(failed(
                "probing the test clip",
                format!(
                    "ffprobe reported {} frames and {} ms instead of {} frames and {} ms",
                    info.frames.map_or("no".into(), |f| f.to_string()),
                    info.duration_ms,
                    testclip::FRAMES,
                    testclip::duration_ms()
                ),
            ));
        }
        let candidates = step("analysing the test clip", pass.candidates(&info, &mut |_| {}))?;
        let times: Vec<u64> = candidates.iter().map(|c| c.ms).collect();
        if times != testclip::frame_times() {
            return Err(failed(
                "analysing the test clip",
                format!("the frames were found at {times:?} ms instead of {:?} ms", testclip::frame_times()),
            ));
        }
        let targets: Vec<Candidate> = candidates;
        let frames = step("extracting the test clip's frames", pass.extract(&info, &targets, &mut |_| {}))?;
        let got: Vec<u64> = frames.iter().map(|f| f.video_ms).collect();
        if got != testclip::frame_times() {
            return Err(failed(
                "extracting the test clip's frames",
                format!("the frames came back at {got:?} ms instead of {:?} ms", testclip::frame_times()),
            ));
        }
        for f in &frames {
            let bytes = std::fs::read(&f.path)?;
            debrief_ai::vision::image::sanitize(&bytes, 64 * 48, &Default::default())
                .map_err(|e| failed("reading the extracted frames", e.to_string()))?;
        }
        Ok(report)
    })();
    let _ = std::fs::remove_dir_all(scratch);
    result
}
