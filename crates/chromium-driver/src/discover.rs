//! Finding an installed Chromium-based browser. The candidate lists are pure functions of the platform and an
//! environment lookup; [`find`] only checks which candidate exists.

use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Linux,
}

impl Os {
    pub const CURRENT: Os = if cfg!(windows) {
        Os::Windows
    } else if cfg!(target_os = "macos") {
        Os::Mac
    } else {
        Os::Linux
    };
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    Edge,
    Chrome,
    Chromium,
}

/// An executable that may be a Chromium browser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Browser {
    pub flavor: Flavor,
    pub path: PathBuf,
}

/// Candidates in order of preference: Edge first on Windows (always present), Chrome first elsewhere.
/// `env` looks up an environment variable.
pub fn candidates(os: Os, env: &dyn Fn(&str) -> Option<String>) -> Vec<Browser> {
    let mut v = Vec::new();
    let mut add = |flavor, p: PathBuf| v.push(Browser { flavor, path: p });
    match os {
        Os::Windows => {
            let roots: Vec<PathBuf> = ["ProgramFiles(x86)", "ProgramFiles", "LOCALAPPDATA"]
                .iter()
                .filter_map(|k| env(k))
                .map(PathBuf::from)
                .collect();
            for r in &roots {
                add(Flavor::Edge, r.join(r"Microsoft\Edge\Application\msedge.exe"));
            }
            for r in &roots {
                add(Flavor::Chrome, r.join(r"Google\Chrome\Application\chrome.exe"));
            }
            for r in &roots {
                add(Flavor::Chromium, r.join(r"Chromium\Application\chrome.exe"));
            }
        }
        Os::Mac => {
            let mut roots = vec![PathBuf::from("/Applications")];
            if let Some(h) = env("HOME") {
                roots.push(PathBuf::from(h).join("Applications"));
            }
            for r in &roots {
                add(Flavor::Chrome, r.join("Google Chrome.app/Contents/MacOS/Google Chrome"));
            }
            for r in &roots {
                add(Flavor::Edge, r.join("Microsoft Edge.app/Contents/MacOS/Microsoft Edge"));
            }
            for r in &roots {
                add(Flavor::Chromium, r.join("Chromium.app/Contents/MacOS/Chromium"));
            }
        }
        Os::Linux => {
            for (f, n) in [
                (Flavor::Chrome, "google-chrome"),
                (Flavor::Chrome, "google-chrome-stable"),
                (Flavor::Edge, "microsoft-edge"),
                (Flavor::Edge, "microsoft-edge-stable"),
                (Flavor::Chromium, "chromium"),
                (Flavor::Chromium, "chromium-browser"),
            ] {
                for d in ["/usr/bin", "/usr/local/bin", "/snap/bin", "/opt/google/chrome"] {
                    add(f, Path::new(d).join(n));
                }
            }
        }
    }
    v
}

/// The first existing candidate. `CHROMIUM_PATH` (an explicit executable) wins.
pub fn find() -> Option<Browser> {
    find_with(Os::CURRENT, &|k| std::env::var(k).ok(), &|p| p.is_file())
}

pub fn find_with(os: Os, env: &dyn Fn(&str) -> Option<String>, exists: &dyn Fn(&Path) -> bool) -> Option<Browser> {
    if let Some(p) = env("CHROMIUM_PATH").map(PathBuf::from).filter(|p| exists(p)) {
        return Some(Browser { flavor: Flavor::Chromium, path: p });
    }
    candidates(os, env).into_iter().find(|b| exists(&b.path))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env(k: &str) -> Option<String> {
        match k {
            "ProgramFiles" => Some(r"C:\Program Files".into()),
            "HOME" => Some("/Users/u".into()),
            _ => None,
        }
    }

    #[test]
    fn windows_prefers_edge() {
        let c = candidates(Os::Windows, &env);
        assert_eq!(c[0].flavor, Flavor::Edge);
        assert!(c[0].path.to_string_lossy().contains("msedge.exe"));
        assert!(c.iter().any(|b| b.flavor == Flavor::Chrome));
    }

    #[test]
    fn mac_checks_user_applications_too() {
        let c = candidates(Os::Mac, &env);
        assert!(c.iter().any(|b| b.path.starts_with("/Users/u/Applications")));
        assert_eq!(c[0].flavor, Flavor::Chrome);
    }

    #[test]
    fn linux_lists_common_names() {
        let c = candidates(Os::Linux, &env);
        assert!(c.iter().any(|b| b.path.ends_with("chromium-browser")));
    }

    #[test]
    fn finds_first_existing_and_honours_override() {
        let only = PathBuf::from("/usr/bin/chromium");
        let b = find_with(Os::Linux, &env, &|p| p == only).unwrap();
        assert_eq!(b.flavor, Flavor::Chromium);
        let over = |k: &str| (k == "CHROMIUM_PATH").then(|| "/x/chrome".to_string());
        assert_eq!(find_with(Os::Linux, &over, &|p| p == Path::new("/x/chrome")).unwrap().path, PathBuf::from("/x/chrome"));
        assert!(find_with(Os::Linux, &env, &|_| false).is_none());
    }
}
