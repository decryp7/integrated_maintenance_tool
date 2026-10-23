//! Notes through their commands (ADR 0021): anchors filled and checked by the engine, typed refusals, the record
//! saved with a project and restored in another window with every anchor resolving (the event in the project's
//! store, the image from the project's copy after the original is gone), and the AI's notes file.

use super::commands::*;
use crate::test_support::{block, main_window, samples, test_app, TempDir};
use crate::windows::CurrentWindow;
use crate::ErrorCode;
use debrief_notes::context::Focus;
use debrief_notes::{Anchor, NoteChange, NoteDraft, Region};
use serde_json::{json, Value};
use tauri::test::MockRuntime;

fn draft(v: Value) -> NoteDraft {
    serde_json::from_value(v).unwrap()
}

fn add(w: CurrentWindow, v: Value) -> crate::CmdResult<debrief_notes::Note> {
    block(notes_add(w, draft(v)))
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for f in std::fs::read_dir(from).unwrap().flatten() {
        if f.path().is_dir() {
            copy_dir(&f.path(), &to.join(f.file_name()));
        } else {
            std::fs::copy(f.path(), to.join(f.file_name())).unwrap();
        }
    }
}

/// A test app whose main window opened a copy of the log4net samples and a 40×20 screenshot next to them.
fn workspace(tag: &str) -> (TempDir, tauri::App<MockRuntime>, std::path::PathBuf) {
    let dir = TempDir::new(tag);
    let logs = dir.0.join("logs");
    copy_dir(&samples().join("log4net"), &logs);
    let png = logs.join("screen.png");
    std::fs::write(&png, crate::activity::report::sample::png(40, 20, |x, _| [x as u8 * 6, 0, 0])).unwrap();
    let app = test_app(Some(dir.0.join("profiles")));
    crate::paths::allow_under(app.handle(), "main", &dir.0);
    let opened = crate::test_support::finished(
        &app,
        block(crate::workspace_commands::open_paths(
            app.handle().clone(),
            main_window(&app),
            vec![logs.to_string_lossy().into_owned()],
        )),
    )
    .unwrap();
    assert!(opened["total"].as_u64().unwrap() > 0);
    (dir, app, png)
}

fn first_event(app: &tauri::App<MockRuntime>) -> u64 {
    main_window(app).engine.store().unwrap().ids_at(0, 1).unwrap()[0].0
}

fn grants(app: &tauri::App<MockRuntime>, label: &str) -> crate::paths::WindowGrants {
    crate::paths::WindowGrants::of(app.handle(), label)
}

