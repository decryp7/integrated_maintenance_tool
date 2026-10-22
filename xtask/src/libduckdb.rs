//! `cargo xtask libduckdb`: fetches the official prebuilt libduckdb the store worker links and points Cargo at
//! it, so plain `cargo build` / `cargo test` work afterwards on Windows, macOS and Linux without a shell.
//!
//! - The pins (version, release assets, SHA-256) stay in one place, `crates/debrief-store-duck/scripts/fetch-libduckdb.sh`,
//!   which CI runs directly; they are parsed from it here as `app/scripts/libduckdb.mjs` does.
//! - The archive is checked against its pin before it is unpacked into a fresh `<target>/libduckdb/<version>/<asset>/`.
//! - `DUCKDB_LIB_DIR` and `DUCKDB_INCLUDE_DIR` go into `.cargo/libduckdb.toml`, a machine-local file that this task
//!   owns entirely (git-ignored), which the committed `.cargo/config.toml` includes when it exists. The values are
//!   relative to the workspace (`relative = true`) when the target folder is inside it, absolute otherwise. They are
//!   plain `[env]` defaults: a `DUCKDB_LIB_DIR` set in the environment (CI's `eval "$(fetch-libduckdb.sh)"`) wins.
//! - Run time: libduckdb-sys passes `DUCKDB_LIB_DIR` to the linker as a search path, and Cargo puts the search paths
//!   inside the target folder on the library path (`PATH`, `LD_LIBRARY_PATH`, `DYLD_FALLBACK_LIBRARY_PATH`) of
//!   everything `cargo run` / `cargo test` starts. That is why the library is unpacked under the target folder: test
//!   binaries find `duckdb.dll` / `libduckdb.so` / `libduckdb.dylib` with nothing copied next to them.

use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Component, Path, PathBuf};

/// The file this task writes, next to the committed `config.toml`.
pub const LOCAL_CONFIG: &str = "libduckdb.toml";

/// Largest archive accepted (the official ones are about 20–40 MB).
const MAX_ARCHIVE: u64 = 512 * 1024 * 1024;

/// One pinned release asset.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pin {
    pub asset: String,
    pub sha256: String,
}

/// The pins of `fetch-libduckdb.sh`: its `VERSION=` and every `ASSET=…; SHA=…` line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Pins {
    pub version: String,
    pub assets: Vec<Pin>,
}

