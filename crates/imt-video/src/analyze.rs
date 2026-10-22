//! The FFmpeg passes over one video (ADR 0033 §3): probe it, list the candidates (`mpdecimate` + scene scores), and
//! extract exactly the selected frames (`select` on their times, scaled for the vision model, `showinfo` for their
//! exact times). The argument lists are pure functions (tested); [`crate::run`] runs them sandboxed.

use crate::error::{Result, VideoError};
use crate::parse::{self, VideoInfo};
use crate::run::{self, Caps, Run, Tools, PROGRESS_FILE};
use crate::select::Candidate;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::time::Duration;

/// Longest video analysed (6 h).
pub const MAX_DURATION_MS: u64 = 6 * 3_600_000;
/// Width the candidates pass works at: small frames make a cursor or a clock too small to count as a change.
pub const ANALYSIS_WIDTH: u32 = 256;
/// Longest side of an extracted frame (what the vision model reads, ADR 0025).
pub const FRAME_SIDE: u32 = 1366;
/// The candidates pass's metadata file in the work folder.
pub const CANDIDATES_FILE: &str = "candidates.txt";
/// The probe's output in the work folder.
pub const PROBE_FILE: &str = "probe.json";
/// Prefix of the extracted frames in the work folder.
pub const FRAME_PREFIX: &str = "frame-";

fn s(v: &str) -> String {
    v.to_string()
}

fn input_arg(input: &Path) -> String {
    input.to_string_lossy().into_owned()
}

/// `ffprobe` of the video's format and streams into [`PROBE_FILE`]; `count_frames` decodes it to count them (slow:
/// the self-test only).
pub fn probe_args(input: &Path, count_frames: bool) -> Vec<String> {
    let mut a = vec![
        s("-hide_banner"),
        s("-v"),
        s("error"),
        s("-print_format"),
        s("json"),
        s("-show_format"),
        s("-show_streams"),
    ];
    if count_frames {
        a.push(s("-count_frames"));
    }
    a.extend([s("-o"), s(PROBE_FILE), input_arg(input)]);
    a
}

/// The start of every ffmpeg run: no stdin, no banner, the first video stream only.
fn ffmpeg_head(input: &Path) -> Vec<String> {
    vec![
        s("-nostdin"),
        s("-hide_banner"),
        s("-progress"),
        s(PROGRESS_FILE),
        s("-i"),
        input_arg(input),
        s("-map"),
        s("0:v:0"),
        s("-an"),
        s("-sn"),
        s("-dn"),
    ]
}

/// The candidates pass: scaled down, near-duplicates dropped (`mpdecimate`, tolerant at this size of a blinking
/// cursor and a ticking clock), the scene score of each remaining frame written with its time.
pub fn candidates_args(input: &Path) -> Vec<String> {
    let mut a = ffmpeg_head(input);
    a.extend([
        s("-vf"),
        format!(
            "scale={ANALYSIS_WIDTH}:-2:flags=fast_bilinear,mpdecimate=hi=64*16:lo=64*6:frac=0.33,select='gte(scene,0)',metadata=mode=print:file={CANDIDATES_FILE}"
        ),
        s("-fps_mode"),
        s("vfr"),
        s("-f"),
        s("null"),
        s("-"),
    ]);
    a
}

/// How close a frame's time must be to a target: half a millisecond, more for long videos whose times FFmpeg prints
/// with fewer decimals.
fn tolerance_s(ms: u64) -> f64 {
    0.0005 + ms as f64 / 1000.0 * 5e-6
}

/// The extraction pass of `targets` (their exact times), scaled for the vision model, `showinfo` reporting each.
pub fn extract_args(input: &Path, targets_ms: &[u64]) -> Vec<String> {
    let terms: Vec<String> = targets_ms
        .iter()
        .map(|ms| format!("lt(abs(t-{:.6}),{:.6})", *ms as f64 / 1000.0, tolerance_s(*ms)))
        .collect();
    let mut a = ffmpeg_head(input);
    a.extend([
        s("-vf"),
        format!(
            "select='{}',scale=w='min(iw,{FRAME_SIDE})':h='min(ih,{FRAME_SIDE})':force_original_aspect_ratio=decrease,showinfo",
            terms.join("+")
        ),
        s("-fps_mode"),
        s("passthrough"),
        s("-f"),
        s("image2"),
        format!("{FRAME_PREFIX}%04d.png"),
    ]);
    a
}

/// The first frame at or after `ms` (the user adds the frame at the player's position).
pub fn extract_at_args(input: &Path, ms: u64) -> Vec<String> {
    let mut a = ffmpeg_head(input);
    a.extend([
        s("-vf"),
        format!(
            "select='gte(t,{:.6})',scale=w='min(iw,{FRAME_SIDE})':h='min(ih,{FRAME_SIDE})':force_original_aspect_ratio=decrease,showinfo",
            ms as f64 / 1000.0
        ),
        s("-frames:v"),
        s("1"),
        s("-fps_mode"),
        s("passthrough"),
        s("-f"),
        s("image2"),
        format!("{FRAME_PREFIX}%04d.png"),
    ]);
    a
}

