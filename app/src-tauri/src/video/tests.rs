//! Video through its commands (ADR 0033): the project's videos listed, frames, notes and alignment changing the
//! records and replacing the frames source in the workspace, removal, the optional copy, the runtime commands and the
//! `media:` scheme (ranges, only the calling window's videos). A real end-to-end import runs when an `ffmpeg` is on
//! the PATH (a clip generated with its `testsrc`, never user footage).

use super::commands::*;
use super::media::serve_for_tests;
use super::runtime_commands::*;
use super::*;
use crate::test_support::{block, main_window, samples, test_app, window, TempDir};
use crate::windows::CurrentWindow;
use imt_video::align::UserAlignment;
use imt_video::api::*;
use imt_video::record::{key_of, FrameRecord, VideoFolder, VideoRecord, RECORD_VERSION};
use imt_video::select::SelectOptions;
use tauri::http::{header, Request, StatusCode};
use tauri::test::MockRuntime;

const SHA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

fn png(n: u8) -> Vec<u8> {
    crate::activity::report::sample::png(32, 24, move |x, _| [x as u8 * 6, n * 40, 90])
}

/// A test app whose main window has a store, and a video of three frames saved in its project folder.
fn app_with_video(tag: &str) -> (TempDir, tauri::App<MockRuntime>, VideoRecord, std::path::PathBuf) {
    let dir = TempDir::new(tag);
    let app = test_app(Some(dir.0.join("profiles")));
    app.manage(VideoState::new(dir.0.join("video")));
    crate::paths::allow_under(app.handle(), "main", &dir.0);
    let win = main_window(&app);
    let folder = folder_of(&win).unwrap();
    let file = dir.0.join("checkout crash 2026-10-14 10-40-00.mp4");
    let mut head = b"\0\0\0\x18ftypmp42".to_vec();
    head.resize(4096, 7);
    std::fs::write(&file, &head).unwrap();
    let mut r = VideoRecord {
        version: RECORD_VERSION,
        key: key_of(SHA),
        name: "checkout crash 2026-10-14 10-40-00.mp4".into(),
        sha256: SHA.into(),
        size: head.len() as u64,
        path: Some(file.to_string_lossy().into_owned()),
        included: false,
        duration_ms: 60_000,
        width: 1920,
        height: 1080,
        codec: "h264".into(),
        fps: Some(30.0),
        creation_ms: None,
        file_modified_ms: None,
        options: SelectOptions::default(),
        user_alignment: UserAlignment::Auto,
        clock: None,
        alignment: imt_video::align::align(&Default::default()),
        selection_notes: vec![],
        frames: (0..3)
            .map(|i| FrameRecord {
                video_ms: i as u64 * 20_000,
                image: format!("frames/{:04}.png", i + 1),
                width: 32,
                height: 24,
                score: 0.3,
                description: (i == 1).then(|| "An error dialog".to_string()),
                note: None,
                manual: false,
            })
            .collect(),
        analysed_ms: 0,
    };
    r.realign("UTC");
    let vdir = folder.dir(&r.key).unwrap();
    std::fs::create_dir_all(vdir.join("frames")).unwrap();
    for i in 0..3u8 {
        std::fs::write(vdir.join(format!("frames/{:04}.png", i + 1)), png(i)).unwrap();
    }
    folder.save(&r).unwrap();
    (dir, app, r, file)
}

fn code<T: std::fmt::Debug>(r: crate::CmdResult<T>) -> crate::ErrorCode {
    r.unwrap_err().code
}

fn frames_source(win: &CurrentWindow) -> Option<imt_core::api::SourceInfo> {
    crate::test_support::store_summary(win)
        .sources
        .into_iter()
        .find(|s| s.name.ends_with(".frames"))
}

