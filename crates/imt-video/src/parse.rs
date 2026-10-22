//! Reading what FFmpeg wrote (ADR 0033 §2): ffprobe's JSON, the `metadata` filter's frame list with scene scores and
//! the `showinfo` lines of FFmpeg's report. Everything is untrusted text: bounded, never panicking, unknown or
//! damaged parts skipped.

use crate::select::Candidate;
use chrono::DateTime;
use serde::Deserialize;

/// Most frame entries read from one output (a 6 h video at 30 fps with nothing dropped would be 650 000; mpdecimate
/// leaves far fewer, and the selection needs no more).
pub const MAX_ENTRIES: usize = 200_000;
/// Longest line looked at.
const MAX_LINE: usize = 4096;
/// Largest ffprobe output read.
pub const MAX_PROBE_BYTES: usize = 4 << 20;

/// What ffprobe says about a video.
#[derive(Debug, Clone, PartialEq)]
pub struct VideoInfo {
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    pub codec: String,
    pub format: String,
    /// Frames per second (average), when known.
    pub fps: Option<f64>,
    /// Frames ffprobe counted (`-count_frames`) or the container claims.
    pub frames: Option<u64>,
    /// The container's or the stream's `creation_time` (epoch ms, UTC).
    pub creation_ms: Option<i64>,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Probe {
    streams: Vec<Stream>,
    format: Format,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Stream {
    codec_type: String,
    codec_name: String,
    width: u32,
    height: u32,
    avg_frame_rate: String,
    nb_frames: Option<String>,
    nb_read_frames: Option<String>,
    duration: Option<String>,
    tags: Tags,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Format {
    format_name: String,
    duration: Option<String>,
    tags: Tags,
}

#[derive(Deserialize, Default)]
#[serde(default)]
struct Tags {
    creation_time: Option<String>,
}

/// Seconds as text ("12.345") to milliseconds; `None` for anything else (negative, `N/A`, huge).
pub fn seconds_ms(s: &str) -> Option<u64> {
    let v: f64 = s.trim().parse().ok()?;
    (v.is_finite() && (0.0..1e9).contains(&v)).then(|| (v * 1000.0).round() as u64)
}

fn rate(s: &str) -> Option<f64> {
    let (n, d) = s.split_once('/').unwrap_or((s, "1"));
    let (n, d): (f64, f64) = (n.trim().parse().ok()?, d.trim().parse().ok()?);
    let r = n / d;
    (r.is_finite() && r > 0.0 && r < 10_000.0).then_some(r)
}

fn creation(t: &Tags) -> Option<i64> {
    let s = t.creation_time.as_deref()?.trim();
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
        // Some muxers write the epoch or 1904-01-01 when they know nothing.
        .filter(|ms| *ms > 0)
}

/// Parses ffprobe's `-print_format json -show_format -show_streams` output. `Err` says why it is not usable.
pub fn probe(json: &str) -> Result<VideoInfo, String> {
    if json.len() > MAX_PROBE_BYTES {
        return Err("ffprobe's answer is too large".into());
    }
    let p: Probe = serde_json::from_str(json).map_err(|e| format!("ffprobe's answer is not valid JSON ({e})"))?;
    let v = p
        .streams
        .iter()
        .find(|s| s.codec_type == "video" && s.width > 0 && s.height > 0)
        .ok_or("the file has no video stream")?;
    let duration_ms = p
        .format
        .duration
        .as_deref()
        .and_then(seconds_ms)
        .or_else(|| v.duration.as_deref().and_then(seconds_ms))
        .ok_or("ffprobe could not tell the video's length")?;
    let count = |s: &Option<String>| s.as_deref().and_then(|n| n.trim().parse::<u64>().ok()).filter(|n| *n > 0);
    Ok(VideoInfo {
        duration_ms,
        width: v.width.min(65_535),
        height: v.height.min(65_535),
        codec: v.codec_name.chars().take(40).collect(),
        format: p.format.format_name.chars().take(80).collect(),
        fps: rate(&v.avg_frame_rate),
        frames: count(&v.nb_read_frames).or_else(|| count(&v.nb_frames)),
        creation_ms: creation(&p.format.tags).or_else(|| creation(&v.tags)),
    })
}

/// The value after `key` up to the next whitespace (`pts_time:12.5 ` → `12.5`).
fn value_after<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    let at = line.find(key)? + key.len();
    line[at..].split_whitespace().next()
}

fn short(line: &str) -> &str {
    let mut end = line.len().min(MAX_LINE);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    &line[..end]
}

/// The `metadata=mode=print` output: `frame:N pts:P pts_time:T` lines, each followed by `key=value` lines of that
/// frame (`lavfi.scene_score=0.123`). Frames without a score get 0.
pub fn candidates(text: &str) -> Vec<Candidate> {
    let mut out: Vec<Candidate> = Vec::new();
    // Whether the last `frame:` line was kept: a score belongs to the frame line before it, kept or not.
    let mut open = false;
    for line in text.lines().map(short) {
        let line = line.trim();
        if line.starts_with("frame:") {
            if out.len() >= MAX_ENTRIES {
                break;
            }
            open = false;
            if let Some(ms) = value_after(line, "pts_time:").and_then(seconds_ms) {
                out.push(Candidate { ms, score: 0.0 });
                open = true;
            }
        } else if let Some(v) = line.strip_prefix("lavfi.scene_score=") {
            if let (true, Some(last), Ok(s)) = (open, out.last_mut(), v.trim().parse::<f32>()) {
                last.score = s;
            }
        }
    }
    out
}

/// The frame times `showinfo` reported (in FFmpeg's report): `... n: 3 pts: 1536 pts_time:0.6 ...`, in order.
pub fn showinfo_times(report: &str) -> Vec<u64> {
    report
        .lines()
        .map(short)
        .filter(|l| l.contains("Parsed_showinfo") && l.contains(" n:"))
        .filter_map(|l| value_after(l, "pts_time:").and_then(seconds_ms))
        .take(MAX_ENTRIES)
        .collect()
}

/// The last error lines of FFmpeg's report (for a message that says what went wrong), without paths' contents: at
/// most `n` lines of 300 characters.
pub fn report_errors(report: &str, n: usize) -> Vec<String> {
    let mut lines: Vec<String> = report
        .lines()
        .map(short)
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .filter(|l| {
            let lower = l.to_ascii_lowercase();
            ["error", "invalid", "could not", "no such", "not supported", "unknown", "failed"]
                .iter()
                .any(|k| lower.contains(k))
        })
        .map(|l| l.chars().take(300).collect())
        .collect();
    let skip = lines.len().saturating_sub(n);
    lines.drain(..skip);
    lines
}

/// The version FFmpeg reports for `-version` (`ffmpeg version n9.0.2-17-g2a571b6068 Copyright…`; in its report the
/// line follows the report's own header).
pub fn version(text: &str) -> Option<String> {
    text.lines().map(short).map(str::trim).find_map(|l| {
        let rest = l.strip_prefix("ffmpeg version ").or_else(|| l.strip_prefix("ffprobe version "))?;
        rest.split_whitespace().next().map(|v| v.chars().take(80).collect())
    })
}

/// Whether `-version` says the build is GPL (`--enable-gpl` in its configuration).
pub fn is_gpl(text: &str) -> bool {
    text.contains("--enable-gpl")
}

#[cfg(test)]
mod tests {
    use super::*;

    const PROBE: &str = r#"{
      "streams": [
        {"index": 0, "codec_type": "audio", "codec_name": "aac"},
        {"index": 1, "codec_type": "video", "codec_name": "h264", "width": 1920, "height": 1080,
         "avg_frame_rate": "30000/1001", "nb_frames": "2697", "tags": {"creation_time": "2026-10-21T08:41:00.250000Z"}}
      ],
      "format": {"format_name": "mov,mp4", "duration": "90.000000", "tags": {}}
    }"#;

    #[test]
    fn ffprobe_answers_are_read_into_typed_values() {
        let v = probe(PROBE).unwrap();
        assert_eq!((v.duration_ms, v.width, v.height, v.codec.as_str()), (90_000, 1920, 1080, "h264"));
        assert!((v.fps.unwrap() - 29.97).abs() < 0.01);
        assert_eq!(v.frames, Some(2697));
        let at = DateTime::parse_from_rfc3339("2026-10-21T08:41:00.25Z").unwrap().timestamp_millis();
        assert_eq!(v.creation_ms, Some(at));
        assert_eq!(v.format, "mov,mp4");
        let counted = PROBE.replace(r#""nb_frames": "2697""#, r#""nb_frames": "2697", "nb_read_frames": "6""#);
        assert_eq!(probe(&counted).unwrap().frames, Some(6));
        let epoch = PROBE.replace("2026-10-21T08:41:00.250000Z", "1970-01-01T00:00:00Z");
        assert_eq!(probe(&epoch).unwrap().creation_ms, None);
        let from_stream = PROBE
            .replace(r#""duration": "90.000000","#, "")
            .replace(r#""nb_frames""#, r#""duration": "5.5", "nb_frames""#);
        assert_eq!(probe(&from_stream).unwrap().duration_ms, 5_500);
    }

    #[test]
    fn unusable_ffprobe_answers_say_why() {
        assert!(probe("not json").unwrap_err().contains("not valid JSON"));
        assert!(probe(r#"{"streams": [{"codec_type": "audio"}]}"#).unwrap_err().contains("no video"));
        assert!(probe(r#"{"streams": [{"codec_type": "video", "width": 2, "height": 2}]}"#)
            .unwrap_err()
            .contains("length"));
        assert!(probe(&" ".repeat(MAX_PROBE_BYTES + 1)).unwrap_err().contains("too large"));
        assert_eq!(seconds_ms("N/A"), None);
        assert_eq!(seconds_ms("-0.033"), None);
        assert_eq!(seconds_ms("1.0005"), Some(1001));
        assert_eq!(rate("0/0"), None);
        assert_eq!(rate("25"), Some(25.0));
    }

    #[test]
    fn candidates_and_their_scores() {
        let text = "frame:0    pts:0       pts_time:0\nlavfi.scene_score=0.000000\nframe:1    pts:1024    pts_time:2.5\n\
                    lavfi.scene_score=0.512\nframe:2 pts:x pts_time:N/A\nlavfi.scene_score=0.9\nframe:3 pts:9 pts_time:4\n";
        let c = candidates(text);
        assert_eq!(c.len(), 3);
        assert_eq!((c[0].ms, c[1].ms, c[1].score, c[2].ms, c[2].score), (0, 2500, 0.512, 4000, 0.0));
        assert!(candidates(&"x".repeat(100_000)).is_empty());
        assert!(candidates("lavfi.scene_score=1\n").is_empty());
    }

    #[test]
    fn showinfo_lines_give_exact_times() {
        let report = "[Parsed_showinfo_2 @ 0000020a] config in time_base: 1/1000\n\
                      [Parsed_showinfo_2 @ 0000020a] n:   0 pts:      0 pts_time:0       duration:200 fmt:yuv420p\n\
                      [Parsed_showinfo_2 @ 0000020a] n:   1 pts:    200 pts_time:0.2     duration:200\n\
                      [out#0/image2 @ 0000] video:12KiB\n";
        assert_eq!(showinfo_times(report), [0, 200]);
        assert_eq!(showinfo_times("ünïcödé ".repeat(2000).as_str()), Vec::<u64>::new());
    }

    #[test]
    fn errors_versions_and_licences() {
        let r = "ffmpeg started\n[in] Invalid data found when processing input\nok\nError opening input file x.mp4.\n";
        assert_eq!(report_errors(r, 1), ["Error opening input file x.mp4."]);
        assert_eq!(report_errors(r, 5).len(), 2);
        let v = "ffmpeg version n9.0.2-17-g2a571b6068-20260930 Copyright (c) 2000-2026\nconfiguration: --enable-shared --disable-gpl\n";
        assert_eq!(version(v).as_deref(), Some("n9.0.2-17-g2a571b6068-20260930"));
        assert_eq!(version("nothing"), None);
        assert!(!is_gpl(v) && is_gpl("configuration: --enable-gpl --enable-libx264"));
    }
}