impl Pins {
    /// Parses the pins out of the fetch script's text.
    pub fn parse(script: &str) -> Result<Pins, String> {
        let version = script
            .lines()
            .find_map(|l| l.strip_prefix("VERSION="))
            .map(str::trim)
            .filter(|v| !v.is_empty() && v.chars().all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-'))
            .ok_or("fetch-libduckdb.sh names no VERSION")?
            .to_owned();
        let assets: Vec<Pin> = script.lines().filter_map(parse_pin_line).collect();
        if assets.is_empty() {
            return Err("fetch-libduckdb.sh pins no assets".into());
        }
        Ok(Pins { version, assets })
    }

    /// The pinned asset for a Rust `std::env::consts::OS` and `ARCH`, or an error naming what is pinned.
    pub fn asset_for(&self, os: &str, arch: &str) -> Result<&Pin, String> {
        let cpu = match arch {
            "x86_64" => Some("amd64"),
            "aarch64" => Some("arm64"),
            _ => None,
        };
        let name = match (os, cpu) {
            ("macos", _) => Some("libduckdb-osx-universal.zip".to_owned()),
            ("windows", Some(cpu)) => Some(format!("libduckdb-windows-{cpu}.zip")),
            ("linux", Some(cpu)) => Some(format!("libduckdb-linux-{cpu}.zip")),
            _ => None,
        };
        name.and_then(|n| self.assets.iter().find(|p| p.asset == n)).ok_or_else(|| {
            let pinned: Vec<&str> = self.assets.iter().map(|p| p.asset.as_str()).collect();
            format!("no pinned libduckdb for {os}-{arch} (pinned: {})", pinned.join(", "))
        })
    }

    /// The release download URL of `pin`.
    pub fn url(&self, pin: &Pin) -> String {
        format!("https://github.com/duckdb/duckdb/releases/download/v{}/{}", self.version, pin.asset)
    }
}

/// `ASSET=libduckdb-….zip; SHA=<64 hex>` anywhere in a line.
fn parse_pin_line(line: &str) -> Option<Pin> {
    let rest = &line[line.find("ASSET=")? + "ASSET=".len()..];
    let (asset, rest) = rest.split_once("; SHA=")?;
    let sha: String = rest.chars().take_while(char::is_ascii_hexdigit).collect();
    let valid_asset = asset.starts_with("libduckdb-")
        && asset.ends_with(".zip")
        && asset.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'));
    (valid_asset && sha.len() == 64).then(|| Pin {
        asset: asset.to_owned(),
        sha256: sha.to_ascii_lowercase(),
    })
}

/// Lower-case hex SHA-256 of `bytes`.
pub fn sha256_hex(bytes: &[u8]) -> String {
    Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// Checks `bytes` against the pin, before anything is written or unpacked.
pub fn verify(pin: &Pin, bytes: &[u8]) -> Result<(), String> {
    let got = sha256_hex(bytes);
    if got == pin.sha256 {
        Ok(())
    } else {
        Err(format!("SHA-256 mismatch for {}: expected {}, got {got}", pin.asset, pin.sha256))
    }
}

/// The folder an asset unpacks into: `<target>/libduckdb/<version>/<asset without .zip>`.
pub fn unpack_dir(target_dir: &Path, version: &str, pin: &Pin) -> PathBuf {
    let stem = pin.asset.strip_suffix(".zip").unwrap_or(&pin.asset);
    target_dir.join("libduckdb").join(version).join(stem)
}

/// The target folder Cargo builds into: `CARGO_TARGET_DIR` (relative to the workspace when relative), else
/// `<workspace>/target`. A `build.target-dir` in a Cargo config is not read; set `CARGO_TARGET_DIR` instead.
pub fn target_dir(root: &Path, env_target: Option<&str>) -> PathBuf {
    match env_target.filter(|t| !t.is_empty()) {
        Some(t) => root.join(t),
        None => root.join("target"),
    }
}

/// How the config names a folder: relative to the workspace when it lies inside it (portable, no machine path),
/// absolute otherwise. Separators are always `/`, which Cargo and the linkers accept on Windows too.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ConfigPath {
    Relative(String),
    Absolute(String),
}

impl ConfigPath {
    pub fn new(root: &Path, dir: &Path) -> ConfigPath {
        match dir.strip_prefix(root) {
            Ok(rel) if rel.components().all(|c| matches!(c, Component::Normal(_))) => ConfigPath::Relative(slashes(rel)),
            _ => ConfigPath::Absolute(slashes(dir)),
        }
    }

    /// The TOML inline table for an `[env]` entry.
    fn toml(&self) -> String {
        match self {
            ConfigPath::Relative(p) => format!("{{ value = {}, relative = true }}", toml_string(p)),
            ConfigPath::Absolute(p) => format!("{{ value = {} }}", toml_string(p)),
        }
    }
}

fn slashes(p: &Path) -> String {
    p.to_string_lossy().replace('\\', "/")
}

fn toml_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if c.is_control() => out.push_str(&format!("\\u{:04X}", u32::from(c))),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// The whole of `.cargo/libduckdb.toml`. The task owns the file and rewrites it on every run.
pub fn render_local_config(version: &str, asset: &str, lib_dir: &ConfigPath) -> String {
    let value = lib_dir.toml();
    format!(
        "# Written by `cargo xtask libduckdb` (libduckdb {version}, {asset}); rerun it to update, delete the file to\n\
         # stop using it. Machine-local: git-ignored, included by .cargo/config.toml. A DUCKDB_LIB_DIR set in the\n\
         # environment takes precedence over these defaults.\n\
         [env]\n\
         DUCKDB_LIB_DIR = {value}\n\
         DUCKDB_INCLUDE_DIR = {value}\n"
    )
}

/// Whether the committed `.cargo/config.toml` includes the local file (optionally), so Cargo reads it.
pub fn config_includes_local(config: &str) -> bool {
    let Ok(table) = config.parse::<toml::Table>() else {
        return false;
    };
    let Some(includes) = table.get("include").and_then(toml::Value::as_array) else {
        return false;
    };
    includes.iter().any(|i| match i {
        toml::Value::String(p) => p == LOCAL_CONFIG,
        toml::Value::Table(t) => t.get("path").and_then(toml::Value::as_str) == Some(LOCAL_CONFIG),
        _ => false,
    })
}

/// Unpacks a verified archive into `dir`, which is emptied first. Entries that would escape `dir` are refused by
/// the zip crate.
pub fn unpack(bytes: &[u8], dir: &Path) -> Result<(), String> {
    if dir.exists() {
        std::fs::remove_dir_all(dir).map_err(|e| format!("could not empty {}: {e}", dir.display()))?;
    }
    std::fs::create_dir_all(dir).map_err(|e| format!("could not create {}: {e}", dir.display()))?;
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(bytes)).map_err(|e| format!("not a zip archive: {e}"))?;
    zip.extract(dir)
        .map_err(|e| format!("could not unpack into {}: {e}", dir.display()))
}

