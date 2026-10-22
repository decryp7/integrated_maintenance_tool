//! Property-based tests for the files IMT reads back from disk or from other people: parser library TOML
//! (`parsers/*.toml`, user library folders), `.imtproj` workspace files, `project.toml` manifests and `.imtz`
//! project / bundle archives. None of them may panic or hang on any input, and what they accept must be
//! internally consistent (a parsed parser round-trips through its canonical TOML).
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p imt-core --test properties_files`.

use imt_core::bundle;
use imt_core::package::{self, ProjectDir, ProjectManifest};
use imt_core::parser::builtin;
use imt_core::parser::library::Library;
use imt_core::parser::Parser;
use imt_core::project::ProjectFile;
use proptest::prelude::*;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

const CASES: u32 = 128;
const HANG_LIMIT: Duration = Duration::from_secs(15);

/// Runs `f` on a helper thread and fails if it panics or takes longer than [`HANG_LIMIT`].
fn guard<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    let handle = std::thread::spawn(move || {
        let _ = tx.send(f());
    });
    match rx.recv_timeout(HANG_LIMIT) {
        Ok(v) => {
            let _ = handle.join();
            v
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => match handle.join() {
            Err(e) => std::panic::resume_unwind(e),
            Ok(()) => panic!("worker exited without a result"),
        },
        Err(mpsc::RecvTimeoutError::Timeout) => panic!("hang: no result within {HANG_LIMIT:?}"),
    }
}

/// Applies byte-level mutations (flip, insert, delete, truncate) to `seed`.
fn mutate(seed: &[u8], ops: &[(u8, usize, u8)]) -> Vec<u8> {
    let mut v = seed.to_vec();
    for &(op, at, byte) in ops {
        let i = if v.is_empty() { 0 } else { at % (v.len() + 1) };
        match op % 4 {
            0 if i < v.len() => v[i] ^= byte | 1,
            1 => v.insert(i.min(v.len()), byte),
            2 if i < v.len() => {
                v.remove(i);
            }
            3 => v.truncate(i),
            _ => {}
        }
    }
    v
}

fn mutate_str(seed: &str, ops: &[(u8, usize, u8)]) -> String {
    String::from_utf8_lossy(&mutate(seed.as_bytes(), ops)).into_owned()
}

fn ops() -> impl Strategy<Value = Vec<(u8, usize, u8)>> {
    prop::collection::vec((any::<u8>(), any::<usize>(), any::<u8>()), 0..12)
}

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!("imt-propf-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        let _ = std::fs::remove_dir_all(&p);
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

// ---------------------------------------------------------------------------
// Parser library TOML
// ---------------------------------------------------------------------------

/// TOML-shaped text built from parser keys, so cases reach past the TOML syntax checks.
fn parser_like() -> impl Strategy<Value = String> {
    let key = prop::sample::select(vec![
        "schema",
        "id",
        "name",
        "version",
        "description",
        "kind",
        "enabled",
        "timestamp_formats",
        "regexes",
        "output_templates",
        "delimiter",
        "level",
        "message",
        "field",
        "pattern",
        "op",
        "value",
        "sniff",
        "extensions",
    ]);
    let value = prop_oneof![
        Just("1".to_string()),
        Just("\"1.0.0\"".to_string()),
        "-?[0-9]{1,6}".prop_map(|s| s),
        "\"[ -~]{0,30}\"".prop_map(|s| s.replace('\\', "/")),
        Just("[]".to_string()),
        Just("[\"(?P<ts>\\\\S+) (?P<message>.*)\"]".to_string()),
        Just("true".to_string()),
        Just("{ a = 1 }".to_string()),
    ];
    let section = prop::sample::select(vec![
        "",
        "[format]",
        "[match]",
        "[[extract]]",
        "[[pipeline]]",
        "[format.json]",
        "[format.csv]",
    ]);
    prop::collection::vec((section, prop::collection::vec((key, value), 0..5)), 0..5).prop_map(|sections| {
        let mut s = String::from("schema = 1\nid = \"prop\"\nname = \"Prop\"\nversion = \"1.0.0\"\n");
        for (header, kvs) in sections {
            s.push_str(header);
            s.push('\n');
            for (k, v) in kvs {
                s.push_str(&format!("{k} = {v}\n"));
            }
        }
        s
    })
}

fn parser_text() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        parser_like(),
        (any::<prop::sample::Index>(), ops()).prop_map(|(i, o)| mutate_str(builtin::FILES[i.index(builtin::FILES.len())].toml, &o)),
    ]
}

/// Every accepted parser round-trips through its canonical TOML to the same parser.
fn check_parser(text: &str) {
    if let Ok(p) = Parser::from_toml(text) {
        let canonical = p.canonical_toml();
        let again =
            Parser::from_toml(&canonical).unwrap_or_else(|e| panic!("canonical TOML of an accepted parser is refused: {e}\n{canonical}"));
        assert_eq!(again.canonical_toml(), canonical, "canonical TOML is not stable");
        assert_eq!(
            again.reference(),
            p.reference(),
            "the content hash changed through the canonical TOML"
        );
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn parser_from_toml_never_panics(text in parser_text()) {
        guard(move || check_parser(&text));
    }

    #[test]
    fn parser_override_never_panics(i in any::<prop::sample::Index>(), fragment in parser_text()) {
        guard(move || {
            let base = &builtin::parsers()[i.index(builtin::parsers().len())];
            if let Ok(p) = base.with_override(&fragment) {
                assert_eq!(Parser::from_toml(&p.canonical_toml()).map(|q| q.reference()).ok(), Some(p.reference()));
            }
        });
    }

    #[test]
    fn library_check_import_never_panics(text in parser_text()) {
        guard(move || {
            let lib = Library::builtin_only();
            let _ = lib.check_import(&text);
        });
    }

    /// A user library folder full of arbitrary files never fails to load: built-ins are always there, and
    /// every file is either a parser or listed as broken.
    #[test]
    fn library_load_survives_any_folder(files in prop::collection::vec(("[a-z]{1,8}", parser_text()), 0..4), index in prop::option::of(prop_oneof![any::<String>(), Just(r#"{"version":1,"disabled":["generic","nlog"]}"#.to_string()), Just(r#"{"version":99}"#.to_string())])) {
        guard(move || {
            let dir = TempDir::new();
            for (stem, text) in &files {
                std::fs::write(dir.0.join(format!("{stem}.toml")), text).unwrap();
            }
            if let Some(index) = &index {
                std::fs::write(dir.0.join(imt_core::parser::library::INDEX_FILE), index).unwrap();
            }
            let lib = Library::load(&dir.0);
            for b in builtin::parsers() {
                assert!(lib.get(b.id.as_str()).is_some(), "built-in {} missing", b.id.as_str());
            }
            let _ = lib.setup();
        });
    }
}

#[test]
fn every_builtin_parser_round_trips() {
    for f in builtin::FILES {
        check_parser(f.toml);
        assert!(Parser::from_toml(f.toml).is_ok());
    }
}

// ---------------------------------------------------------------------------
// `.imtproj` workspace files and `project.toml`
// ---------------------------------------------------------------------------

const PROJECT_SEED: &str = r#"{"version":2,"name":"p","sources":["a.log","b/c.log","x.zip!/y.log"],"removed":["d.log"],"profile":"name = \"x\"\n","ui":{"notes":{"version":1,"notes":[]}},"clock":{"a.log":120},"store":"store","reports":[{"bad":1}]}"#;

const MANIFEST_SEED: &str =
    "format = \"imt-project\"\nversion = 1\nname = \"demo\"\n\n[[clock_offsets]]\npath = \"a.log\"\noffset_ms = 5\n";

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    /// A damaged `.imtproj` is an error, never a panic; an accepted one has only resolved paths.
    #[test]
    fn project_file_load_never_panics(text in prop_oneof![any::<String>(), (ops()).prop_map(|o| mutate_str(PROJECT_SEED, &o))]) {
        guard(move || {
            let dir = TempDir::new();
            let path = dir.0.join("p.imtproj");
            std::fs::write(&path, &text).unwrap();
            if let Ok(p) = ProjectFile::load(&path) {
                assert!(p.version <= imt_core::project::PROJECT_VERSION);
                // What loads saves and loads again.
                p.save(&path).unwrap();
                let again = ProjectFile::load(&path).unwrap();
                assert_eq!(again.sources, p.sources);
                assert_eq!(again.removed, p.removed);
            }
        });
    }

    #[test]
    fn project_manifest_parse_never_panics(text in prop_oneof![any::<String>(), ops().prop_map(|o| mutate_str(MANIFEST_SEED, &o))]) {
        guard(move || {
            if let Ok(m) = ProjectManifest::parse(&text) {
                assert_eq!(m.format, package::PROJECT_FORMAT);
                assert!(m.version <= package::PROJECT_VERSION);
                let back = ProjectManifest::parse(&m.to_toml().unwrap()).unwrap();
                assert_eq!(back, m);
            }
        });
    }

    #[test]
    fn safe_entry_path_never_escapes(name in prop_oneof![any::<String>(), "([a-z.]{0,3}[/\\\\:]){0,5}[a-z.]{0,4}"]) {
        if let Ok(p) = bundle::safe_entry_path(&name) {
            assert!(p.is_relative(), "{name:?} became {p:?}");
            assert!(!p.components().any(|c| matches!(c, std::path::Component::ParentDir | std::path::Component::RootDir | std::path::Component::Prefix(_))), "{name:?} became {p:?}");
        }
    }
}

#[test]
fn project_file_seed_loads() {
    let dir = TempDir::new();
    let path = dir.0.join("p.imtproj");
    std::fs::write(&path, PROJECT_SEED).unwrap();
    let p = ProjectFile::load(&path).unwrap();
    assert_eq!(p.sources.len(), 3);
    assert!(p.reports.is_empty(), "an unreadable report is skipped");
    assert!(ProjectManifest::parse(MANIFEST_SEED).is_ok());
}

// ---------------------------------------------------------------------------
// `.imtz` archives
// ---------------------------------------------------------------------------

/// A small exported project, built once.
fn project_archive() -> &'static Vec<u8> {
    static BYTES: OnceLock<Vec<u8>> = OnceLock::new();
    BYTES.get_or_init(|| {
        let dir = TempDir::new();
        let p = ProjectDir::create(&dir.0.join("demo"), "demo").unwrap();
        std::fs::create_dir_all(p.dir.join("sources")).unwrap();
        std::fs::write(p.dir.join("sources/app.log"), "2026-10-14 10:00:00 INFO hello\n".repeat(20)).unwrap();
        std::fs::write(p.dir.join("snapshots/one.json"), "{\"a\":1}").unwrap();
        let out = dir.0.join("demo.imtz");
        p.export(&out).unwrap();
        std::fs::read(out).unwrap()
    })
}

fn zip_of(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        if w.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).is_ok() {
            let _ = w.write_all(data);
        }
    }
    w.finish().map(|c| c.into_inner()).unwrap_or_default()
}

fn archive_bytes() -> impl Strategy<Value = Vec<u8>> {
    let entry_name = prop_oneof![
        Just("project.toml".to_string()),
        Just("manifest.json".to_string()),
        "[a-z./\\\\]{1,16}",
        any::<String>(),
    ];
    let entry_data = prop_oneof![
        Just(MANIFEST_SEED.as_bytes().to_vec()),
        Just(b"format = \"other\"\n".to_vec()),
        Just(br#"{"format":"imt-share","version":1,"entries":[]}"#.to_vec()),
        prop::collection::vec(any::<u8>(), 0..256),
    ];
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..1024),
        ops().prop_map(|o| mutate(project_archive(), &o)),
        prop::collection::vec((entry_name, entry_data), 0..5).prop_map(|f| zip_of(&f)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    /// Any bytes named `.imtz` open as a project, a share or collector bundle, or fail with a message; nothing
    /// is ever written outside the destination folder.
    #[test]
    fn open_package_never_panics(bytes in archive_bytes()) {
        guard(move || {
            let dir = TempDir::new();
            let path = dir.0.join("in.imtz");
            std::fs::write(&path, &bytes).unwrap();
            let dest = dir.0.join("dest");
            std::fs::create_dir_all(&dest).unwrap();
            if let Ok(package::Package::Project { dir: p, .. }) = package::open_package(&path, &dest) {
                assert!(p.dir.starts_with(&dest));
            }
            // Nothing but `in.imtz` and `dest` in the parent.
            let mut names: Vec<_> = std::fs::read_dir(&dir.0).unwrap().filter_map(|e| e.ok()).map(|e| e.file_name()).collect();
            names.sort();
            assert_eq!(names, vec![std::ffi::OsString::from("dest"), std::ffi::OsString::from("in.imtz")]);
        });
    }
}

#[test]
fn exported_project_opens() {
    let dir = TempDir::new();
    let path = dir.0.join("demo.imtz");
    std::fs::write(&path, project_archive()).unwrap();
    let dest = dir.0.join("dest");
    match package::open_package(&path, &dest).unwrap() {
        package::Package::Project { dir, manifest } => {
            assert_eq!(manifest.name, "demo");
            assert!(dir.dir.join("sources/app.log").is_file());
        }
        other => panic!("expected a project, got {other:?}"),
    }
    let _: &Path = &dest;
}
