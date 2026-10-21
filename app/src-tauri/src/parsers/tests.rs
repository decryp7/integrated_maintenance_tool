//! The Settings ▸ Parsers commands against the real store (in process) and a library in a temp folder.

use super::commands::*;
use super::dto::*;
use crate::activity::OpId;
use crate::test_support::Now as _;
use crate::test_support::{import, job_done, main_window, samples, test_app, TempDir};
use crate::{AppState, ErrorCode};
use std::path::Path;
use tauri::Manager;

const MINE: &str = "# shared by a colleague\nschema = 1\nid = \"orders\"\nname = \"Orders\"\nversion = \"1.0.0\"\n[match]\nfiles = ['*.orders']\n[format]\nkind = \"text\"\nregexes = ['^(?P<timestamp>\\S+ \\S+) (?P<level>\\w+) (?P<message>.*)$']\n";

fn grant(app: &tauri::App<tauri::test::MockRuntime>, path: &Path, access: crate::paths::Access) {
    crate::paths::grant(app.handle(), "main", path, access, crate::paths::GrantSource::Dialog).unwrap();
}

#[test]
fn the_library_is_listed_edited_and_persisted() {
    let dir = TempDir::new("parsers-lib");
    let app = test_app(Some(dir.0.join("profiles")));
    let (st, shared) = (|| main_window(&app), || app.state::<AppState>());
    let lib = parsers_list(shared(), st()).now().unwrap();
    assert_eq!(lib.parsers.len(), imt_core::parser::builtin::FILES.len());
    assert!(lib
        .parsers
        .iter()
        .all(|p| p.origin == ParserOrigin::Builtin && p.enabled && p.sources.is_empty()));
    assert_eq!((lib.broken.len(), lib.stale), (0, 0));

    let saved = parser_save(app.handle().clone(), shared(), st(), MINE.into(), None, None)
        .now()
        .unwrap();
    let mine = saved.parsers.iter().find(|p| p.id == "orders").unwrap();
    assert_eq!((mine.origin, mine.kind.as_str()), (ParserOrigin::User, "text"));
    assert!(dir.0.join("parsers/orders.toml").is_file());
    let clash = parser_save(app.handle().clone(), shared(), st(), MINE.into(), None, None)
        .now()
        .unwrap_err();
    assert_eq!(clash.code, ErrorCode::FileExists);
    parser_save(app.handle().clone(), shared(), st(), MINE.into(), None, Some(true))
        .now()
        .unwrap();
    let bad = parser_save(
        app.handle().clone(),
        shared(),
        st(),
        "schema = 1\nid = \"x\"\nname = \n".into(),
        None,
        None,
    )
    .now()
    .unwrap_err();
    assert_eq!((bad.code, bad.line), (ErrorCode::Invalid, Some(3)));
    assert_eq!(parser_get(shared(), "orders".into()).now().unwrap().toml, MINE);
    assert!(parser_get(shared(), "nope".into()).now().is_err());

    let copy = parser_duplicate(app.handle().clone(), shared(), "nlog".into()).now().unwrap();
    assert_eq!((copy.id.as_str(), copy.origin), ("nlog-copy", ParserOrigin::User));
    let off = parser_set_enabled(app.handle().clone(), shared(), st(), "generic".into(), false)
        .now()
        .unwrap();
    assert!(!off.parsers.iter().find(|p| p.id == "generic").unwrap().enabled);
    let removed = parser_remove(app.handle().clone(), shared(), st(), "nlog-copy".into())
        .now()
        .unwrap();
    assert!(removed.parsers.iter().all(|p| p.id != "nlog-copy"));
    let builtin = parser_remove(app.handle().clone(), shared(), st(), "nlog".into())
        .now()
        .unwrap_err();
    assert_eq!(builtin.code, ErrorCode::Invalid);
    // A new app reads the same library.
    let again = test_app(Some(dir.0.join("profiles")));
    let lib = parsers_list(again.state::<AppState>(), main_window(&again)).now().unwrap();
    assert!(lib.parsers.iter().any(|p| p.id == "orders"));
    assert!(!lib.parsers.iter().find(|p| p.id == "generic").unwrap().enabled);
    let v = validate("schema = 2\n");
    assert!(!v.ok && v.error.unwrap().message.contains("update IMT"));
    let v = parser_validate(MINE.into()).now().unwrap();
    assert_eq!((v.ok, v.id.as_deref(), v.version.as_deref()), (true, Some("orders"), Some("1.0.0")));
}

