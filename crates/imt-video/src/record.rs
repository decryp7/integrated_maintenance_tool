//! What the project keeps of a video (ADR 0033 §4), in the project (store) folder:
//!
//! ```text
//! videos/<key>/video.json        the VideoRecord (versioned)
//! videos/<key>/frames/0003.png   the kept frames, sanitised (PNG or JPEG)
//! videos/<key>/<name>.frames     the frames source the store reads (one event per frame)
//! videos/<key>/media/<name>      the video itself, only when "Include the video in the project" is on
//! ```
//!
//! `<key>` is the first 16 hex digits of the video's SHA-256. A damaged or newer `video.json` is skipped with a
//! warning, never an error. Writes go through a temporary file and a rename.

use crate::align::{align, AlignInput, Alignment, ClockReading, UserAlignment};
use crate::error::{Result, VideoError};
use crate::select::{SelectNote, SelectOptions};
use imt_core::importers::video::{write_frames, FrameLine, FramesHeader, FRAMES_EXTENSION, FRAMES_VERSION, MAX_TEXT_CHARS};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// `version` of the record this build writes and reads.
pub const RECORD_VERSION: u32 = 1;
/// The folder of the videos inside the project folder.
pub const VIDEOS: &str = "videos";
pub const RECORD_FILE: &str = "video.json";
pub const FRAMES_DIR: &str = "frames";
pub const MEDIA_DIR: &str = "media";
/// Most frames all the videos of one project keep together.
pub const MAX_PROJECT_FRAMES: usize = 600;
/// Largest `video.json` read.
const MAX_RECORD_BYTES: u64 = 8 << 20;

/// One kept frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct FrameRecord {
    /// Its exact time in the video (ms), as FFmpeg's `showinfo` reported it.
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub video_ms: u64,
    /// Its image, relative to the video's folder (`frames/0003.png`).
    pub image: String,
    pub width: u32,
    pub height: u32,
    /// How much it differed from the frame before (FFmpeg's scene score, 0–1).
    #[serde(default)]
    pub score: f32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// Added by the user (not chosen by the selection).
    #[serde(default)]
    pub manual: bool,
}

/// A video of the project.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "ts", derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct VideoRecord {
    pub version: u32,
    pub key: String,
    /// The file name.
    pub name: String,
    pub sha256: String,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub size: u64,
    /// Where the file was on the machine that analysed it (it is referenced, not copied, by default).
    #[serde(default)]
    pub path: Option<String>,
    /// A copy is in `media/` (and goes with the project).
    #[serde(default)]
    pub included: bool,
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub duration_ms: u64,
    pub width: u32,
    pub height: u32,
    #[serde(default)]
    pub codec: String,
    #[serde(default)]
    pub fps: Option<f64>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub creation_ms: Option<i64>,
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number | null"))]
    pub file_modified_ms: Option<i64>,
    #[serde(default)]
    pub options: SelectOptions,
    #[serde(default)]
    pub user_alignment: UserAlignment,
    #[serde(default)]
    pub clock: Option<ClockReading>,
    /// The alignment as last decided (kept for display; decided again whenever its inputs change).
    pub alignment: Alignment,
    #[serde(default)]
    pub selection_notes: Vec<SelectNote>,
    pub frames: Vec<FrameRecord>,
    /// When it was analysed (epoch ms).
    #[serde(default)]
    #[cfg_attr(feature = "ts", ts(type = "number"))]
    pub analysed_ms: i64,
}

/// The key of a video: the first 16 hex digits of its SHA-256.
pub fn key_of(sha256: &str) -> String {
    sha256.chars().take(16).collect()
}

/// Whether `k` can be a key (and so a folder name).
pub fn is_key(k: &str) -> bool {
    k.len() == 16 && k.chars().all(|c| c.is_ascii_hexdigit() && !c.is_ascii_uppercase())
}

/// A file name safe in any folder: letters, digits, space, `.-_()`; at most 100 characters; never empty or hidden.
pub fn safe_file_name(name: &str) -> String {
    let s: String = name
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || matches!(c, ' ' | '.' | '-' | '_' | '(' | ')') {
                c
            } else {
                '_'
            }
        })
        .take(100)
        .collect();
    let s = s.trim().trim_start_matches('.').to_string();
    if s.is_empty() {
        "video".into()
    } else {
        s
    }
}

