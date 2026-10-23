//! Taking a video into the project (ADR 0033 §3-4): hash it, probe it, find the candidate frames, pick the kept ones,
//! extract them, keep a clean copy of each as a frame image and save the record with its frames source. Adding one
//! frame at a time (the user's) goes through [`add_frame`]. Everything FFmpeg does goes through [`Pass`], so it is
//! sandboxed, capped and cancellable; the shell only calls these functions inside a job.

use crate::align::UserAlignment;
use crate::analyze::{Extracted, Pass, MAX_DURATION_MS};
use crate::describe::FRAME_PIXELS;
use crate::error::{Result, VideoError};
use crate::parse::VideoInfo;
use crate::record::{key_of, FrameRecord, VideoFolder, VideoRecord, FRAMES_DIR, MAX_PROJECT_FRAMES, RECORD_VERSION};
use crate::run::Tools;
use crate::select::{select, SelectOptions, MAX_CAP};
use debrief_ai::llm::ImageMime;
use debrief_ai::vision::{sanitize, ImageLimits};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// How near (ms) a re-analysed frame must be to an earlier one to keep its note and description.
const CARRY_MS: u64 = 500;
/// Where in the whole job each stage ends (0–1).
const HASH_END: f64 = 0.05;
const PROBE_END: f64 = 0.08;
const CANDIDATES_END: f64 = 0.6;
const EXTRACT_END: f64 = 0.95;

/// What a video import is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    Hashing,
    Probing,
    Analysing,
    Extracting,
    Saving,
}

/// Everything an import needs.
pub struct Ingest<'a> {
    pub tools: &'a Tools,
    pub folder: &'a VideoFolder,
    /// The video file (canonical, chosen by the user).
    pub input: &'a Path,
    pub options: SelectOptions,
    /// Keep a copy of the video in the project.
    pub include: bool,
    /// The profile's zone for times shown without one.
    pub tz: &'a str,
    /// A fresh folder FFmpeg may write; the caller removes it.
    pub work: &'a Path,
    pub memory_bytes: u64,
    pub cancel: &'a AtomicBool,
}

fn cancelled(cancel: &AtomicBool) -> Result<()> {
    if cancel.load(Ordering::SeqCst) {
        Err(VideoError::Cancelled)
    } else {
        Ok(())
    }
}

/// SHA-256 (lowercase hex) and size of a file, read in blocks; `progress` hears the share read.
pub fn hash_file(path: &Path, cancel: &AtomicBool, progress: &mut dyn FnMut(f64)) -> Result<(String, u64)> {
    let mut f = std::fs::File::open(path).map_err(|e| VideoError::Io(format!("cannot read {}: {e}", path.display())))?;
    let total = f.metadata()?.len();
    let mut h = Sha256::new();
    let mut buf = vec![0u8; 1 << 20];
    let mut done = 0u64;
    loop {
        cancelled(cancel)?;
        let n = f.read(&mut buf)?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
        done += n as u64;
        if total > 0 {
            progress(done as f64 / total as f64);
        }
    }
    let hex: String = h.finalize().iter().map(|b| format!("{b:02x}")).collect();
    Ok((hex, total))
}

fn modified_ms(path: &Path) -> Option<i64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(d.as_millis()).ok()
}

/// Writes the clean copy of an extracted frame into the video's folder; returns the record's entry.
fn keep_frame(dir: &Path, n: usize, e: &Extracted) -> Result<FrameRecord> {
    let bytes = std::fs::read(&e.path)?;
    let clean = sanitize(&bytes, FRAME_PIXELS, &ImageLimits::default())
        .map_err(|err| VideoError::Tool(format!("FFmpeg wrote a frame that cannot be used ({err})")))?
        .image;
    let ext = match clean.mime() {
        ImageMime::Png => "png",
        ImageMime::Jpeg => "jpg",
    };
    let rel = format!("{FRAMES_DIR}/{n:04}.{ext}");
    std::fs::create_dir_all(dir.join(FRAMES_DIR))?;
    let target = dir.join(&rel);
    let part = target.with_extension("part");
    std::fs::write(&part, clean.bytes())?;
    std::fs::rename(&part, &target)?;
    Ok(FrameRecord {
        video_ms: e.video_ms,
        image: rel,
        width: clean.width(),
        height: clean.height(),
        score: e.score,
        description: None,
        note: None,
        manual: false,
    })
}

/// Gives `fresh` the notes, descriptions and manual flags of the nearest earlier frame within [`CARRY_MS`].
fn carry_over(fresh: &mut [FrameRecord], earlier: &[FrameRecord]) {
    for f in fresh {
        if let Some(e) = earlier.iter().min_by_key(|e| e.video_ms.abs_diff(f.video_ms)) {
            if e.video_ms.abs_diff(f.video_ms) <= CARRY_MS {
                f.description = e.description.clone();
                f.note = e.note.clone();
            }
        }
    }
}