#[test]
fn parsers_are_shared_through_files_the_user_chose() {
    let dir = TempDir::new("parsers-share");
    let app = test_app(Some(dir.0.join("profiles")));
    let shared = || app.state::<AppState>();
    let grants = || crate::paths::WindowGrants::of(app.handle(), "main");
    let file = dir.0.join("orders.toml");
    std::fs::write(&file, MINE).unwrap();
    let path = file.to_string_lossy().into_owned();
    assert!(parser_import_check(shared(), grants(), path.clone()).now().is_err(), "not chosen");
    grant(&app, &file, crate::paths::Access::Read);
    let check = parser_import_check(shared(), grants(), path).now().unwrap();
    assert_eq!((check.id.as_str(), check.relation), ("orders", ParserRelation::New));
    assert_eq!(check.toml, MINE);
    // A newer version of a parser in the library is an update; the existing text comes along for the diff.
    let newer = dir.0.join("nlog.toml");
    let text = imt_core::parser::builtin::file("nlog")
        .unwrap()
        .toml
        .replace("version = \"1.0.0\"", "version = \"1.1.0\"");
    std::fs::write(&newer, text).unwrap();
    grant(&app, &newer, crate::paths::Access::Read);
    let check = parser_import_check(shared(), grants(), newer.to_string_lossy().into_owned())
        .now()
        .unwrap();
    assert_eq!(check.relation, ParserRelation::Newer);
    assert_eq!(check.existing.unwrap().version, "1.0.0");
    let broken = dir.0.join("broken.toml");
    std::fs::write(&broken, "schema = 1\nid = \"b\"\nname = \"B\"\nversion = \"one\"\n").unwrap();
    grant(&app, &broken, crate::paths::Access::Read);
    let e = parser_import_check(shared(), grants(), broken.to_string_lossy().into_owned())
        .now()
        .unwrap_err();
    assert_eq!(e.line, Some(4));

    let out = dir.0.join("out").join("clef.toml");
    std::fs::create_dir_all(out.parent().unwrap()).unwrap();
    let out_s = out.to_string_lossy().into_owned();
    let export = |overwrite| parser_export(shared(), grants(), OpId::default(), "serilog-clef".into(), out_s.clone(), overwrite).now();
    assert!(export(None).is_err(), "not chosen");
    grant(&app, &out, crate::paths::Access::Write);
    export(None).unwrap();
    assert_eq!(
        std::fs::read_to_string(&out).unwrap(),
        imt_core::parser::builtin::file("serilog-clef").unwrap().toml
    );
    let exists = export(None).unwrap_err();
    assert!(
        exists.message.ends_with("already exists; confirm to replace it"),
        "{}",
        exists.message
    );
    export(Some(true)).unwrap();
}

#[test]
fn drafts_parse_with_and_reread_work_on_the_window() {
    let dir = TempDir::new("parsers-window");
    let app = test_app(Some(dir.0.join("profiles")));
    let (st, shared) = (|| main_window(&app), || app.state::<AppState>());
    import(&st(), &[samples().join("serilog-clef")]);
    let lib = parsers_list(shared(), st()).now().unwrap();
    let clef = lib.parsers.iter().find(|p| p.id == "serilog-clef").unwrap();
    assert!(
        !clef.sources.is_empty() && clef.sources.iter().all(|s| s.name.ends_with(".clef")),
        "{lib:?}"
    );

    // The wizard's draft of a changed built-in saves an override.
    let draft = parser_draft(
        shared(),
        ParserDraftRequest {
            file_name: "a.clef".into(),
            base: Some("serilog-clef".into()),
            timestamp_format: Some("%d.%m.%Y %H:%M:%S".into()),
            ..Default::default()
        },
    )
    .now()
    .unwrap();
    assert_eq!((draft.id.as_str(), draft.origin), ("serilog-clef", ParserOrigin::Override));
    let new = parser_draft(
        shared(),
        ParserDraftRequest {
            file_name: "notes.txt".into(),
            regex: Some("^(?P<message>.+)$".into()),
            ..Default::default()
        },
    )
    .now()
    .unwrap();
    assert_eq!((new.id.as_str(), new.origin), ("custom-notes", ParserOrigin::User));
    let unknown = ParserDraftRequest {
        base: Some("nope".into()),
        ..Default::default()
    };
    assert!(parser_draft(shared(), unknown).now().is_err());

    // Saving it makes the window's CLEF sources stale; re-reading reads them with it.
    let saved = parser_save(app.handle().clone(), shared(), st(), draft.toml, Some("serilog-clef".into()), None)
        .now()
        .unwrap();
    assert!(saved.stale > 0, "{saved:?}");
    let job = parsers_reread(app.handle().clone(), st()).now().unwrap();
    job_done(&app, job);
    let lib = parsers_list(shared(), st()).now().unwrap();
    assert_eq!(lib.stale, 0);
    assert!(!lib.parsers.iter().find(|p| p.id == "serilog-clef").unwrap().sources.is_empty());

    // Parse with… the generic parser (the re-read gave the sources new ids).
    let source = lib.parsers.iter().find(|p| p.id == "serilog-clef").unwrap().sources[0].id;
    let job = parser_assign(app.handle().clone(), shared(), st(), source, Some("generic".into()))
        .now()
        .unwrap();
    job_done(&app, job);
    let lib = parsers_list(shared(), st()).now().unwrap();
    assert_eq!(lib.parsers.iter().find(|p| p.id == "generic").unwrap().sources.len(), 1);
    assert!(parser_assign(app.handle().clone(), shared(), st(), source, Some("nope".into()))
        .now()
        .is_err());
}
