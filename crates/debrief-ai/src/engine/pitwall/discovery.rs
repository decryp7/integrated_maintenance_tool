//! Finding a running Pitwall (Pitwall ADR 0009, Debrief ADR 0059): its `discovery.json` in Pitwall's data folder, else
//! the well-known port on 127.0.0.1. Pure decisions over an injected [`Locator`], so tests use a temporary folder and
//! their own port and never the user's real Pitwall.

use serde::Deserialize;
use std::path::{Path, PathBuf};
use std::sync::Arc;

/// The discovery file's name in Pitwall's data folder.
pub const FILE_NAME: &str = "discovery.json";
/// Largest discovery file read.
pub const MAX_BYTES: u64 = 16 * 1024;
/// Pitwall's well-known port (next to Ollama's 11434).
pub const WELL_KNOWN_PORT: u16 = 11435;

/// The fields of `discovery.json` Debrief reads; unknown fields are ignored (the file grows additively).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Discovery {
    pub format: u32,
    pub version: String,
    pub api_version: u32,
    pub pid: u32,
    pub port: u16,
    pub base_url: String,
}

impl Discovery {
    /// Parses the file: bounded, typed, and only a base URL on this machine is trusted.
    pub fn parse(text: &str) -> Result<Discovery, String> {
        if text.len() as u64 > MAX_BYTES {
            return Err("Pitwall's discovery file is too large".into());
        }
        let d: Discovery = serde_json::from_str(text).map_err(|e| format!("Pitwall's discovery file is damaged ({e})"))?;
        if d.base_url != format!("http://127.0.0.1:{}", d.port) || d.port < 1024 {
            return Err("Pitwall's discovery file does not point at this computer".into());
        }
        if d.version.len() > 64 {
            return Err("Pitwall's discovery file is damaged (version)".into());
        }
        Ok(d)
    }
}

/// Which operating system's folder rule applies.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Os {
    Windows,
    Mac,
    Other,
}

impl Os {
    pub fn current() -> Os {
        if cfg!(windows) {
            Os::Windows
        } else if cfg!(target_os = "macos") {
            Os::Mac
        } else {
            Os::Other
        }
    }
}

/// The environment Pitwall's data folder is derived from (the same rule as Pitwall's own).
#[derive(Debug, Clone, Default)]
pub struct FolderEnv {
    /// `PITWALL_HOME`.
    pub pitwall_home: Option<PathBuf>,
    pub home: Option<PathBuf>,
    /// `APPDATA` (Windows).
    pub appdata: Option<PathBuf>,
    /// `XDG_DATA_HOME`.
    pub xdg_data_home: Option<PathBuf>,
}

impl FolderEnv {
    pub fn from_process() -> FolderEnv {
        let path = |n: &str| std::env::var_os(n).filter(|v| !v.is_empty()).map(PathBuf::from);
        FolderEnv {
            pitwall_home: path("PITWALL_HOME"),
            home: path("HOME").or_else(|| path("USERPROFILE")),
            appdata: path("APPDATA"),
            xdg_data_home: path("XDG_DATA_HOME"),
        }
    }
}

/// Pitwall's data folder.
pub fn data_dir(env: &FolderEnv, os: Os) -> PathBuf {
    if let Some(h) = &env.pitwall_home {
        return h.clone();
    }
    let home = env.home.clone().unwrap_or_default();
    match os {
        Os::Windows => env.appdata.clone().unwrap_or(home).join("Pitwall"),
        Os::Mac => home.join("Library/Application Support/Pitwall"),
        Os::Other => env
            .xdg_data_home
            .clone()
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local/share"))
            .join("pitwall"),
    }
}

/// Whether a process id is alive.
pub type PidAlive = Arc<dyn Fn(u32) -> bool + Send + Sync>;

/// Where to look for Pitwall.
#[derive(Clone)]
pub struct Locator {
    /// Pitwall's data folder.
    pub data_dir: PathBuf,
    /// The port tried when there is no discovery file (`None`: only the discovery file counts; tests).
    pub fallback_port: Option<u16>,
    pub pid_alive: PidAlive,
}

impl std::fmt::Debug for Locator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Locator")
            .field("data_dir", &self.data_dir)
            .field("fallback_port", &self.fallback_port)
            .finish()
    }
}

/// What the files say, before anything is asked over the network.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Found {
    /// No data folder: Pitwall was never run here.
    Absent,
    /// A discovery file of a live process.
    Discovered(Discovery),
    /// No usable discovery file; `port` may still answer (the well-known port), `installed` when the data folder exists.
    Guess {
        port: Option<u16>,
        installed: bool,
        note: Option<String>,
    },
}

impl Locator {
    /// The real machine: Pitwall's folder for this user, the well-known port, the OS process table.
    pub fn system() -> Locator {
        Locator {
            data_dir: data_dir(&FolderEnv::from_process(), Os::current()),
            fallback_port: Some(WELL_KNOWN_PORT),
            pid_alive: Arc::new(pid_alive),
        }
    }

    /// Reads the discovery file (bounded) and decides where Pitwall may be.
    pub fn look(&self) -> Found {
        let installed = self.data_dir.is_dir();
        let guess = |note: Option<String>| Found::Guess {
            port: self.fallback_port,
            installed,
            note,
        };
        let path = self.data_dir.join(FILE_NAME);
        let text = match read_bounded(&path) {
            Ok(Some(t)) => t,
            Ok(None) if !installed && self.fallback_port.is_none() => return Found::Absent,
            Ok(None) => return guess(None),
            Err(e) => return guess(Some(e)),
        };
        match Discovery::parse(&text) {
            Ok(d) if (self.pid_alive)(d.pid) => Found::Discovered(d),
            // A file left behind by a crashed Pitwall: the server may still answer on the well-known port.
            Ok(_) => guess(None),
            Err(e) => guess(Some(e)),
        }
    }
}

