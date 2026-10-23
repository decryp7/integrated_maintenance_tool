#![no_main]
//! Projects from disk or from other people: an `.debriefproj` workspace file (JSON), a `project.toml` manifest and an
//! `.debriefz` archive (project, share or collector bundle) opened into a scratch folder. The first byte selects.
//! Nothing panics and an extracted project stays inside the destination folder.
use debrief_core::package::{self, ProjectManifest};
use debrief_core::project::ProjectFile;
use libfuzzer_sys::fuzz_target;
use std::path::PathBuf;
use std::sync::OnceLock;

fn scratch() -> &'static PathBuf {
    static D: OnceLock<PathBuf> = OnceLock::new();
    D.get_or_init(|| {
        let d = std::env::temp_dir().join(format!("debrief-fuzz-project-{}", std::process::id()));
        std::fs::create_dir_all(&d).expect("scratch folder");
        d
    })
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let dir = scratch();
    match pick % 3 {
        0 => {
            let path = dir.join("p.debriefproj");
            if std::fs::write(&path, rest).is_ok() {
                let _ = ProjectFile::load(&path);
            }
        }
        1 => {
            if let Ok(text) = std::str::from_utf8(rest) {
                if let Ok(m) = ProjectManifest::parse(text) {
                    let back = ProjectManifest::parse(&m.to_toml().expect("a parsed manifest writes")).expect("and reads back");
                    assert_eq!(back, m);
                }
            }
        }
        _ => {
            let path = dir.join("in.debriefz");
            let dest = dir.join("dest");
            let _ = std::fs::remove_dir_all(&dest);
            if std::fs::write(&path, rest).is_ok() && std::fs::create_dir_all(&dest).is_ok() {
                if let Ok(package::Package::Project { dir: p, .. }) = package::open_package(&path, &dest) {
                    assert!(p.dir.starts_with(&dest));
                }
            }
        }
    }
});