/// Description or note text as kept: one line, no control characters, at most 2 000 characters; `None` when empty.
pub fn clean_text(s: &str) -> Option<String> {
    let t: String = s.chars().map(|c| if c.is_control() && c != '\n' { ' ' } else { c }).collect();
    let t = t.trim();
    (!t.is_empty()).then(|| t.chars().take(MAX_TEXT_CHARS).collect())
}

impl VideoRecord {
    /// Decides the alignment again from the record's evidence and the profile's zone.
    pub fn realign(&mut self, tz: &str) {
        self.alignment = align(&AlignInput {
            user: self.user_alignment,
            clock: self.clock.as_ref(),
            file_name: &self.name,
            creation_ms: self.creation_ms,
            file_modified_ms: self.file_modified_ms,
            duration_ms: self.duration_ms,
            tz,
        });
    }

    /// The frames source's file name.
    pub fn source_file_name(&self) -> String {
        format!("{}.{FRAMES_EXTENSION}", safe_file_name(&self.name))
    }

    /// The frames source's text.
    pub fn frames_text(&self) -> Result<String> {
        let header = FramesHeader {
            video_frames: FRAMES_VERSION,
            video: self.name.clone(),
            sha256: self.sha256.clone(),
            duration_ms: self.duration_ms,
        };
        let lines: Vec<FrameLine> = self
            .frames
            .iter()
            .enumerate()
            .map(|(i, f)| FrameLine {
                frame: i as u32,
                video_ms: f.video_ms,
                time: self.alignment.at(f.video_ms),
                text: f.description.clone(),
                note: f.note.clone(),
                aligned_by: self.alignment.by,
                confidence: self.alignment.confidence,
            })
            .collect();
        write_frames(&header, &lines).map_err(|e| VideoError::Io(e.to_string()))
    }

    /// The frame at `index`.
    pub fn frame(&self, index: usize) -> Result<&FrameRecord> {
        self.frames
            .get(index)
            .ok_or_else(|| VideoError::NotFound(format!("frame {} of {}", index + 1, self.name)))
    }

    /// Parses `video.json`, refusing records of other versions and keys that do not match their hash.
    pub fn parse(text: &str) -> Result<VideoRecord> {
        let r: VideoRecord = serde_json::from_str(text).map_err(|e| VideoError::Invalid(format!("the video record is damaged ({e})")))?;
        if r.version != RECORD_VERSION {
            return Err(VideoError::Invalid(format!(
                "the video record has version {}, this build reads {RECORD_VERSION}",
                r.version
            )));
        }
        if !is_key(&r.key) || key_of(&r.sha256) != r.key {
            return Err(VideoError::Invalid("the video record does not match its video".into()));
        }
        if r.frames.iter().any(|f| !is_frame_image(&f.image)) {
            return Err(VideoError::Invalid("the video record names a frame outside its folder".into()));
        }
        Ok(r)
    }
}

/// `frames/<digits>.png|jpg`: the only images a record may name.
pub fn is_frame_image(rel: &str) -> bool {
    let Some(name) = rel.strip_prefix("frames/") else { return false };
    let Some((stem, ext)) = name.rsplit_once('.') else { return false };
    !stem.is_empty() && stem.len() <= 6 && stem.chars().all(|c| c.is_ascii_digit()) && matches!(ext, "png" | "jpg")
}

/// The videos of one project folder.
pub struct VideoFolder {
    root: PathBuf,
}

/// What [`VideoFolder::list`] found.
#[derive(Debug, Default)]
pub struct Listed {
    pub videos: Vec<VideoRecord>,
    /// Folders whose record could not be used, with why (shown as a warning).
    pub skipped: Vec<(String, String)>,
}

fn write_atomic(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        VideoError::from(e)
    })
}

impl VideoFolder {
    /// The videos of the project folder `project`.
    pub fn of_project(project: &Path) -> VideoFolder {
        VideoFolder {
            root: project.join(VIDEOS),
        }
    }

    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The folder of the video `key` (checked).
    pub fn dir(&self, key: &str) -> Result<PathBuf> {
        if !is_key(key) {
            return Err(VideoError::Invalid("that is not a video of this project".into()));
        }
        Ok(self.root.join(key))
    }