#[test]
fn engine_errors_become_typed_command_errors() {
    use crate::ErrorCode as C;
    use imt_video::{AiUnavailable, VideoError as V};
    let cases = [
        (V::Invalid("x".into()), C::Invalid),
        (V::NotFound("x".into()), C::NotFound),
        (V::NoRuntime("x".into()), C::Missing),
        (V::Integrity("x".into()), C::Integrity),
        (
            V::SelfTest {
                step: "s".into(),
                message: "m".into(),
            },
            C::SelfTestFailed,
        ),
        (V::Tool("x".into()), C::Failed),
        (V::Timeout("x".into()), C::Timeout),
        (V::OverMemory("x".into()), C::Memory),
        (V::Limit("x".into()), C::Limit),
        (V::Cancelled, C::Cancelled),
        (V::Io("x".into()), C::Io),
        (
            V::AiUnavailable {
                kind: AiUnavailable::NoVision,
                message: "x".into(),
            },
            C::NoVision,
        ),
        (
            V::AiUnavailable {
                kind: AiUnavailable::NoModel,
                message: "x".into(),
            },
            C::NoModel,
        ),
        (
            V::AiUnavailable {
                kind: AiUnavailable::NoRuntime,
                message: "x".into(),
            },
            C::NoRuntime,
        ),
        (V::Ai("x".into()), C::Engine),
    ];
    for (e, want) in cases {
        let got = cmd(e);
        assert_eq!(got.code, want);
    }
    assert!(cmd(V::NoRuntime("x".into())).hint.is_some_and(|h| h.contains("Settings")));
}

#[test]
fn the_csp_allows_the_media_scheme_and_nothing_remote() {
    let conf: serde_json::Value = serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
    let csp = conf["app"]["security"]["csp"].as_str().unwrap();
    let media = csp
        .split(';')
        .map(str::trim)
        .find(|d| d.starts_with("media-src"))
        .expect("media-src");
    assert_eq!(media, "media-src 'self' media: http://media.localhost");
    assert!(!csp.contains("unsafe-eval") && !csp.contains("https:"));
}

#[test]
fn the_projects_videos_are_listed_with_where_they_play_from() {
    let (_dir, app, r, file) = app_with_video("video-list");
    let win = main_window(&app);
    let list = block(video_list(win.clone())).unwrap();
    assert_eq!((list.videos.len(), list.project_frames, list.max_project_frames), (1, 3, 600));
    let v = &list.videos[0];
    assert_eq!(v.media, MediaState::Referenced);
    assert_eq!(v.media_url.as_deref(), Some(format!("{MEDIA_BASE}{}", r.key).as_str()));
    assert_eq!(v.source_id, None, "not in the workspace until something publishes it");
    // The file moved: frames and notes still work, the player has nothing.
    std::fs::remove_file(&file).unwrap();
    let v = &block(video_list(win)).unwrap().videos[0];
    assert_eq!((v.media, v.media_url.clone()), (MediaState::Missing, None));
}

#[test]
fn frames_images_are_read_clean_and_bad_references_are_refused() {
    let (_dir, app, r, _f) = app_with_video("video-frames");
    let win = main_window(&app);
    let ok = block(video_frame_image(
        win.clone(),
        FrameRef {
            key: r.key.clone(),
            index: 2,
        },
    ))
    .unwrap();
    assert!(ok.data_url.starts_with("data:image/png;base64,"));
    assert_eq!(
        code(block(video_frame_image(
            win.clone(),
            FrameRef {
                key: r.key.clone(),
                index: 3
            }
        ))),
        crate::ErrorCode::NotFound
    );
    assert_eq!(
        code(block(video_frame_image(
            win.clone(),
            FrameRef {
                key: "../../etc".into(),
                index: 0
            }
        ))),
        crate::ErrorCode::NotFound
    );
    assert_eq!(
        code(block(video_frame_image(
            win,
            FrameRef {
                key: "0000000000000000".into(),
                index: 0
            }
        ))),
        crate::ErrorCode::NotFound
    );
}

#[test]
fn a_note_or_alignment_rewrites_the_frames_source_in_the_workspace() {
    let (_dir, app, r, _f) = app_with_video("video-source");
    let win = main_window(&app);
    let v = block(video_set_note(
        app.handle().clone(),
        win.clone(),
        NoteRequest {
            key: r.key.clone(),
            index: 1,
            note: Some("  the dialog opens\nhere ".into()),
        },
    ))
    .unwrap();
    assert_eq!(v.record.frames[1].note.as_deref(), Some("the dialog opens here"));
    let source = frames_source(&win).expect("the frames are a source");
    assert_eq!((source.records, Some(source.id)), (3, v.source_id));
    // The alignment moves every event.
    let start = 1_760_438_400_000i64;
    let v = block(video_set_alignment(
        app.handle().clone(),
        win.clone(),
        AlignRequest {
            key: r.key.clone(),
            alignment: UserAlignment::Start { start_ms: start },
        },
    ))
    .unwrap();
    assert_eq!(v.record.alignment.start_ms, start);
    let source = frames_source(&win).unwrap();
    assert_eq!((source.records, source.from, source.to), (3, Some(start), Some(start + 40_000)));
    // Still one source after changes: it is replaced, not added.
    assert_eq!(
        crate::test_support::store_summary(&win)
            .sources
            .iter()
            .filter(|s| s.name.ends_with(".frames"))
            .count(),
        1
    );
    // A note can be removed; bad frames are refused.
    let v = block(video_set_note(
        app.handle().clone(),
        win.clone(),
        NoteRequest {
            key: r.key.clone(),
            index: 1,
            note: None,
        },
    ))
    .unwrap();
    assert_eq!(v.record.frames[1].note, None);
    assert_eq!(
        code(block(video_set_note(
            app.handle().clone(),
            win,
            NoteRequest {
                key: r.key.clone(),
                index: 9,
                note: Some("x".into())
            }
        ))),
        crate::ErrorCode::NotFound
    );
}