fn download(url: &str) -> Result<Vec<u8>, String> {
    eprintln!("xtask: fetching {url}");
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder().tls_config(tls).build().into();
    let mut body = Vec::new();
    agent
        .get(url)
        .call()
        .map_err(|e| format!("downloading {url} failed: {e}"))?
        .into_body()
        .into_reader()
        .take(MAX_ARCHIVE + 1)
        .read_to_end(&mut body)
        .map_err(|e| format!("downloading {url} failed: {e}"))?;
    if body.len() as u64 > MAX_ARCHIVE {
        return Err(format!("{url} is larger than {MAX_ARCHIVE} bytes"));
    }
    Ok(body)
}

/// Fetches (when not cached), verifies and unpacks the host's pinned libduckdb, then writes `.cargo/libduckdb.toml`.
pub fn run(root: &Path) -> Result<(), String> {
    let script_path = root
        .join("crates")
        .join("debrief-store-duck")
        .join("scripts")
        .join("fetch-libduckdb.sh");
    let script = std::fs::read_to_string(&script_path).map_err(|e| format!("could not read {}: {e}", script_path.display()))?;
    let pins = Pins::parse(&script)?;
    let pin = pins.asset_for(std::env::consts::OS, std::env::consts::ARCH)?;
    let env_target = std::env::var("CARGO_TARGET_DIR").ok();
    let target = target_dir(root, env_target.as_deref());
    let dir = unpack_dir(&target, &pins.version, pin);
    let marker = dir.join(".verified");
    if marker.is_file() {
        eprintln!("xtask: libduckdb {} already in {}", pins.version, dir.display());
    } else {
        let bytes = download(&pins.url(pin))?;
        verify(pin, &bytes)?;
        unpack(&bytes, &dir)?;
        std::fs::write(&marker, b"").map_err(|e| format!("could not write {}: {e}", marker.display()))?;
        eprintln!("xtask: libduckdb {} unpacked into {}", pins.version, dir.display());
    }

    let cargo_dir = root.join(".cargo");
    let config = std::fs::read_to_string(cargo_dir.join("config.toml")).unwrap_or_default();
    if !config_includes_local(&config) {
        return Err(format!(
            ".cargo/config.toml does not include {LOCAL_CONFIG}; add `include = [{{ path = \"{LOCAL_CONFIG}\", optional = true }}]` at its top"
        ));
    }
    let local = cargo_dir.join(LOCAL_CONFIG);
    let text = render_local_config(&pins.version, &pin.asset, &ConfigPath::new(root, &dir));
    std::fs::write(&local, text).map_err(|e| format!("could not write {}: {e}", local.display()))?;
    eprintln!(
        "xtask: wrote {}; `cargo build` and `cargo test` now link this libduckdb",
        local.display()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCRIPT: &str = "set -eu\n\nVERSION=1.5.6\nCACHE=x\n\
        case x in\n  MINGW*-x86_64)\n    ASSET=libduckdb-windows-amd64.zip; SHA=44cf59583f9951d2cb09b1bf115a63ecb2d8901e363903029d86c7d8683fe96a ;;\n\
          Linux-aarch64)\n    ASSET=libduckdb-linux-arm64.zip; SHA=B72ED9F05003F5E9D2015F7CEADA6416B377D9DD33169CDDE3C9E33897856EEE ;;\n\
          Darwin-*)\n    ASSET=libduckdb-osx-universal.zip; SHA=e0bc007d9b0094c0970ac1847a8601d10aad07cbd2910ce586ec810f77b638d6 ;;\n\
          Bad)\n    ASSET=libduckdb-x.zip; SHA=abc ;;\n    ASSET=../evil.zip; SHA=e0bc007d9b0094c0970ac1847a8601d10aad07cbd2910ce586ec810f77b638d6 ;;\nesac\n\
        DIR=\"$CACHE/$VERSION/${ASSET%.zip}\"\n";

    #[test]
    fn parses_the_version_and_every_well_formed_pin() {
        let pins = Pins::parse(SCRIPT).unwrap();
        assert_eq!(pins.version, "1.5.6");
        let names: Vec<&str> = pins.assets.iter().map(|p| p.asset.as_str()).collect();
        assert_eq!(
            names,
            [
                "libduckdb-windows-amd64.zip",
                "libduckdb-linux-arm64.zip",
                "libduckdb-osx-universal.zip"
            ]
        );
        assert_eq!(
            pins.assets[1].sha256,
            "b72ed9f05003f5e9d2015f7ceada6416b377d9dd33169cdde3c9e33897856eee"
        );
        assert_eq!(
            pins.url(&pins.assets[0]),
            "https://github.com/duckdb/duckdb/releases/download/v1.5.6/libduckdb-windows-amd64.zip"
        );
    }

    #[test]
    fn a_script_without_version_or_pins_is_refused() {
        assert_eq!(
            Pins::parse("ASSET=libduckdb-a.zip; SHA=00").unwrap_err(),
            "fetch-libduckdb.sh names no VERSION"
        );
        assert_eq!(Pins::parse("VERSION=1.0\n").unwrap_err(), "fetch-libduckdb.sh pins no assets");
        assert!(Pins::parse("VERSION=1.0;rm -rf /\nASSET=libduckdb-a.zip; SHA=0").is_err());
    }

    #[test]
    fn the_committed_script_pins_every_supported_host() {
        let script = std::fs::read_to_string(crate::repo_root().join("crates/debrief-store-duck/scripts/fetch-libduckdb.sh")).unwrap();
        let pins = Pins::parse(&script).unwrap();
        for (os, arch) in [
            ("windows", "x86_64"),
            ("windows", "aarch64"),
            ("linux", "x86_64"),
            ("linux", "aarch64"),
            ("macos", "x86_64"),
            ("macos", "aarch64"),
        ] {
            assert!(pins.asset_for(os, arch).is_ok(), "{os}-{arch}");
        }
    }

    #[test]
    fn picks_the_asset_for_the_host() {
        let pins = Pins::parse(SCRIPT).unwrap();
        assert_eq!(pins.asset_for("windows", "x86_64").unwrap().asset, "libduckdb-windows-amd64.zip");
        assert_eq!(pins.asset_for("macos", "aarch64").unwrap().asset, "libduckdb-osx-universal.zip");
        assert_eq!(pins.asset_for("macos", "x86_64").unwrap().asset, "libduckdb-osx-universal.zip");
        let err = pins.asset_for("linux", "x86_64").unwrap_err();
        assert!(
            err.starts_with("no pinned libduckdb for linux-x86_64 (pinned: libduckdb-windows-amd64.zip"),
            "{err}"
        );
        assert!(pins.asset_for("freebsd", "x86_64").is_err());
        assert!(pins.asset_for("windows", "riscv64").is_err());
    }

    #[test]
    fn verifies_the_archive_against_its_pin() {
        let pin = Pin {
            asset: "libduckdb-a.zip".into(),
            sha256: sha256_hex(b"archive"),
        };
        assert!(verify(&pin, b"archive").is_ok());
        let err = verify(&pin, b"tampered").unwrap_err();
        assert!(err.starts_with("SHA-256 mismatch for libduckdb-a.zip: expected "), "{err}");
        assert_eq!(sha256_hex(b""), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    }

    #[test]
    fn the_unpack_folder_is_versioned_per_asset_under_the_target() {
        let pin = Pin {
            asset: "libduckdb-linux-amd64.zip".into(),
            sha256: String::new(),
        };
        let dir = unpack_dir(Path::new("t"), "1.5.6", &pin);
        assert_eq!(dir, Path::new("t").join("libduckdb").join("1.5.6").join("libduckdb-linux-amd64"));
        let root = Path::new("repo");
        assert_eq!(target_dir(root, None), root.join("target"));
        assert_eq!(target_dir(root, Some("")), root.join("target"));
        assert_eq!(target_dir(root, Some("other")), root.join("other"));
    }

    #[test]
    fn config_paths_are_workspace_relative_when_inside_it() {
        let root = std::env::temp_dir().join("repo");
        let inside = root.join("target").join("libduckdb").join("1.5.6").join("libduckdb-windows-amd64");
        assert_eq!(
            ConfigPath::new(&root, &inside),
            ConfigPath::Relative("target/libduckdb/1.5.6/libduckdb-windows-amd64".into())
        );
        let outside = std::env::temp_dir().join("elsewhere").join("libduckdb");
        assert!(
            matches!(ConfigPath::new(&root, &outside), ConfigPath::Absolute(p) if !p.contains('\\') && p.ends_with("elsewhere/libduckdb"))
        );
        // A `..` inside the joined path would leave the workspace: named absolutely.
        let escaping = root.join("..").join("x");
        assert!(matches!(ConfigPath::new(&root, &escaping), ConfigPath::Absolute(_)));
    }

    #[test]
    fn the_local_config_is_valid_toml_with_both_variables() {
        for path in [
            ConfigPath::Relative("target/libduckdb/1.5.6/a".into()),
            ConfigPath::Absolute("C:/Users/x \"y\"/a\\b".into()),
        ] {
            let text = render_local_config("1.5.6", "libduckdb-windows-amd64.zip", &path);
            let table: toml::Table = text.parse().unwrap();
            let env = table["env"].as_table().unwrap();
            for key in ["DUCKDB_LIB_DIR", "DUCKDB_INCLUDE_DIR"] {
                let entry = env[key].as_table().unwrap();
                match &path {
                    ConfigPath::Relative(p) => {
                        assert_eq!(entry["value"].as_str(), Some(p.as_str()));
                        assert_eq!(entry["relative"].as_bool(), Some(true));
                    }
                    ConfigPath::Absolute(p) => {
                        assert_eq!(entry["value"].as_str(), Some(p.as_str()));
                        assert!(!entry.contains_key("relative"));
                    }
                }
            }
        }
    }

    #[test]
    fn recognises_the_include_of_the_local_config() {
        assert!(config_includes_local(
            "include = [{ path = \"libduckdb.toml\", optional = true }]\n[alias]\nx = \"y\"\n"
        ));
        assert!(config_includes_local("include = [\"other.toml\", \"libduckdb.toml\"]\n"));
        assert!(!config_includes_local("include = [\"other.toml\"]\n"));
        assert!(!config_includes_local("[alias]\nxtask = \"run -p xtask --\"\n"));
        assert!(!config_includes_local("not toml ["));
        assert!(!config_includes_local(""));
    }

    #[test]
    fn the_committed_cargo_config_includes_the_local_file() {
        let config = std::fs::read_to_string(crate::repo_root().join(".cargo").join("config.toml")).unwrap();
        assert!(config_includes_local(&config));
    }

    #[test]
    fn unpacks_a_zip_into_a_fresh_folder() {
        let mut buf = std::io::Cursor::new(Vec::new());
        {
            let mut w = zip::ZipWriter::new(&mut buf);
            w.start_file("duckdb.h", zip::write::SimpleFileOptions::default()).unwrap();
            std::io::Write::write_all(&mut w, b"/* header */").unwrap();
            w.finish().unwrap();
        }
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("lib");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("stale.dll"), b"old").unwrap();
        unpack(buf.get_ref(), &dir).unwrap();
        assert_eq!(std::fs::read(dir.join("duckdb.h")).unwrap(), b"/* header */");
        assert!(!dir.join("stale.dll").exists());
        assert!(unpack(b"not a zip", &dir).unwrap_err().starts_with("not a zip archive"));
    }
}
