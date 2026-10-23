//! Finding an `ffmpeg` already on this computer (ADR 0033 §1), for "Use this one" in Settings ▸ Video: the usual
//! install folders of Homebrew, package managers and Windows installs, and the folders of `PATH`. Pure over the lists
//! it is given; only existence and the presence of a sibling `ffprobe` are looked at (nothing is run).

use crate::api::DetectedFfmpeg;
use crate::run::exe;
use std::path::{Path, PathBuf};

/// Usual places, with the name shown for them.
#[cfg(not(windows))]
const KNOWN: &[(&str, &str)] = &[
    ("/opt/homebrew/bin", "Homebrew"),
    ("/usr/local/bin", "Homebrew or a manual install"),
    ("/usr/bin", "the system"),
    ("/snap/bin", "Snap"),
];
#[cfg(windows)]
const KNOWN: &[(&str, &str)] = &[
    ("C:\\ffmpeg\\bin", "a manual install"),
    ("C:\\Program Files\\ffmpeg\\bin", "a manual install"),
    ("C:\\ProgramData\\chocolatey\\bin", "Chocolatey"),
];

fn origin_of(dir: &Path) -> String {
    let d = dir.to_string_lossy();
    for (known, name) in KNOWN {
        if d == *known {
            return (*name).to_string();
        }
    }
    if d.to_ascii_lowercase().contains("scoop") {
        return "Scoop".into();
    }
    "your PATH".into()
}

/// The `ffmpeg` programs (with `ffprobe` beside them) in the usual places and in `path_var`'s folders, each once.
pub fn detect(path_var: Option<&std::ffi::OsStr>) -> Vec<DetectedFfmpeg> {
    let mut dirs: Vec<PathBuf> = KNOWN.iter().map(|(d, _)| PathBuf::from(d)).collect();
    if let Some(p) = path_var {
        dirs.extend(std::env::split_paths(p));
    }
    detect_in(&dirs)
}

/// [`detect`] over explicit folders.
pub fn detect_in(dirs: &[PathBuf]) -> Vec<DetectedFfmpeg> {
    let mut out: Vec<DetectedFfmpeg> = Vec::new();
    let mut seen: Vec<PathBuf> = Vec::new();
    for dir in dirs {
        let ffmpeg = dir.join(exe("ffmpeg"));
        if !ffmpeg.is_file() || !dir.join(exe("ffprobe")).is_file() {
            continue;
        }
        // The same program reached through a link (Homebrew's `bin` entries) is listed once.
        let real = std::fs::canonicalize(&ffmpeg).unwrap_or_else(|_| ffmpeg.clone());
        if seen.contains(&real) {
            continue;
        }
        seen.push(real);
        out.push(DetectedFfmpeg {
            path: ffmpeg.to_string_lossy().into_owned(),
            origin: origin_of(dir),
        });
        if out.len() >= 8 {
            break;
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn install(dir: &Path, with_probe: bool) {
        std::fs::create_dir_all(dir).unwrap();
        std::fs::write(dir.join(exe("ffmpeg")), b"x").unwrap();
        if with_probe {
            std::fs::write(dir.join(exe("ffprobe")), b"x").unwrap();
        }
    }

    #[test]
    fn only_complete_installs_are_listed_once() {
        let d = tempfile::tempdir().unwrap();
        let (a, b, c) = (d.path().join("a"), d.path().join("scoop").join("b"), d.path().join("c"));
        install(&a, true);
        install(&b, true);
        install(&c, false);
        let found = detect_in(&[a.clone(), a.clone(), b.clone(), c, d.path().join("none")]);
        assert_eq!(found.len(), 2);
        assert_eq!(found[0].origin, "your PATH");
        assert_eq!(found[1].origin, "Scoop");
        assert!(found[0].path.ends_with(&exe("ffmpeg")));
        let path = std::env::join_paths([&a, &b]).unwrap();
        assert!(detect(Some(&path)).len() >= 2);
        assert!(detect(None).len() <= 8);
    }

    #[test]
    fn a_link_to_the_same_program_is_one_entry() {
        #[cfg(unix)]
        {
            let d = tempfile::tempdir().unwrap();
            let real = d.path().join("real");
            install(&real, true);
            let linked = d.path().join("bin");
            std::fs::create_dir_all(&linked).unwrap();
            std::os::unix::fs::symlink(real.join("ffmpeg"), linked.join("ffmpeg")).unwrap();
            std::os::unix::fs::symlink(real.join("ffprobe"), linked.join("ffprobe")).unwrap();
            assert_eq!(detect_in(&[linked, real]).len(), 1);
        }
    }

    #[test]
    fn known_places_have_names() {
        assert_eq!(origin_of(Path::new(KNOWN[0].0)), KNOWN[0].1);
    }
}