#[test]
fn anchors_are_filled_and_checked_by_the_engine() {
    let (_dir, app, _png) = workspace("notes-anchors");
    let id = first_event(&app);
    // The webview's excerpt is not trusted: the engine takes the event's own.
    let n = add(main_window(&app), json!({"anchor": {"kind": "event", "eventId": id, "excerpt": "forged"}, "text": "Look **here**", "tags": ["Disk"], "severity": "problem"})).unwrap();
    let Anchor::Event { excerpt, ts, source, .. } = &n.anchor else {
        panic!("{n:?}")
    };
    assert!(excerpt != "forged" && !excerpt.is_empty() && ts.is_some() && !source.is_empty());
    assert_eq!((n.id.as_str(), n.tags[0].as_str()), ("n1", "disk"));
    assert_eq!(n.author, super::author().chars().take(64).collect::<String>());
    // Typed refusals.
    let code = |r: crate::CmdResult<debrief_notes::Note>| r.unwrap_err().code;
    assert_eq!(
        code(add(
            main_window(&app),
            json!({"anchor": {"kind": "event", "eventId": u32::MAX}, "text": "x"})
        )),
        ErrorCode::NotFound
    );
    assert_eq!(
        code(add(
            main_window(&app),
            json!({"anchor": {"kind": "range", "from": 5, "to": 1}, "text": "x"})
        )),
        ErrorCode::Invalid
    );
    assert_eq!(
        code(add(main_window(&app), json!({"anchor": {"kind": "project"}, "text": " "}))),
        ErrorCode::Invalid
    );
    assert_eq!(
        code(add(
            main_window(&app),
            json!({"anchor": {"kind": "source", "path": "/not/listed.log"}, "text": "x"})
        )),
        ErrorCode::Invalid
    );
    let unopened = json!({"anchor": {"kind": "image", "image": {"key": debrief_notes::image_key(b"never"), "name": "x.png", "width": 4, "height": 4}, "regions": [{"x": 0, "y": 0, "w": 1, "h": 1}]}, "text": "x"});
    assert_eq!(code(add(main_window(&app), unopened)), ErrorCode::NotFound);
    let big = "x".repeat(debrief_notes::MAX_TEXT_BYTES + 1);
    assert_eq!(
        code(add(main_window(&app), json!({"anchor": {"kind": "project"}, "text": big}))),
        ErrorCode::Limit
    );
    // Sources of the workspace, changes and removal.
    let summary = crate::test_support::store_summary(&main_window(&app));
    let path = summary.sources[0].path.clone();
    let s = add(
        main_window(&app),
        json!({"anchor": {"kind": "source", "path": path}, "text": "rotated hourly"}),
    )
    .unwrap();
    let change = NoteChange {
        text: Some("rotated daily".into()),
        ..NoteChange::default()
    };
    assert_eq!(
        block(notes_update(main_window(&app), s.id.clone(), change)).unwrap().text,
        "rotated daily"
    );
    let regions = NoteChange {
        regions: Some(vec![Region { x: 0, y: 0, w: 1, h: 1 }]),
        ..NoteChange::default()
    };
    assert_eq!(
        block(notes_update(main_window(&app), s.id.clone(), regions)).unwrap_err().code,
        ErrorCode::Invalid
    );
    assert!(block(notes_remove(main_window(&app), s.id.clone())).unwrap());
    assert_eq!(block(notes_remove(main_window(&app), s.id)).unwrap_err().code, ErrorCode::NotFound);
    let list = block(notes_list(main_window(&app))).unwrap();
    assert_eq!((list.notes.len(), list.notes[0].resolved, list.max_regions), (1, true, 16));
}

#[test]
fn image_regions_are_checked_against_the_image_and_kept_in_the_project() {
    let (_dir, app, png) = workspace("notes-images");
    // Listed in the Explorer (an unrecognised file of the opened folder): opens without a dialog.
    let img = block(notes_image_open(
        main_window(&app),
        grants(&app, "main"),
        png.to_string_lossy().into_owned(),
    ))
    .unwrap();
    assert_eq!((img.width, img.height, img.key.len()), (40, 20, 64));
    assert!(img.data_url.starts_with("data:image/"));
    let region = |x: u32, w: u32| json!({"anchor": {"kind": "image", "image": {"key": img.key, "name": "forged", "width": 4000, "height": 4000}, "regions": [{"x": x, "y": 0, "w": w, "h": 10}]}, "text": "error banner"});
    // The engine's size, not the webview's: a region beyond 40 px wide is outside.
    assert_eq!(add(main_window(&app), region(30, 20)).unwrap_err().code, ErrorCode::Invalid);
    let n = add(main_window(&app), region(5, 10)).unwrap();
    let Anchor::Image { image, .. } = &n.anchor else { panic!() };
    assert_eq!((image.name.as_str(), image.width), ("screen.png", 40));
    let store_dir = main_window(&app).engine.store().unwrap().store_dir();
    assert!(
        super::images::find_copy(&store_dir, &img.key).is_some(),
        "the clean copy is in the project"
    );
    assert_eq!(block(notes_image_get(main_window(&app), img.key.clone())).unwrap().key, img.key);
    assert_eq!(
        block(notes_image_get(main_window(&app), "nope".into())).unwrap_err().code,
        ErrorCode::NotFound
    );
    // Files that are not images, or not chosen, are refused.
    let other = std::env::temp_dir().join("debrief-notes-not-chosen.png");
    std::fs::write(&other, b"x").unwrap();
    assert!(block(notes_image_open(
        main_window(&app),
        grants(&app, "main"),
        other.to_string_lossy().into_owned()
    ))
    .is_err());
    let fake = png.with_file_name("fake.png");
    std::fs::write(&fake, b"not a png").unwrap();
    assert_eq!(
        block(notes_image_open(
            main_window(&app),
            grants(&app, "main"),
            fake.to_string_lossy().into_owned()
        ))
        .unwrap_err()
        .code,
        ErrorCode::Invalid
    );
}