    /// Every readable record, by name; damaged ones are listed in `skipped`.
    pub fn list(&self) -> Listed {
        let mut out = Listed::default();
        let Ok(entries) = std::fs::read_dir(&self.root) else { return out };
        for e in entries.flatten() {
            let key = e.file_name().to_string_lossy().into_owned();
            if !is_key(&key) || !e.path().is_dir() {
                continue;
            }
            match self.load(&key) {
                Ok(r) => out.videos.push(r),
                Err(err) => out.skipped.push((key, err.to_string())),
            }
        }
        out.videos.sort_by(|a, b| a.name.cmp(&b.name).then(a.key.cmp(&b.key)));
        out
    }

    /// The record of `key`.
    pub fn load(&self, key: &str) -> Result<VideoRecord> {
        let path = self.dir(key)?.join(RECORD_FILE);
        let f = std::fs::File::open(&path).map_err(|_| VideoError::NotFound(format!("the video {key}")))?;
        let mut text = String::new();
        use std::io::Read;
        f.take(MAX_RECORD_BYTES + 1).read_to_string(&mut text)?;
        if text.len() as u64 > MAX_RECORD_BYTES {
            return Err(VideoError::Invalid("the video record is too large".into()));
        }
        let r = VideoRecord::parse(&text)?;
        if r.key != key {
            return Err(VideoError::Invalid("the video record is in another video's folder".into()));
        }
        Ok(r)
    }

    /// Frames the other videos of the project keep.
    pub fn frames_besides(&self, key: &str) -> usize {
        self.list().videos.iter().filter(|v| v.key != key).map(|v| v.frames.len()).sum()
    }

    /// Saves `r` (record and frames source); refuses more than [`MAX_PROJECT_FRAMES`] frames in the project.
    /// Returns the frames source's path.
    pub fn save(&self, r: &VideoRecord) -> Result<PathBuf> {
        let dir = self.dir(&r.key)?;
        if key_of(&r.sha256) != r.key {
            return Err(VideoError::Invalid("the video record does not match its video".into()));
        }
        let others = self.frames_besides(&r.key);
        if others + r.frames.len() > MAX_PROJECT_FRAMES {
            return Err(VideoError::Limit(format!(
                "a project keeps at most {MAX_PROJECT_FRAMES} video frames and the other videos keep {others}: keep fewer frames or remove a video"
            )));
        }
        std::fs::create_dir_all(dir.join(FRAMES_DIR))?;
        let json = serde_json::to_vec_pretty(r).map_err(|e| VideoError::Io(e.to_string()))?;
        write_atomic(&dir.join(RECORD_FILE), &json)?;
        // A renamed source (another name) leaves no old frames file behind.
        let source = dir.join(r.source_file_name());
        if let Ok(entries) = std::fs::read_dir(&dir) {
            for e in entries.flatten() {
                let p = e.path();
                if p.extension().is_some_and(|x| x == FRAMES_EXTENSION) && p != source {
                    let _ = std::fs::remove_file(p);
                }
            }
        }
        write_atomic(&source, r.frames_text()?.as_bytes())?;
        // Frame images no record names any more.
        if let Ok(entries) = std::fs::read_dir(dir.join(FRAMES_DIR)) {
            for e in entries.flatten() {
                let rel = format!("{FRAMES_DIR}/{}", e.file_name().to_string_lossy());
                if !r.frames.iter().any(|f| f.image == rel) {
                    let _ = std::fs::remove_file(e.path());
                }
            }
        }
        Ok(source)
    }

    /// The image file of a frame.
    pub fn frame_path(&self, r: &VideoRecord, index: usize) -> Result<PathBuf> {
        let f = r.frame(index)?;
        if !is_frame_image(&f.image) {
            return Err(VideoError::Invalid("the frame is outside its folder".into()));
        }
        Ok(self.dir(&r.key)?.join(&f.image))
    }

    /// The included copy of the video, when there is one.
    pub fn media_path(&self, r: &VideoRecord) -> Result<Option<PathBuf>> {
        let p = self.dir(&r.key)?.join(MEDIA_DIR).join(safe_file_name(&r.name));
        Ok(p.is_file().then_some(p))
    }

