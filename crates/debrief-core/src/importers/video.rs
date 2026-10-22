//! The frames of an analysed video as a source (ADR 0033 §4): `<video name>.frames`, written by `debrief-video` into the
//! project folder, is JSON lines: a typed header (`{"videoFrames": 1, ...}`) and one line per kept frame. Each frame
//! becomes one event: time = the aligned moment, message = the AI description (or "Frame 3 at 00:01:23.400" before
//! there is one), fields `Video`, `Frame`, `VideoTime`, `Note`, `AlignedBy`, `Confidence`. `Video` identifies the
//! writer, so each video is its own timeline lane.
//!
//! The format types live here, next to their reader, so the writer (`debrief-video`) and the store worker (which only
//! has the engine's built-in importers) agree by construction.

use super::{ImportCtx, Imported, Importer};
use crate::model::{Level, Record};
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// `videoFrames` of the header this build writes and reads.
pub const FRAMES_VERSION: u32 = 1;
/// File extension of a frames source.
pub const FRAMES_EXTENSION: &str = "frames";
/// Most frame lines read from one file (a video keeps at most 120; the rest is not a frames file Debrief wrote).
pub const MAX_FRAME_LINES: usize = 1_000;
/// Longest description or note kept in an event.
pub const MAX_TEXT_CHARS: usize = 2_000;

/// The identity field of a frames source: one lane per video.
pub const VIDEO_FIELD: &str = "Video";

/// The first line of a frames file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FramesHeader {
    /// Always [`FRAMES_VERSION`].
    pub video_frames: u32,
    /// The video's file name.
    pub video: String,
    /// The video's SHA-256 (lowercase hex).
    pub sha256: String,
    pub duration_ms: u64,
}

/// How a frame's time was decided (ADR 0033 §5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AlignedBy {
    /// The user set the start or an offset.
    User,
    /// A clock on the screen, read by the vision model.
    Clock,
    /// A date and time in the file name (screen recorders name files by their start).
    FileName,
    /// The container's creation time.
    Container,
    /// The file's modification time minus the duration.
    FileTime,
    /// Nothing tells: video time 0 is the Unix epoch (shown as "not aligned").
    None,
}

impl AlignedBy {
    pub fn as_str(self) -> &'static str {
        match self {
            AlignedBy::User => "user",
            AlignedBy::Clock => "on-screen clock",
            AlignedBy::FileName => "file name",
            AlignedBy::Container => "container metadata",
            AlignedBy::FileTime => "file time",
            AlignedBy::None => "not aligned",
        }
    }
}

/// How far a frame's time can be trusted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Confidence {
    Low,
    Medium,
    High,
}

impl Confidence {
    pub fn as_str(self) -> &'static str {
        match self {
            Confidence::Low => "low",
            Confidence::Medium => "medium",
            Confidence::High => "high",
        }
    }
}

/// One kept frame.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FrameLine {
    /// Its position among the kept frames (0-based).
    pub frame: u32,
    /// Its time in the video (ms from the start).
    pub video_ms: u64,
    /// The aligned moment (epoch ms, UTC).
    pub time: i64,
    /// The AI description, when there is one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    pub aligned_by: AlignedBy,
    pub confidence: Confidence,
}

/// `00:01:23.400` (or `1:02:03.400` past an hour).
pub fn video_time(ms: u64) -> String {
    let (h, m, s, f) = (ms / 3_600_000, ms / 60_000 % 60, ms / 1000 % 60, ms % 1000);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}.{f:03}")
    } else {
        format!("{m:02}:{s:02}.{f:03}")
    }
}

fn clean(s: &str) -> String {
    let one: String = s.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    one.split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .chars()
        .take(MAX_TEXT_CHARS)
        .collect()
}

impl FrameLine {
    /// The event of this frame (line numbers: the header is line 1).
    pub fn to_record(&self, header: &FramesHeader, line: u32) -> Record {
        let mut r = Record::new(line);
        r.ts = Some(self.time);
        r.level = Level::Information;
        let at = video_time(self.video_ms);
        r.message = match self.text.as_deref().map(clean).filter(|t| !t.is_empty()) {
            Some(t) => t,
            None => format!("Frame {} at {at}", self.frame + 1),
        };
        let f = &mut r.fields;
        f.insert(VIDEO_FIELD.into(), Value::String(clean(&header.video)));
        f.insert("Frame".into(), Value::from(self.frame + 1));
        f.insert("VideoTime".into(), Value::String(at));
        if let Some(n) = self.note.as_deref().map(clean).filter(|n| !n.is_empty()) {
            f.insert("Note".into(), Value::String(n));
        }
        f.insert("AlignedBy".into(), Value::String(self.aligned_by.as_str().into()));
        f.insert("Confidence".into(), Value::String(self.confidence.as_str().into()));
        r
    }
}

/// Writes a frames file's text (header, then one line per frame).
pub fn write_frames(header: &FramesHeader, frames: &[FrameLine]) -> Result<String> {
    let mut out = serde_json::to_string(header)?;
    out.push('\n');
    for f in frames {
        out.push_str(&serde_json::to_string(f)?);
        out.push('\n');
    }
    Ok(out)
}