/// Analyses the video and saves it in the project, replacing an earlier analysis of the same video (the user's
/// alignment, the clock reading, the inclusion and the notes and descriptions of frames that are kept again carry
/// over; frames the user added are kept too). Returns the record.
pub fn analyse(i: &Ingest, on: &mut dyn FnMut(Stage, f64)) -> Result<VideoRecord> {
    let options = i.options.bounded();
    on(Stage::Hashing, 0.0);
    let (sha256, size) = hash_file(i.input, i.cancel, &mut |p| on(Stage::Hashing, p * HASH_END))?;
    let key = key_of(&sha256);
    let earlier = i.folder.load(&key).ok();
    on(Stage::Probing, HASH_END);
    let pass = Pass {
        tools: i.tools,
        input: i.input,
        work: i.work,
        memory_bytes: i.memory_bytes,
        cancel: i.cancel,
    };
    let info = pass.probe(false)?;
    on(Stage::Analysing, PROBE_END);
    let candidates = pass.candidates(&info, &mut |p| on(Stage::Analysing, PROBE_END + p * (CANDIDATES_END - PROBE_END)))?;
    let selection = select(candidates, options);
    cancelled(i.cancel)?;
    let manual: Vec<FrameRecord> = earlier.iter().flat_map(|r| r.frames.iter().filter(|f| f.manual).cloned()).collect();
    let others = i.folder.frames_besides(&key);
    if others + selection.kept.len() > MAX_PROJECT_FRAMES {
        return Err(VideoError::Limit(format!(
            "a project keeps at most {MAX_PROJECT_FRAMES} video frames and the other videos keep {others}: keep fewer frames of this one or remove a video"
        )));
    }
    on(Stage::Extracting, CANDIDATES_END);
    let extracted = pass.extract(&info, &selection.kept, &mut |p| {
        on(Stage::Extracting, CANDIDATES_END + p * (EXTRACT_END - CANDIDATES_END))
    })?;
    on(Stage::Saving, EXTRACT_END);
    let dir = i.folder.dir(&key)?;
    let mut frames = Vec::with_capacity(extracted.len());
    for (n, e) in extracted.iter().enumerate() {
        cancelled(i.cancel)?;
        frames.push(keep_frame(&dir, n + 1, e)?);
    }
    if let Some(r) = &earlier {
        carry_over(&mut frames, &r.frames);
    }
    // The user's own frames survive a re-analysis (re-extracted at their exact times when they differ from the new ones).
    for m in manual {
        if !frames.iter().any(|f| f.video_ms.abs_diff(m.video_ms) <= CARRY_MS) {
            if let Ok(e) = pass.extract_at(&info, m.video_ms) {
                let mut f = keep_frame(&dir, frames.len() + 1, &e)?;
                f.manual = true;
                f.description = m.description;
                f.note = m.note;
                frames.push(f);
            }
        }
    }
    frames.sort_by_key(|f| f.video_ms);
    renumber(&dir, &mut frames)?;
    let mut record = VideoRecord {
        version: RECORD_VERSION,
        key,
        name: i
            .input
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| "video".into()),
        sha256,
        size,
        path: Some(i.input.to_string_lossy().into_owned()),
        included: earlier.as_ref().is_some_and(|r| r.included) || i.include,
        duration_ms: info.duration_ms,
        width: info.width,
        height: info.height,
        codec: info.codec.clone(),
        fps: info.fps,
        creation_ms: info.creation_ms,
        file_modified_ms: modified_ms(i.input),
        options,
        user_alignment: earlier.as_ref().map_or(UserAlignment::Auto, |r| r.user_alignment),
        clock: earlier.as_ref().and_then(|r| r.clock.clone()),
        alignment: crate::align::align(&Default::default()),
        selection_notes: selection.notes,
        frames,
        analysed_ms: now_ms(),
    };
    record.realign(i.tz);
    i.folder.save(&record)?;
    if record.included {
        i.folder.include(&record, i.input)?;
    }
    on(Stage::Saving, 1.0);
    Ok(record)
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()
        .and_then(|d| i64::try_from(d.as_millis()).ok())
        .unwrap_or(0)
}

/// Names the frame images `0001`, `0002`... in time order (renaming through a temporary name, so none is overwritten).
pub fn renumber(dir: &Path, frames: &mut [FrameRecord]) -> Result<()> {
    let mut moves: Vec<(PathBuf, PathBuf)> = Vec::new();
    for (n, f) in frames.iter().enumerate() {
        let ext = Path::new(&f.image)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_string();
        let rel = format!("{FRAMES_DIR}/{:04}.{ext}", n + 1);
        if rel != f.image {
            moves.push((dir.join(&f.image), dir.join(&rel)));
        }
    }
    for (from, _) in &moves {
        let tmp = from.with_extension("moving");
        std::fs::rename(from, &tmp)?;
    }
    for (from, to) in &moves {
        std::fs::rename(from.with_extension("moving"), to)?;
    }
    for (n, f) in frames.iter_mut().enumerate() {
        let ext = Path::new(&f.image)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("png")
            .to_string();
        f.image = format!("{FRAMES_DIR}/{:04}.{ext}", n + 1);
    }
    Ok(())
}