fn read_bounded(path: &Path) -> Result<Option<String>, String> {
    use std::io::Read;
    let f = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(format!("Pitwall's discovery file cannot be read ({e})")),
    };
    let mut text = String::new();
    f.take(MAX_BYTES + 1)
        .read_to_string(&mut text)
        .map_err(|_| "Pitwall's discovery file is damaged".to_string())?;
    Ok(Some(text))
}

/// Whether process `pid` exists (any user).
pub fn pid_alive(pid: u32) -> bool {
    use sysinfo::{Pid, ProcessRefreshKind, ProcessesToUpdate, System};
    if pid == 0 {
        return false;
    }
    let mut sys = System::new();
    let p = Pid::from_u32(pid);
    sys.refresh_processes_specifics(ProcessesToUpdate::Some(&[p]), true, ProcessRefreshKind::nothing());
    sys.process(p).is_some()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn file(port: u16, pid: u32) -> String {
        format!(
            r#"{{"format":1,"version":"1.0.4","apiVersion":1,"pid":{pid},"port":{port},"baseUrl":"http://127.0.0.1:{port}","startedAt":7,"future":true}}"#
        )
    }

    fn locator(dir: &Path, fallback: Option<u16>) -> Locator {
        Locator {
            data_dir: dir.to_path_buf(),
            fallback_port: fallback,
            pid_alive: Arc::new(|pid| pid == 42),
        }
    }

    #[test]
    fn parsing_is_strict_about_the_address_and_lenient_about_new_fields() {
        let d = Discovery::parse(&file(23456, 42)).unwrap();
        assert_eq!((d.api_version, d.port, d.version.as_str(), d.format), (1, 23456, "1.0.4", 1));
        assert!(
            Discovery::parse(&file(23456, 42).replace("http://127.0.0.1:23456", "http://evil.example:23456"))
                .unwrap_err()
                .contains("this computer")
        );
        assert!(Discovery::parse(&file(80, 42)).unwrap_err().contains("this computer"));
        assert!(Discovery::parse("{").unwrap_err().contains("damaged"));
        assert!(Discovery::parse(&" ".repeat(MAX_BYTES as usize + 1))
            .unwrap_err()
            .contains("too large"));
        assert!(Discovery::parse(&file(23456, 42).replace("1.0.4", &"9".repeat(65))).is_err());
    }

    #[test]
    fn the_folder_rule_matches_pitwalls() {
        let env = FolderEnv {
            home: Some("/h".into()),
            appdata: Some("C:/A".into()),
            xdg_data_home: Some("/x".into()),
            ..FolderEnv::default()
        };
        assert_eq!(data_dir(&env, Os::Mac), PathBuf::from("/h/Library/Application Support/Pitwall"));
        assert_eq!(data_dir(&env, Os::Windows), PathBuf::from("C:/A/Pitwall"));
        assert_eq!(data_dir(&env, Os::Other), PathBuf::from("/x/pitwall"));
        let rel = FolderEnv {
            xdg_data_home: Some("rel".into()),
            ..env.clone()
        };
        assert_eq!(data_dir(&rel, Os::Other), PathBuf::from("/h/.local/share/pitwall"));
        let own = FolderEnv {
            pitwall_home: Some("/p".into()),
            ..env
        };
        assert_eq!(data_dir(&own, Os::Mac), PathBuf::from("/p"));
        assert!(matches!(Os::current(), Os::Windows | Os::Mac | Os::Other));
        let _ = FolderEnv::from_process();
    }

    #[test]
    fn look_reads_the_file_of_a_live_process_and_guesses_otherwise() {
        let d = tempfile::tempdir().unwrap();
        let missing = d.path().join("none");
        assert_eq!(locator(&missing, None).look(), Found::Absent);
        assert_eq!(
            locator(&missing, Some(9999)).look(),
            Found::Guess {
                port: Some(9999),
                installed: false,
                note: None
            }
        );
        assert_eq!(
            locator(d.path(), None).look(),
            Found::Guess {
                port: None,
                installed: true,
                note: None
            }
        );
        std::fs::write(d.path().join(FILE_NAME), file(23456, 42)).unwrap();
        assert!(matches!(locator(d.path(), None).look(), Found::Discovered(x) if x.port == 23456));
        // A stale file (dead process) is a guess.
        std::fs::write(d.path().join(FILE_NAME), file(23456, 7)).unwrap();
        assert!(matches!(locator(d.path(), Some(1)).look(), Found::Guess { note: None, .. }));
        std::fs::write(d.path().join(FILE_NAME), "garbage").unwrap();
        assert!(matches!(locator(d.path(), None).look(), Found::Guess { note: Some(n), .. } if n.contains("damaged")));
        std::fs::write(d.path().join(FILE_NAME), vec![b' '; MAX_BYTES as usize + 10]).unwrap();
        assert!(matches!(locator(d.path(), None).look(), Found::Guess { note: Some(n), .. } if n.contains("too large")));
        assert!(format!("{:?}", locator(d.path(), None)).contains("Locator"));
    }

    #[test]
    fn this_process_is_alive_and_pid_zero_is_not() {
        assert!(pid_alive(std::process::id()));
        assert!(!pid_alive(0));
        let _ = Locator::system().data_dir;
    }
}