/// One extracted frame in the work folder.
#[derive(Debug, Clone, PartialEq)]
pub struct Extracted {
    /// Its exact time (`showinfo`).
    pub video_ms: u64,
    pub path: PathBuf,
    pub score: f32,
}

/// Matches the extracted files (in output order, with `showinfo`'s times) to the targets: for each target the
/// nearest output, each output at most once. Pure.
pub fn match_outputs(targets: &[Candidate], times: &[u64], files: &[PathBuf]) -> Vec<Extracted> {
    let outputs: Vec<(u64, &PathBuf)> = times.iter().copied().zip(files).collect();
    let mut used = vec![false; outputs.len()];
    let mut out = Vec::new();
    for t in targets {
        let best = (0..outputs.len())
            .filter(|i| !used[*i])
            .min_by_key(|i| (outputs[*i].0.abs_diff(t.ms), *i));
        if let Some(i) = best {
            // A target FFmpeg did not decode (a damaged stretch) is left out rather than matched far away.
            if outputs[i].0.abs_diff(t.ms) <= 1000 {
                used[i] = true;
                out.push(Extracted {
                    video_ms: outputs[i].0,
                    path: outputs[i].1.clone(),
                    score: t.score,
                });
            }
        }
    }
    out.sort_by_key(|e| e.video_ms);
    out
}