/// The video file a record's frames can still be taken from: its included copy, else the file it was opened from
/// when it is still there with the same size. The flag says whether it is the included copy.
pub fn media_file(folder: &VideoFolder, r: &VideoRecord) -> Option<(PathBuf, bool)> {
    if let Ok(Some(p)) = folder.media_path(r) {
        return Some((p, true));
    }
    let p = PathBuf::from(r.path.as_deref()?);
    let meta = std::fs::metadata(&p).ok()?;
    (meta.is_file() && meta.len() == r.size).then_some((p, false))
}

/// Adds the frame at or after `ms` (the player's position) to a video, as the user's own frame.
pub fn add_frame(i: &Ingest, record: &mut VideoRecord, ms: u64) -> Result<usize> {
    if record.frames.len() >= MAX_CAP {
        return Err(VideoError::Limit(format!(
            "a video keeps at most {MAX_CAP} frames: remove one first"
        )));
    }
    if i.folder.frames_besides(&record.key) + record.frames.len() >= MAX_PROJECT_FRAMES {
        return Err(VideoError::Limit(format!(
            "a project keeps at most {MAX_PROJECT_FRAMES} video frames: remove one first"
        )));
    }
    let pass = Pass {
        tools: i.tools,
        input: i.input,
        work: i.work,
        memory_bytes: i.memory_bytes,
        cancel: i.cancel,
    };
    let info: VideoInfo = pass.probe(false)?;
    if info.duration_ms > MAX_DURATION_MS {
        return Err(VideoError::Limit("the video is too long".into()));
    }
    let e = pass.extract_at(&info, ms)?;
    if record.frames.iter().any(|f| f.video_ms == e.video_ms) {
        return Err(VideoError::Invalid("that moment is already one of the frames".into()));
    }
    let dir = i.folder.dir(&record.key)?;
    let mut f = keep_frame(&dir, record.frames.len() + 1_000, &e)?;
    f.manual = true;
    let at = record.frames.partition_point(|x| x.video_ms < f.video_ms);
    record.frames.insert(at, f);
    renumber(&dir, &mut record.frames)?;
    i.folder.save(record)?;
    Ok(at)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr(ms: u64, image: &str) -> FrameRecord {
        FrameRecord {
            video_ms: ms,
            image: image.into(),
            width: 1,
            height: 1,
            score: 0.0,
            description: None,
            note: None,
            manual: false,
        }
    }

    #[test]
    fn notes_carry_over_to_the_nearest_frame_within_half_a_second() {
        let mut old = vec![fr(1000, "frames/0001.png"), fr(9000, "frames/0002.png")];
        old[0].note = Some("login".into());
        old[1].description = Some("error".into());
        let mut fresh = vec![
            fr(1300, "frames/0001.png"),
            fr(5000, "frames/0002.png"),
            fr(9400, "frames/0003.png"),
        ];
        carry_over(&mut fresh, &old);
        assert_eq!(fresh[0].note.as_deref(), Some("login"));
        assert_eq!(fresh[1].note, None);
        assert_eq!(fresh[2].description.as_deref(), Some("error"));
        carry_over(&mut fresh, &[]);
    }

    #[test]
    fn hashing_is_the_sha256_and_can_be_cancelled() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("a.bin");
        std::fs::write(&p, b"abc").unwrap();
        let cancel = AtomicBool::new(false);
        let mut last = 0.0;
        let (h, size) = hash_file(&p, &cancel, &mut |x| last = x).unwrap();
        assert_eq!(h, "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
        assert_eq!((size, last), (3, 1.0));
        std::fs::write(&p, b"").unwrap();
        assert_eq!(hash_file(&p, &cancel, &mut |_| {}).unwrap().1, 0);
        cancel.store(true, Ordering::SeqCst);
        assert_eq!(hash_file(&p, &cancel, &mut |_| {}).unwrap_err(), VideoError::Cancelled);
        assert!(matches!(
            hash_file(&d.path().join("none"), &AtomicBool::new(false), &mut |_| {}),
            Err(VideoError::Io(_))
        ));
    }

    #[test]
    fn frames_are_numbered_in_time_order_without_losing_files() {
        let d = tempfile::tempdir().unwrap();
        std::fs::create_dir_all(d.path().join("frames")).unwrap();
        for (n, c) in [("0001.png", "a"), ("1001.png", "b"), ("0002.jpg", "c")] {
            std::fs::write(d.path().join("frames").join(n), c).unwrap();
        }
        // time order: a (0001), b (1001, inserted in the middle), c (0002.jpg)
        let mut frames = vec![fr(0, "frames/0001.png"), fr(500, "frames/1001.png"), fr(900, "frames/0002.jpg")];
        renumber(d.path(), &mut frames).unwrap();
        let names: Vec<_> = frames.iter().map(|f| f.image.as_str()).collect();
        assert_eq!(names, ["frames/0001.png", "frames/0002.png", "frames/0003.jpg"]);
        let read = |n: &str| std::fs::read_to_string(d.path().join("frames").join(n)).unwrap();
        assert_eq!(
            (read("0001.png"), read("0002.png"), read("0003.jpg")),
            ("a".into(), "b".into(), "c".into())
        );
        assert!(!d.path().join("frames/1001.png").exists());
    }
}