#[test]
fn frames_can_be_removed_but_never_the_last_one() {
    let (_dir, app, r, _f) = app_with_video("video-remove-frame");
    let win = main_window(&app);
    let h = app.handle().clone();
    let v = block(video_remove_frame(
        h.clone(),
        win.clone(),
        FrameRef {
            key: r.key.clone(),
            index: 0,
        },
    ))
    .unwrap();
    assert_eq!(v.record.frames.len(), 2);
    assert_eq!(v.record.frames[0].video_ms, 20_000);
    assert!(v
        .record
        .frames
        .iter()
        .enumerate()
        .all(|(i, f)| f.image == format!("frames/{:04}.png", i + 1)));
    let folder = folder_of(&win).unwrap();
    assert!(
        folder.frame_path(&v.record, 1).unwrap().is_file() && !folder.frame_path(&v.record, 1).unwrap().with_file_name("0003.png").exists()
    );
    block(video_remove_frame(
        h.clone(),
        win.clone(),
        FrameRef {
            key: r.key.clone(),
            index: 0,
        },
    ))
    .unwrap();
    assert_eq!(
        code(block(video_remove_frame(h, win, FrameRef { key: r.key, index: 0 }))),
        crate::ErrorCode::Invalid
    );
}

#[test]
fn the_video_can_be_copied_into_the_project_and_the_copy_dropped() {
    let (_dir, app, r, file) = app_with_video("video-include");
    let win = main_window(&app);
    let h = app.handle().clone();
    let v = block(video_include(
        h.clone(),
        win.clone(),
        IncludeRequest {
            key: r.key.clone(),
            include: true,
        },
    ))
    .unwrap();
    assert_eq!(v.media, MediaState::Included);
    assert!(v.record.included);
    std::fs::remove_file(&file).unwrap();
    assert_eq!(
        block(video_list(win.clone())).unwrap().videos[0].media,
        MediaState::Included,
        "the copy plays when the original is gone"
    );
    let v = block(video_include(
        h.clone(),
        win.clone(),
        IncludeRequest {
            key: r.key.clone(),
            include: false,
        },
    ))
    .unwrap();
    assert_eq!(v.media, MediaState::Missing);
    assert_eq!(
        code(block(video_include(h, win, IncludeRequest { key: r.key, include: true }))),
        crate::ErrorCode::NotFound
    );
}

#[test]
fn removing_a_video_removes_its_source_frames_and_copy_but_not_the_original() {
    let (_dir, app, r, file) = app_with_video("video-remove");
    let win = main_window(&app);
    let h = app.handle().clone();
    block(video_include(
        h.clone(),
        win.clone(),
        IncludeRequest {
            key: r.key.clone(),
            include: true,
        },
    ))
    .unwrap();
    block(video_set_note(
        h.clone(),
        win.clone(),
        NoteRequest {
            key: r.key.clone(),
            index: 0,
            note: Some("x".into()),
        },
    ))
    .unwrap();
    assert!(frames_source(&win).is_some());
    let folder = folder_of(&win).unwrap();
    let dir = folder.dir(&r.key).unwrap();
    block(video_remove(h.clone(), win.clone(), r.key.clone())).unwrap();
    assert!(!dir.exists() && frames_source(&win).is_none());
    assert!(file.is_file(), "the user's file is never touched");
    assert!(block(video_list(win.clone())).unwrap().videos.is_empty());
    assert_eq!(code(block(video_remove(h, win, "not a key".into()))), crate::ErrorCode::NotFound);
}