/// Notes are saved with the project and restored in another window: the event anchor resolves in the project's
/// store and the image opens from the project's copy, without the original files.
#[test]
fn anchors_resolve_after_reopening_a_project() {
    let (dir, app, png) = workspace("notes-project");
    let id = first_event(&app);
    add(
        main_window(&app),
        json!({"anchor": {"kind": "event", "eventId": id}, "text": "first failure"}),
    )
    .unwrap();
    add(
        main_window(&app),
        json!({"anchor": {"kind": "range", "from": 1, "to": 2}, "text": "outage"}),
    )
    .unwrap();
    let img = block(notes_image_open(
        main_window(&app),
        grants(&app, "main"),
        png.to_string_lossy().into_owned(),
    ))
    .unwrap();
    add(main_window(&app), json!({"anchor": {"kind": "image", "image": {"key": img.key, "name": "", "width": 1, "height": 1}, "regions": [{"x": 1, "y": 1, "w": 5, "h": 5}]}, "text": "red bar"})).unwrap();
    let record = serde_json::to_value(block(notes_record(main_window(&app))).unwrap()).unwrap();
    assert_eq!(record["version"], 1);
    let file = dir.0.join("incident.debriefz").to_string_lossy().into_owned();
    crate::test_support::finished(
        &app,
        block(crate::workspace_commands::save_project(
            app.handle().clone(),
            main_window(&app),
            grants(&app, "main"),
            Default::default(),
            file.clone(),
            "Incident".into(),
            json!({ "notes": record }),
            None,
            None,
        )),
    )
    .unwrap();
    std::fs::remove_dir_all(dir.0.join("logs")).unwrap();
    crate::paths::allow_under(app.handle(), "w-2", &dir.0);
    let w2 = || CurrentWindow::of(app.handle(), "w-2").unwrap();
    let opened = crate::test_support::finished(
        &app,
        block(crate::workspace_commands::open_project(
            app.handle().clone(),
            w2(),
            grants(&app, "w-2"),
            Default::default(),
            file,
        )),
    )
    .unwrap();
    let restored = block(notes_restore(w2(), Some(opened["ui"]["notes"].clone()))).unwrap();
    assert_eq!((restored.count, restored.warning), (3, None));
    let list = block(notes_list(w2())).unwrap();
    assert!(list.notes.iter().all(|n| n.resolved), "{:?}", list.notes);
    let back = block(notes_image_get(w2(), img.key.clone())).unwrap();
    assert_eq!(
        (back.width, back.height, back.path),
        (40, 20, None),
        "the original's size, from the project's copy"
    );
    // A note on an event the store does not have says so instead of disappearing.
    let mut v = opened["ui"]["notes"].clone();
    v["notes"][0]["anchor"]["eventId"] = json!(u32::MAX);
    block(notes_restore(w2(), Some(v))).unwrap();
    assert!(!block(notes_list(w2())).unwrap().notes[0].resolved);
    // Damaged and newer records restore nothing and say why; none clears.
    let r = block(notes_restore(w2(), Some(json!({"version": 99})))).unwrap();
    assert!(r.count == 0 && r.warning.unwrap().contains("newer"));
    assert_eq!(block(notes_restore(w2(), None)).unwrap(), NotesRestore { count: 0, warning: None });
}

#[test]
fn the_ai_reads_the_notes_as_a_knowledge_file_focused_first() {
    let mut set = debrief_notes::NoteSet::default();
    assert!(super::ai_file(&set, &Focus::default()).is_none());
    for (anchor, text) in [(Anchor::Instant { at: 10 }, "early"), (Anchor::Instant { at: 500 }, "focused")] {
        let d = NoteDraft {
            anchor,
            text: text.into(),
            tags: None,
            severity: None,
        };
        set.add(&d, "", 0).unwrap();
    }
    let f = super::ai_file(
        &set,
        &Focus {
            range: Some((400, 600)),
            ..Focus::default()
        },
    )
    .unwrap();
    assert_eq!((f.id.as_str(), f.name.as_str()), (super::NOTES_KNOWLEDGE_ID, "Notes"));
    assert!(f.text.find("focused").unwrap() < f.text.find("early").unwrap());
}
