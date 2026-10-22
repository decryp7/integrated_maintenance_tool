//! Taking a video into the project against the fake ffmpeg: the record, its frames and source, re-analysis keeping the
//! user's work, adding a frame, the project cap, cancel, and what is left behind on failure.

use imt_video::ingest::{add_frame, analyse, media_file, Ingest, Stage};
use imt_video::record::{VideoFolder, MAX_PROJECT_FRAMES};
use imt_video::run::Tools;
use imt_video::select::SelectOptions;
use imt_video::VideoError;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

const FAKE: &str = env!("CARGO_BIN_EXE_imt-video-fake-ffmpeg");

fn serial() -> std::sync::MutexGuard<'static, ()> {
    static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
    LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

struct Env {
    _dir: tempfile::TempDir,
    root: PathBuf,
    tools: Tools,
    folder: VideoFolder,
    video: PathBuf,
    work: PathBuf,
}

fn env(frames: &[(u64, f32)]) -> Env {
    let dir = tempfile::tempdir().unwrap();
    let root = std::fs::canonicalize(dir.path()).unwrap();
    let rt = root.join("runtime");
    std::fs::create_dir_all(&rt).unwrap();
    let tools = Tools::in_dir(&rt);
    for exe in [&tools.ffmpeg, &tools.ffprobe] {
        std::fs::copy(FAKE, exe).unwrap();
    }
    let video = root.join("checkout crash 2026-10-14 10-40-00.mp4");
    let mut text = String::from("FAKEVIDEO duration_ms=60000\n");
    for (ms, s) in frames {
        text.push_str(&format!("{ms} {s}\n"));
    }
    std::fs::write(&video, text).unwrap();
    let work = root.join("work");
    std::fs::create_dir_all(&work).unwrap();
    let folder = VideoFolder::of_project(&root.join("project"));
    Env {
        _dir: dir,
        root,
        tools,
        folder,
        video,
        work,
    }
}

fn ingest<'a>(e: &'a Env, cancel: &'a AtomicBool, options: SelectOptions) -> Ingest<'a> {
    Ingest {
        tools: &e.tools,
        folder: &e.folder,
        input: &e.video,
        options,
        include: false,
        tz: "UTC",
        work: &e.work,
        memory_bytes: 512 << 20,
        cancel,
    }
}

const SCENES: [(u64, f32); 6] = [(0, 0.0), (4000, 0.4), (9000, 0.5), (30_000, 0.7), (45_000, 0.3), (59_000, 0.2)];

#[test]
fn a_video_becomes_a_record_with_frames_and_a_source() {
    let _s = serial();
    let e = env(&SCENES);
    let cancel = AtomicBool::new(false);
    let mut stages = Vec::new();
    let r = analyse(&ingest(&e, &cancel, SelectOptions::default()), &mut |s, p| stages.push((s, p))).unwrap();
    assert_eq!(r.frames.len(), 6);
    assert_eq!(r.frames[0].video_ms, 0);
    assert!(r
        .frames
        .iter()
        .enumerate()
        .all(|(i, f)| f.image == format!("frames/{:04}.png", i + 1) && !f.manual));
    assert_eq!((r.duration_ms, r.width, r.height), (60_000, 1920, 1080));
    assert_eq!(
        r.alignment.by,
        imt_core::importers::video::AlignedBy::FileName,
        "the recorder names its files by start"
    );
    assert!(stages.windows(2).all(|w| w[0].1 <= w[1].1), "progress never goes back: {stages:?}");
    assert_eq!(stages.last().unwrap(), &(Stage::Saving, 1.0));
    // On disk: record, source, images.
    let dir = e.folder.dir(&r.key).unwrap();
    assert!(dir.join("video.json").is_file() && dir.join(r.source_file_name()).is_file());
    assert_eq!(std::fs::read_dir(dir.join("frames")).unwrap().count(), 6);
    assert_eq!(e.folder.load(&r.key).unwrap(), r);
    assert_eq!(media_file(&e.folder, &r), Some((e.video.clone(), false)));
    let _ = Path::new("");
}

#[test]
fn analysing_again_keeps_the_users_work() {
    let _s = serial();
    let e = env(&SCENES);
    let cancel = AtomicBool::new(false);
    let mut r = analyse(&ingest(&e, &cancel, SelectOptions::default()), &mut |_, _| {}).unwrap();
    r.frames[1].note = Some("the dialog opens".into());
    r.frames[2].description = Some("An error dialog".into());
    r.user_alignment = imt_video::align::UserAlignment::Offset { offset_ms: 2000 };
    e.folder.save(&r).unwrap();
    let opts = SelectOptions {
        cap: 3,
        ..SelectOptions::default()
    };
    let again = analyse(&ingest(&e, &cancel, opts), &mut |_, _| {}).unwrap();
    assert_eq!(again.key, r.key);
    assert_eq!(again.frames.len(), 3);
    assert_eq!(again.user_alignment, r.user_alignment);
    assert_eq!(
        std::fs::read_dir(e.folder.dir(&r.key).unwrap().join("frames")).unwrap().count(),
        3,
        "old images are removed"
    );
}