/// The header of `head`'s first line, when it is one this build reads.
fn header_of(head: &str) -> Option<FramesHeader> {
    let first = head.lines().next()?.trim();
    if !first.starts_with("{\"videoFrames\"") {
        return None;
    }
    serde_json::from_str::<FramesHeader>(first)
        .ok()
        .filter(|h| h.video_frames == FRAMES_VERSION)
}

/// Reads frames files (see the module docs).
pub struct VideoFramesImporter;

impl Importer for VideoFramesImporter {
    fn id(&self) -> &str {
        "video-frames"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        // Above the JSON importer (95): the header is unambiguous.
        if header_of(head).is_some() {
            100
        } else {
            0
        }
    }

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let Some(header) = header_of(content) else {
            bail!("this is not a video frames file of a version this build reads")
        };
        let mut out = Imported::default();
        for (i, line) in content.lines().enumerate().skip(1) {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            if out.records.len() >= MAX_FRAME_LINES {
                out.skipped += 1;
                continue;
            }
            match serde_json::from_str::<FrameLine>(line) {
                Ok(f) => out.records.push(f.to_record(&header, (i + 1) as u32)),
                Err(_) => out.skipped += 1,
            }
        }
        Ok(out)
    }

    fn multiline(&self) -> bool {
        false
    }

    fn identity_fields(&self) -> Vec<String> {
        vec![VIDEO_FIELD.into()]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::CompiledProfile;
    use std::path::Path;

    fn header() -> FramesHeader {
        FramesHeader {
            video_frames: 1,
            video: "checkout.mp4".into(),
            sha256: "ab".repeat(32),
            duration_ms: 90_000,
        }
    }

    fn frame(i: u32, text: Option<&str>) -> FrameLine {
        FrameLine {
            frame: i,
            video_ms: 1_500 * u64::from(i),
            time: 1_760_000_000_000 + 1_500 * i64::from(i),
            text: text.map(Into::into),
            note: (i == 1).then(|| "the dialog\nappears".into()),
            aligned_by: AlignedBy::Clock,
            confidence: Confidence::High,
        }
    }

    fn import(text: &str) -> Result<Imported> {
        let profile: CompiledProfile = crate::parser::testing::compiled("");
        let ctx = ImportCtx {
            profile: &profile,
            path: Path::new("checkout.mp4.frames"),
            file_date: None,
        };
        assert_eq!(
            VideoFramesImporter.sniff(&ctx, text),
            if header_of(text).is_some() { 100 } else { 0 }
        );
        VideoFramesImporter.import(&ctx, text)
    }

    #[test]
    fn frames_round_trip_into_one_event_each() {
        let text = write_frames(&header(), &[frame(0, Some("A login dialog.")), frame(1, None)]).unwrap();
        let got = import(&text).unwrap();
        assert_eq!(got.records.len(), 2);
        let (a, b) = (&got.records[0], &got.records[1]);
        assert_eq!(a.message, "A login dialog.");
        assert_eq!(a.ts, Some(1_760_000_000_000));
        assert_eq!(a.line, 2);
        assert_eq!(b.message, "Frame 2 at 00:01.500");
        assert_eq!(b.fields["Note"], "the dialog appears");
        assert_eq!(b.fields["Video"], "checkout.mp4");
        assert_eq!(b.fields["Frame"], 2);
        assert_eq!(b.fields["AlignedBy"], "on-screen clock");
        assert_eq!(b.fields["Confidence"], "high");
        assert_eq!(VideoFramesImporter.identity_fields(), ["Video"]);
        assert!(!VideoFramesImporter.multiline());
        assert_eq!(VideoFramesImporter.id(), "video-frames");
    }

    #[test]
    fn other_files_and_damaged_lines_are_not_trusted() {
        assert!(import("{\"timestamp\": 1}\n").is_err());
        assert!(import("{\"videoFrames\": 2, \"video\": \"x\", \"sha256\": \"\", \"durationMs\": 0}\n").is_err());
        let text = format!(
            "{}not json\n\n{}\n",
            write_frames(&header(), &[]).unwrap(),
            serde_json::to_string(&frame(0, Some("ok"))).unwrap()
        );
        let got = import(&text).unwrap();
        assert_eq!((got.records.len(), got.skipped), (1, 1));
        let many = write_frames(&header(), &vec![frame(0, None); MAX_FRAME_LINES + 3]).unwrap();
        let got = import(&many).unwrap();
        assert_eq!((got.records.len(), got.skipped), (MAX_FRAME_LINES, 3));
    }

    #[test]
    fn video_times_and_labels() {
        assert_eq!(video_time(0), "00:00.000");
        assert_eq!(video_time(83_400), "01:23.400");
        assert_eq!(video_time(3_723_004), "1:02:03.004");
        for a in [
            AlignedBy::User,
            AlignedBy::Clock,
            AlignedBy::FileName,
            AlignedBy::Container,
            AlignedBy::FileTime,
            AlignedBy::None,
        ] {
            assert!(!a.as_str().is_empty());
        }
        assert_eq!(
            [Confidence::Low, Confidence::Medium, Confidence::High].map(Confidence::as_str),
            ["low", "medium", "high"]
        );
        let long = FrameLine {
            text: Some("x".repeat(MAX_TEXT_CHARS + 50)),
            ..frame(0, None)
        };
        assert_eq!(long.to_record(&header(), 2).message.chars().count(), MAX_TEXT_CHARS);
    }
}