#[test]
fn opening_refuses_what_it_cannot_analyse_with_what_to_do() {
    let (dir, app, _r, _f) = app_with_video("video-open-refused");
    let win = main_window(&app);
    let h = app.handle().clone();
    let grants = || crate::paths::WindowGrants::of(app.handle(), "main");
    let req = |p: &std::path::Path| OpenVideoRequest {
        path: p.to_string_lossy().into_owned(),
        options: SelectOptions::default(),
        include: false,
    };
    let text = dir.0.join("notes.txt");
    std::fs::write(&text, "x").unwrap();
    let e = block(video_open(h.clone(), win.clone(), grants(), req(&text))).unwrap_err();
    assert_eq!(e.code, crate::ErrorCode::Unsupported);
    // A video, but no FFmpeg is set up: Missing, with the Settings hint.
    let clip = dir.0.join("clip.mp4");
    std::fs::write(&clip, b"x").unwrap();
    let e = block(video_open(h, win, grants(), req(&clip))).unwrap_err();
    assert_eq!(e.code, crate::ErrorCode::Missing);
    assert!(e.hint.unwrap().contains("Settings"));
}

#[test]
fn sniffing_picks_videos_by_name_and_first_bytes() {
    let (dir, app, _r, file) = app_with_video("video-sniff");
    let grants = crate::paths::WindowGrants::of(app.handle(), "main");
    let fake = dir.0.join("log.mp4");
    std::fs::write(&fake, "2026-10-14 10:41:14 INFO started").unwrap();
    let text = dir.0.join("a.log");
    std::fs::write(&text, "x").unwrap();
    let paths = [file.clone(), fake, text].map(|p| p.to_string_lossy().into_owned()).to_vec();
    assert_eq!(
        block(video_sniff(crate::paths::WindowGrants::of(app.handle(), "main"), paths)).unwrap(),
        vec![file.to_string_lossy().into_owned()]
    );
    let many = vec!["x".to_string(); 1_001];
    assert_eq!(code(block(video_sniff(grants, many))), crate::ErrorCode::Limit);
}

#[test]
fn the_runtime_commands_report_validate_and_refuse_what_cannot_work() {
    let dir = TempDir::new("video-runtime");
    let app = test_app(None);
    app.manage(VideoState::new(dir.0.join("video")));
    let h = app.handle().clone();
    let v = block(video_runtime(h.clone())).unwrap();
    assert_eq!(v.status.mode, imt_video::runtime::RuntimeMode::None);
    assert!(!v.platform.is_empty());
    assert_eq!(
        v.can_download,
        !cfg!(target_os = "macos") && cfg!(any(windows, target_os = "linux"))
    );
    if !v.can_download {
        assert_eq!(code(block(video_runtime_install(h.clone()))), crate::ErrorCode::Unsupported);
    }
    assert_eq!(
        code(block(video_runtime_set_custom(h.clone(), "  ".into()))),
        crate::ErrorCode::Invalid
    );
    assert_eq!(
        code(block(video_runtime_set_custom(h.clone(), "x".repeat(5000)))),
        crate::ErrorCode::Invalid
    );
    assert_eq!(
        code(block(video_runtime_set_custom(
            h.clone(),
            dir.0.join("none").to_string_lossy().into_owned()
        ))),
        crate::ErrorCode::NotFound
    );
    assert_eq!(code(block(video_runtime_delete(h.clone(), "".into()))), crate::ErrorCode::Invalid);
    assert_eq!(
        code(block(video_runtime_delete(h.clone(), "autobuild-1".into()))),
        crate::ErrorCode::NotFound
    );
    assert_eq!(code(block(video_runtime_rollback(h.clone()))), crate::ErrorCode::NotFound);
    assert_eq!(
        block(video_runtime_clear_custom(h)).unwrap().status.mode,
        imt_video::runtime::RuntimeMode::None
    );
}

fn get(uri: &str, range: Option<&str>) -> Request<Vec<u8>> {
    let mut b = Request::builder().method("GET").uri(uri);
    if let Some(r) = range {
        b = b.header(header::RANGE, r);
    }
    b.body(Vec::new()).unwrap()
}