/// The `frame-NNNN.png` files of the work folder, in order.
fn frame_files(work: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(work)
        .map(|it| {
            it.flatten()
                .map(|e| e.path())
                .filter(|p| {
                    p.file_name()
                        .and_then(|n| n.to_str())
                        .is_some_and(|n| n.starts_with(FRAME_PREFIX) && n.ends_with(".png"))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort();
    v
}

/// What a pass needs: the tools, the video, a work folder, the caps and Cancel.
pub struct Pass<'a> {
    pub tools: &'a Tools,
    pub input: &'a Path,
    pub work: &'a Path,
    pub memory_bytes: u64,
    pub cancel: &'a AtomicBool,
}

impl Pass<'_> {
    /// The video's real path: the sandbox opens it by the path it is given and cannot resolve links outside its folders.
    fn real_input(&self) -> PathBuf {
        std::fs::canonicalize(self.input).unwrap_or_else(|_| self.input.to_path_buf())
    }

    fn run<'b>(
        &'b self,
        exe: &'b Path,
        args: Vec<String>,
        timeout: Duration,
        label: &'b str,
        on_tick: Option<&'b mut dyn FnMut()>,
    ) -> Result<run::Ran> {
        run::run(
            self.tools,
            Run {
                exe,
                args,
                work: self.work,
                input: Some(self.input),
                caps: Caps {
                    memory_bytes: self.memory_bytes,
                    timeout,
                },
                cancel: self.cancel,
                label,
                on_tick,
            },
        )
    }

    /// Probes the video; refuses one without a video stream or longer than [`MAX_DURATION_MS`].
    pub fn probe(&self, count_frames: bool) -> Result<VideoInfo> {
        let _ = std::fs::remove_file(self.work.join(PROBE_FILE));
        self.run(
            &self.tools.ffprobe,
            probe_args(&self.real_input(), count_frames),
            Duration::from_secs(30),
            "ffprobe",
            None,
        )?;
        let info = parse::probe(&run::output(self.work, PROBE_FILE)).map_err(VideoError::Tool)?;
        if info.duration_ms > MAX_DURATION_MS {
            return Err(VideoError::Limit(format!(
                "the video is {} min long; IMT analyses videos of up to {} h: cut it first",
                info.duration_ms / 60_000,
                MAX_DURATION_MS / 3_600_000
            )));
        }
        Ok(info)
    }

    /// The candidates; `progress` hears the share done.
    pub fn candidates(&self, info: &VideoInfo, progress: &mut dyn FnMut(f64)) -> Result<Vec<Candidate>> {
        let work = self.work.to_path_buf();
        let duration = info.duration_ms;
        let mut tick = || {
            if let Some(p) = run::progress_of(&work, duration) {
                progress(p);
            }
        };
        let _ = std::fs::remove_file(self.work.join(CANDIDATES_FILE));
        self.run(
            &self.tools.ffmpeg,
            candidates_args(&self.real_input()),
            run::pass_timeout(duration),
            "the frame analysis",
            Some(&mut tick),
        )?;
        let c = parse::candidates(&run::output(self.work, CANDIDATES_FILE));
        if c.is_empty() {
            return Err(VideoError::Tool("FFmpeg found no frames in the video: it may be damaged".into()));
        }
        Ok(c)
    }

    fn clear_frames(&self) {
        for f in frame_files(self.work) {
            let _ = std::fs::remove_file(f);
        }
    }

    /// Extracts `targets` (selected candidates).
    pub fn extract(&self, info: &VideoInfo, targets: &[Candidate], progress: &mut dyn FnMut(f64)) -> Result<Vec<Extracted>> {
        if targets.is_empty() {
            return Ok(Vec::new());
        }
        self.clear_frames();
        let work = self.work.to_path_buf();
        let duration = info.duration_ms;
        let mut tick = || {
            if let Some(p) = run::progress_of(&work, duration) {
                progress(p);
            }
        };
        let ms: Vec<u64> = targets.iter().map(|t| t.ms).collect();
        let ran = self.run(
            &self.tools.ffmpeg,
            extract_args(&self.real_input(), &ms),
            run::pass_timeout(duration),
            "the frame extraction",
            Some(&mut tick),
        )?;
        let got = match_outputs(targets, &parse::showinfo_times(&ran.report), &frame_files(self.work));
        if got.is_empty() {
            return Err(VideoError::Tool("FFmpeg extracted no frames".into()));
        }
        Ok(got)
    }

    /// The first frame at or after `ms`.
    pub fn extract_at(&self, info: &VideoInfo, ms: u64) -> Result<Extracted> {
        if ms > info.duration_ms {
            return Err(VideoError::Invalid("that time is after the end of the video".into()));
        }
        self.clear_frames();
        let ran = self.run(
            &self.tools.ffmpeg,
            extract_at_args(&self.real_input(), ms),
            run::pass_timeout(info.duration_ms),
            "the frame extraction",
            None,
        )?;
        let times = parse::showinfo_times(&ran.report);
        let files = frame_files(self.work);
        match (times.first(), files.first()) {
            (Some(t), Some(f)) => Ok(Extracted {
                video_ms: *t,
                path: f.clone(),
                score: 0.0,
            }),
            _ => Err(VideoError::Tool("FFmpeg found no frame at that time".into())),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(ms: u64) -> Candidate {
        Candidate { ms, score: 0.3 }
    }

    #[test]
    fn argument_lists_never_go_through_a_shell() {
        let input = Path::new("/videos/a b;rm -rf.mp4");
        let p = probe_args(input, true);
        assert_eq!(p.last().unwrap(), "/videos/a b;rm -rf.mp4");
        assert!(p.contains(&"-count_frames".to_string()) && !probe_args(input, false).contains(&"-count_frames".to_string()));
        let a = candidates_args(input);
        assert_eq!(&a[..2], ["-nostdin", "-hide_banner"]);
        assert!(a
            .iter()
            .any(|x| x.contains("mpdecimate") && x.contains("metadata=mode=print:file=candidates.txt")));
        assert_eq!(a.iter().filter(|x| *x == "/videos/a b;rm -rf.mp4").count(), 1);
        let e = extract_args(input, &[0, 1500]);
        let vf = &e[e.iter().position(|x| x == "-vf").unwrap() + 1];
        assert!(
            vf.starts_with("select='lt(abs(t-0.000000),0.000500)+lt(abs(t-1.500000),0.0005"),
            "{vf}"
        );
        assert!(vf.contains(")',scale="), "{vf}");
        assert!(vf.ends_with(",showinfo"));
        assert_eq!(e.last().unwrap(), "frame-%04d.png");
        let at = extract_at_args(input, 2_250);
        assert!(at.iter().any(|x| x.starts_with("select='gte(t,2.250000)'")));
        assert!(at.windows(2).any(|w| w == ["-frames:v", "1"]));
    }

    #[test]
    fn outputs_match_their_nearest_target_once() {
        let files: Vec<PathBuf> = (1..=4).map(|i| PathBuf::from(format!("frame-{i:04}.png"))).collect();
        let got = match_outputs(&[c(0), c(1000), c(1000), c(9000)], &[0, 999, 1001, 20_000], &files);
        let ms: Vec<u64> = got.iter().map(|e| e.video_ms).collect();
        assert_eq!(ms, [0, 999, 1001], "a far-away output is not a match");
        assert_eq!(got[1].path, files[1]);
        assert!(match_outputs(&[c(5)], &[], &[]).is_empty());
        assert!(
            match_outputs(&[c(5)], &[5, 6], &files[..1]).len() == 1,
            "times without a file are ignored"
        );
    }

    #[test]
    fn frame_files_are_listed_in_order() {
        let dir = tempfile::tempdir().unwrap();
        for n in ["frame-0002.png", "frame-0001.png", "other.png", "frame-0003.txt"] {
            std::fs::write(dir.path().join(n), b"x").unwrap();
        }
        let names: Vec<String> = frame_files(dir.path())
            .iter()
            .map(|p| p.file_name().unwrap().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names, ["frame-0001.png", "frame-0002.png"]);
        assert!(frame_files(&dir.path().join("none")).is_empty());
    }
}