    /// Copies the video into the project (`media/`), replacing an earlier copy.
    pub fn include(&self, r: &VideoRecord, from: &Path) -> Result<PathBuf> {
        let dir = self.dir(&r.key)?.join(MEDIA_DIR);
        std::fs::create_dir_all(&dir)?;
        let dest = dir.join(safe_file_name(&r.name));
        let part = dest.with_extension("part");
        std::fs::copy(from, &part).map_err(|e| {
            let _ = std::fs::remove_file(&part);
            VideoError::from(e)
        })?;
        std::fs::rename(&part, &dest)?;
        Ok(dest)
    }

    /// Removes the included copy.
    pub fn exclude(&self, r: &VideoRecord) -> Result<()> {
        let dir = self.dir(&r.key)?.join(MEDIA_DIR);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }

    /// Removes the video's folder (record, frames, source, copy).
    pub fn remove(&self, key: &str) -> Result<()> {
        let dir = self.dir(key)?;
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
        }
        Ok(())
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use imt_core::importers::video::{AlignedBy, Confidence};

    pub(crate) fn record(sha: &str, frames: usize) -> VideoRecord {
        let mut r = VideoRecord {
            version: RECORD_VERSION,
            key: key_of(sha),
            name: "checkout crash.mp4".into(),
            sha256: sha.into(),
            size: 1234,
            path: Some("C:/rec/checkout crash.mp4".into()),
            included: false,
            duration_ms: 60_000,
            width: 1920,
            height: 1080,
            codec: "h264".into(),
            fps: Some(30.0),
            creation_ms: Some(1_760_000_000_000),
            file_modified_ms: None,
            options: SelectOptions::default(),
            user_alignment: UserAlignment::Auto,
            clock: None,
            alignment: crate::align::align(&AlignInput::default()),
            selection_notes: vec![],
            frames: (0..frames)
                .map(|i| FrameRecord {
                    video_ms: i as u64 * 1000,
                    image: format!("frames/{i:04}.png"),
                    width: 64,
                    height: 48,
                    score: 0.2,
                    description: (i == 0).then(|| "A login dialog.".into()),
                    note: None,
                    manual: false,
                })
                .collect(),
            analysed_ms: 1,
        };
        r.realign("UTC");
        r
    }

    #[test]
    fn records_round_trip_and_feed_the_frames_source() {
        let dir = tempfile::tempdir().unwrap();
        let folder = VideoFolder::of_project(dir.path());
        let r = record(&"a1".repeat(32), 3);
        assert_eq!((r.alignment.by, r.alignment.confidence), (AlignedBy::Container, Confidence::Medium));
        let source = folder.save(&r).unwrap();
        assert_eq!(source.file_name().unwrap(), "checkout crash.mp4.frames");
        let listed = folder.list();
        assert_eq!(listed.videos.len(), 1);
        assert_eq!(listed.videos[0], r);
        let text = std::fs::read_to_string(&source).unwrap();
        assert!(text.starts_with("{\"videoFrames\":1"));
        assert_eq!(text.lines().count(), 4);
        assert!(text.contains("\"time\":1760000001000"), "{text}");
        assert_eq!(
            folder.frame_path(&r, 2).unwrap(),
            dir.path().join("videos").join(&r.key).join("frames/0002.png")
        );
        assert!(folder.frame_path(&r, 3).is_err());
    }

    #[test]
    fn saving_removes_what_the_record_no_longer_names() {
        let dir = tempfile::tempdir().unwrap();
        let folder = VideoFolder::of_project(dir.path());
        let mut r = record(&"b2".repeat(32), 3);
        folder.save(&r).unwrap();
        let frames = folder.dir(&r.key).unwrap().join(FRAMES_DIR);
        for i in 0..3 {
            std::fs::write(frames.join(format!("{i:04}.png")), b"png").unwrap();
        }
        r.frames.remove(1);
        r.name = "renamed.mp4".into();
        folder.save(&r).unwrap();
        let d = folder.dir(&r.key).unwrap();
        assert!(!frames.join("0001.png").exists() && frames.join("0002.png").exists());
        assert!(!d.join("checkout crash.mp4.frames").exists() && d.join("renamed.mp4.frames").exists());
    }