#[test]
fn the_player_gets_ranges_of_its_own_windows_videos_only() {
    let (_dir, app, r, file) = app_with_video("video-media");
    let handle = app.handle();
    let uri = format!("{MEDIA_BASE}{}", r.key);
    let len = std::fs::metadata(&file).unwrap().len();
    let resp = serve_for_tests(handle, "main", &get(&uri, Some("bytes=0-99")));
    assert_eq!(resp.status(), StatusCode::PARTIAL_CONTENT);
    assert_eq!(resp.body().len(), 100);
    assert_eq!(resp.headers()[header::CONTENT_RANGE], format!("bytes 0-99/{len}").as_str());
    assert_eq!(resp.headers()[header::CONTENT_TYPE], "video/mp4");
    assert_eq!(resp.headers()["X-Content-Type-Options"], "nosniff");
    let tail = serve_for_tests(handle, "main", &get(&uri, Some("bytes=-16")));
    assert_eq!(tail.body().len(), 16);
    let whole = serve_for_tests(handle, "main", &get(&uri, None));
    assert_eq!((whole.status(), whole.body().len() as u64), (StatusCode::PARTIAL_CONTENT, len));
    assert_eq!(
        serve_for_tests(handle, "main", &get(&uri, Some(&format!("bytes={len}-")))).status(),
        StatusCode::RANGE_NOT_SATISFIABLE
    );
    // Another window's project has no such video; nor does a made-up key, a path, or a POST.
    let _other = window(&app, "w-2");
    assert_eq!(serve_for_tests(handle, "w-2", &get(&uri, None)).status(), StatusCode::NOT_FOUND);
    for bad in [
        "media://localhost/ffffffffffffffff",
        "media://localhost/../../etc/passwd",
        "media://localhost/",
    ] {
        assert_eq!(
            serve_for_tests(handle, "main", &get(bad, None)).status(),
            StatusCode::NOT_FOUND,
            "{bad}"
        );
    }
    let post = Request::builder().method("POST").uri(&uri).body(Vec::new()).unwrap();
    assert_eq!(serve_for_tests(handle, "main", &post).status(), StatusCode::METHOD_NOT_ALLOWED);
    // The file moved: nothing to serve.
    std::fs::remove_file(&file).unwrap();
    assert_eq!(serve_for_tests(handle, "main", &get(&uri, None)).status(), StatusCode::NOT_FOUND);
}

/// With a real `ffmpeg` on the PATH: choose it, analyse a generated clip and find its frames in the workspace.
#[test]
fn a_generated_clip_is_analysed_end_to_end_when_ffmpeg_is_installed() {
    let Some(ffmpeg) = std::env::var_os("PATH")
        .into_iter()
        .flat_map(|p| std::env::split_paths(&p).collect::<Vec<_>>())
        .map(|d| d.join("ffmpeg"))
        .find(|p| p.is_file() && p.with_file_name("ffprobe").is_file())
    else {
        eprintln!("no ffmpeg on the PATH: skipping the end-to-end video test");
        return;
    };
    let dir = TempDir::new("video-e2e");
    let app = test_app(Some(dir.0.join("profiles")));
    app.manage(VideoState::new(dir.0.join("video")));
    crate::paths::allow_under(app.handle(), "main", &dir.0);
    let clip = dir.0.join("recording 2026-10-14 10-40-00.mp4");
    let made = std::process::Command::new(&ffmpeg)
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
        .args(["-filter_complex", "[0:v][1:v]concat=n=2:v=1:a=0", "-pix_fmt", "yuv420p"])
        .arg(&clip)
        .status();
    if !made.is_ok_and(|s| s.success()) {
        eprintln!("this ffmpeg cannot generate the test clip: skipping");
        return;
    }
    let h = app.handle().clone();
    let v =
        block(video_runtime_set_custom(h.clone(), ffmpeg.to_string_lossy().into_owned())).expect("the real ffmpeg passes the self-test");
    assert_eq!(v.status.mode, imt_video::runtime::RuntimeMode::Custom);
    let win = main_window(&app);
    let grants = crate::paths::WindowGrants::of(app.handle(), "main");
    let view = block(video_open(
        h,
        win.clone(),
        grants,
        OpenVideoRequest {
            path: clip.to_string_lossy().into_owned(),
            options: SelectOptions::default(),
            include: false,
        },
    ))
    .expect("the clip is analysed");
    assert!(view.record.frames.len() >= 2, "{:?}", view.record.frames);
    assert_eq!(view.record.alignment.by, imt_core::importers::video::AlignedBy::FileName);
    let source = frames_source(&win).expect("the frames are a source");
    assert_eq!(source.records, view.record.frames.len());
    assert_eq!(Some(source.id), view.source_id);
    let _ = samples();
}