#[test]
fn the_users_own_frame_is_added_in_time_order_and_survives() {
    let _s = serial();
    let e = env(&SCENES);
    let cancel = AtomicBool::new(false);
    let mut r = analyse(
        &ingest(
            &e,
            &cancel,
            SelectOptions {
                cap: 3,
                ..SelectOptions::default()
            },
        ),
        &mut |_, _| {},
    )
    .unwrap();
    let kept: Vec<u64> = r.frames.iter().map(|f| f.video_ms).collect();
    let extra = SCENES.iter().map(|s| s.0).find(|ms| !kept.contains(ms)).unwrap();
    let at = add_frame(&ingest(&e, &cancel, SelectOptions::default()), &mut r, extra).unwrap();
    assert!(r.frames[at].manual && r.frames[at].video_ms == extra);
    assert!(r.frames.windows(2).all(|w| w[0].video_ms < w[1].video_ms));
    assert!(r
        .frames
        .iter()
        .enumerate()
        .all(|(i, f)| f.image.starts_with(&format!("frames/{:04}.", i + 1))));
    assert!(matches!(
        add_frame(&ingest(&e, &cancel, SelectOptions::default()), &mut r, extra),
        Err(VideoError::Invalid(_))
    ));
    // A re-analysis keeps it.
    let again = analyse(
        &ingest(
            &e,
            &cancel,
            SelectOptions {
                cap: 3,
                ..SelectOptions::default()
            },
        ),
        &mut |_, _| {},
    )
    .unwrap();
    assert!(again.frames.iter().any(|f| f.manual && f.video_ms == extra));
}

#[test]
fn the_project_cap_and_cancel_leave_nothing_half_done() {
    let _s = serial();
    let e = env(&SCENES);
    let cancel = AtomicBool::new(false);
    // Another video of the project keeps all but two of the frames the project may hold.
    let first = analyse(
        &ingest(
            &e,
            &cancel,
            SelectOptions {
                cap: 2,
                ..SelectOptions::default()
            },
        ),
        &mut |_, _| {},
    )
    .unwrap();
    let mut other = first.clone();
    other.sha256 = "ab".repeat(32);
    other.key = imt_video::record::key_of(&other.sha256);
    other.frames = (0..MAX_PROJECT_FRAMES - 2)
        .map(|i| imt_video::record::FrameRecord {
            image: format!("frames/{:04}.png", i + 1),
            video_ms: i as u64,
            ..first.frames[0].clone()
        })
        .collect();
    e.folder.save(&other).unwrap();
    std::fs::write(&e.video, "FAKEVIDEO duration_ms=60000\n0 0\n4000 0.4\n9000 0.5\n").unwrap();
    let err = analyse(&ingest(&e, &cancel, SelectOptions::default()), &mut |_, _| {}).unwrap_err();
    assert!(matches!(&err, VideoError::Limit(m) if m.contains("remove a video")), "{err}");
    // Cancel before anything.
    let e2 = env(&SCENES);
    let cancelled = AtomicBool::new(true);
    assert_eq!(
        analyse(&ingest(&e2, &cancelled, SelectOptions::default()), &mut |_, _| {}).unwrap_err(),
        VideoError::Cancelled
    );
    assert!(!e2.root.join("project").join("videos").exists() || e2.folder.list().videos.is_empty());
    cancelled.store(false, Ordering::SeqCst);
}

#[test]
fn a_video_ffmpeg_cannot_read_is_a_clear_error() {
    let _s = serial();
    let e = env(&SCENES);
    std::fs::write(&e.video, b"not a video").unwrap();
    let cancel = AtomicBool::new(false);
    let err = analyse(&ingest(&e, &cancel, SelectOptions::default()), &mut |_, _| {}).unwrap_err();
    assert!(matches!(err, VideoError::Tool(_)), "{err}");
    assert!(e.folder.list().videos.is_empty());
}

#[test]
fn the_included_copy_is_what_plays_when_the_original_is_gone() {
    let _s = serial();
    let e = env(&SCENES);
    let cancel = AtomicBool::new(false);
    let mut i = ingest(&e, &cancel, SelectOptions::default());
    i.include = true;
    let r = analyse(&i, &mut |_, _| {}).unwrap();
    assert!(r.included);
    let (p, included) = media_file(&e.folder, &r).unwrap();
    assert!(included && p.starts_with(e.folder.dir(&r.key).unwrap()));
    std::fs::remove_file(&e.video).unwrap();
    assert!(media_file(&e.folder, &r).unwrap().1);
    e.folder.exclude(&r).unwrap();
    assert_eq!(media_file(&e.folder, &r), None, "gone and not included");
}