    #[test]
    fn damaged_newer_and_foreign_records_are_skipped_not_fatal() {
        let dir = tempfile::tempdir().unwrap();
        let folder = VideoFolder::of_project(dir.path());
        let good = record(&"c3".repeat(32), 1);
        folder.save(&good).unwrap();
        let put = |key: &str, text: &str| {
            let d = folder.root().join(key);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(RECORD_FILE), text).unwrap();
        };
        put("0000000000000001", "{ damaged");
        let mut newer = serde_json::to_value(record(&"d4".repeat(32), 1)).unwrap();
        newer["version"] = 2.into();
        put("d4d4d4d4d4d4d4d4", &newer.to_string());
        let foreign = record(&"e5".repeat(32), 1);
        put("e6e6e6e6e6e6e6e6", &serde_json::to_string(&foreign).unwrap());
        let mut escape = serde_json::to_value(record(&"f6".repeat(32), 1)).unwrap();
        escape["frames"][0]["image"] = "../../secret.png".into();
        put("f6f6f6f6f6f6f6f6", &escape.to_string());
        std::fs::create_dir_all(folder.root().join("not-a-key")).unwrap();
        let listed = folder.list();
        assert_eq!(listed.videos, [good]);
        assert_eq!(listed.skipped.len(), 4, "{:?}", listed.skipped);
        assert!(listed.skipped.iter().any(|(_, why)| why.contains("version 2")));
        assert!(listed.skipped.iter().any(|(_, why)| why.contains("another video")));
        assert!(listed.skipped.iter().any(|(_, why)| why.contains("outside")));
        assert!(VideoFolder::of_project(&dir.path().join("none")).list().videos.is_empty());
        assert!(folder.load("../x").is_err());
    }

    #[test]
    fn the_project_keeps_a_bounded_number_of_frames() {
        let dir = tempfile::tempdir().unwrap();
        let folder = VideoFolder::of_project(dir.path());
        folder.save(&record(&"a1".repeat(32), 100)).unwrap();
        let mut big = record(&"b2".repeat(32), 1);
        big.frames = (0..=(MAX_PROJECT_FRAMES - 100)).map(|_| big.frames[0].clone()).collect();
        let e = folder.save(&big).unwrap_err();
        assert!(matches!(e, VideoError::Limit(_)) && e.to_string().contains("keep 100"), "{e}");
        let mut wrong = record(&"c3".repeat(32), 1);
        wrong.key = "c3c3c3c3c3c3c3c4".into();
        assert!(folder.save(&wrong).is_err());
    }

    #[test]
    fn the_video_can_be_included_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let folder = VideoFolder::of_project(dir.path());
        let r = record(&"a1".repeat(32), 1);
        folder.save(&r).unwrap();
        assert_eq!(folder.media_path(&r).unwrap(), None);
        let src = dir.path().join("in.mp4");
        std::fs::write(&src, b"video bytes").unwrap();
        let copy = folder.include(&r, &src).unwrap();
        assert_eq!(std::fs::read(&copy).unwrap(), b"video bytes");
        assert_eq!(folder.media_path(&r).unwrap(), Some(copy));
        folder.exclude(&r).unwrap();
        assert_eq!(folder.media_path(&r).unwrap(), None);
        assert!(folder.include(&r, &dir.path().join("missing.mp4")).is_err());
        folder.remove(&r.key).unwrap();
        assert!(folder.list().videos.is_empty());
        folder.remove(&r.key).unwrap();
    }

    #[test]
    fn names_texts_and_keys() {
        assert_eq!(safe_file_name("a/b\\c:d*.mp4"), "a_b_c_d_.mp4");
        assert_eq!(safe_file_name("..."), "video");
        assert_eq!(safe_file_name(&"x".repeat(300)).len(), 100);
        assert_eq!(clean_text("  \u{7}hi\n there "), Some("hi\n there".into()));
        assert_eq!(clean_text(" \t "), None);
        assert!(is_key("0123456789abcdef") && !is_key("0123456789ABCDEF") && !is_key("01"));
        assert!(is_frame_image("frames/0001.png") && is_frame_image("frames/12.jpg"));
        for bad in [
            "frames/../x.png",
            "x.png",
            "frames/a.png",
            "frames/0001.gif",
            "frames/.png",
            "frames/1234567.png",
        ] {
            assert!(!is_frame_image(bad), "{bad}");
        }
    }
}
