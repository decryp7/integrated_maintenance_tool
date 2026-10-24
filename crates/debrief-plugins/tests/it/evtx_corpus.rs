//! Real-world EVTX files through the Windows plugin's importer, over the real IPC: the EVTX-ATTACK-SAMPLES corpus
//! (<https://github.com/sbousseaden/EVTX-ATTACK-SAMPLES>, GPL-3.0, about 280 event logs recorded while running
//! attack techniques). The corpus is downloaded at test time, at a pinned commit and checked against a pinned
//! SHA-256, into a temporary folder; it is never committed or redistributed.
//!
//! Off by default (it needs the network): the test does nothing unless `DEBRIEF_EVTX_CORPUS=1` is set, which the
//! nightly `corpus` workflow does. `DEBRIEF_CORPUS_CACHE` names a folder that keeps the verified archive between runs
//! (the workflow caches it), so a night without a change downloads nothing:
//!
//! ```sh
//! DEBRIEF_EVTX_CORPUS=1 cargo test -p debrief-plugins --test it evtx_corpus::
//! ```
//!
//! The corpus is licensed GPL-3.0 (`LICENSE.GPL` in the repository): it is fetched for the test run and never
//! committed, bundled or redistributed.
//!
//! Every file must import without the plugin crashing or being stopped at its memory cap, give events, and
//! every event must have a timestamp. Failures name the file and what went wrong, never event content (the
//! corpus holds real command lines and account names).

use crate::common;

use debrief_core::plugin::PluginStatus;
use debrief_core::{Profile, Workspace};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

const COMMIT: &str = "4ceed2f4706daf601c212a8f91c113dd85349a2c";
const ARCHIVE_SHA256: &str = "ea8bf4cd30f673907f2ad0ca2ccff501bac55fd28573ee77c0940d9b85396da5";
/// The archive is about 6 MB and expands to about 50 MB (the largest log is about 10 MB).
const MAX_ARCHIVE: u64 = 64 << 20;
const MAX_ENTRY: u64 = 64 << 20;
const MAX_EXPANDED: u64 = 512 << 20;
/// At least this many logs, so a corpus that silently shrank (or a broken extraction) fails.
const MIN_FILES: usize = 250;

struct TempDir(PathBuf);
impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// The archive: from the cache folder when one is set and holds a copy that still matches the pinned SHA-256,
/// else downloaded (and then stored there).
fn archive() -> Vec<u8> {
    let cached = std::env::var_os("DEBRIEF_CORPUS_CACHE").map(|d| PathBuf::from(d).join(format!("evtx-attack-samples-{COMMIT}.zip")));
    if let Some(bytes) = cached.as_ref().and_then(|p| std::fs::read(p).ok()) {
        if bytes.len() as u64 <= MAX_ARCHIVE && hex(&Sha256::digest(&bytes)) == ARCHIVE_SHA256 {
            return bytes;
        }
    }
    let bytes = download();
    if let Some(path) = cached {
        if let Some(dir) = path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&path, &bytes);
    }
    bytes
}

fn download() -> Vec<u8> {
    let url = format!("https://codeload.github.com/sbousseaden/EVTX-ATTACK-SAMPLES/zip/{COMMIT}");
    // The OS trust store (as `cargo xtask libduckdb`), so TLS-inspecting proxies work.
    let tls = ureq::tls::TlsConfig::builder()
        .root_certs(ureq::tls::RootCerts::PlatformVerifier)
        .build();
    let agent: ureq::Agent = ureq::Agent::config_builder().tls_config(tls).build().into();
    let mut bytes = Vec::new();
    agent
        .get(&url)
        .call()
        .unwrap_or_else(|e| panic!("downloading {url}: {e}"))
        .into_body()
        .into_reader()
        .take(MAX_ARCHIVE + 1)
        .read_to_end(&mut bytes)
        .unwrap_or_else(|e| panic!("reading {url}: {e}"));
    assert!(bytes.len() as u64 <= MAX_ARCHIVE, "{url} is larger than {MAX_ARCHIVE} bytes");
    let sum = hex(&Sha256::digest(&bytes));
    assert_eq!(
        sum, ARCHIVE_SHA256,
        "the archive of {COMMIT} changed (sha256 {sum}); check it and update ARCHIVE_SHA256"
    );
    bytes
}

fn hex(b: &[u8]) -> String {
    b.iter().map(|x| format!("{x:02x}")).collect()
}

/// Writes the archive's `.evtx` files under `dir` (flattened names, so no path from the archive is trusted) and
/// returns them with their path in the corpus.
fn extract(archive: &[u8], dir: &Path) -> Vec<(String, PathBuf)> {
    let mut zip = zip::ZipArchive::new(std::io::Cursor::new(archive)).expect("the archive is a zip");
    let mut out = Vec::new();
    let mut expanded = 0u64;
    for i in 0..zip.len() {
        let mut entry = zip.by_index(i).unwrap();
        let Some(name) = entry.enclosed_name() else { continue };
        if !entry.is_file() || !name.extension().is_some_and(|e| e.eq_ignore_ascii_case("evtx")) {
            continue;
        }
        // The path inside the repository, without the archive's top folder.
        let corpus: String = name
            .components()
            .skip(1)
            .map(|c| c.as_os_str().to_string_lossy())
            .collect::<Vec<_>>()
            .join("/");
        assert!(entry.size() <= MAX_ENTRY, "{corpus}: {} bytes, over the limit", entry.size());
        expanded += entry.size();
        assert!(expanded <= MAX_EXPANDED, "the archive expands past {MAX_EXPANDED} bytes");
        let mut bytes = Vec::with_capacity(entry.size() as usize);
        entry.by_ref().take(MAX_ENTRY + 1).read_to_end(&mut bytes).unwrap();
        let path = dir.join(format!("{:04}.evtx", out.len()));
        std::fs::write(&path, bytes).unwrap();
        out.push((corpus, path));
    }
    out
}

/// What went wrong with one file, without anything from inside it.
fn check(path: &Path) -> Option<String> {
    let mut ws = Workspace::with_registry(Profile::default_profile(), common::registry(&[common::WINDOWS])).unwrap();
    if ws.open(&[path.to_path_buf()]).is_err() {
        return Some("open failed".into());
    }
    let plugin = ws
        .plugins()
        .into_iter()
        .find(|p| p.id == common::WINDOWS)
        .expect("the Windows plugin");
    if plugin.status != PluginStatus::Active || plugin.failures > 0 {
        let cap = plugin.last_error.as_deref().is_some_and(|e| e.contains("memory cap"));
        return Some(
            if cap {
                "plugin stopped at its memory cap"
            } else {
                "plugin crashed"
            }
            .into(),
        );
    }
    let s = ws.summary();
    if !s.unrecognized.is_empty() {
        return Some("not imported".into());
    }
    let Some(src) = s.sources.first() else {
        return Some("no source".into());
    };
    if src.importer != "evtx" {
        return Some(format!("read by the {} importer", src.importer));
    }
    if src.records == 0 {
        return Some("no events".into());
    }
    let untimed = ws.records().iter().filter(|r| r.ts.is_none()).count();
    (untimed > 0).then(|| format!("{untimed} of {} events without a timestamp", src.records))
}

#[test]
fn every_real_world_evtx_file_imports() {
    if std::env::var_os("DEBRIEF_EVTX_CORPUS").is_none() {
        eprintln!("skipped: set DEBRIEF_EVTX_CORPUS=1 to download the EVTX-ATTACK-SAMPLES corpus and import it");
        return;
    }
    let dir = TempDir(std::env::temp_dir().join(format!("debrief-evtx-corpus-{}", std::process::id())));
    let _ = std::fs::remove_dir_all(&dir.0);
    std::fs::create_dir_all(&dir.0).unwrap();
    let files = extract(&archive(), &dir.0);
    assert!(files.len() >= MIN_FILES, "only {} .evtx files in the archive", files.len());
    let failures: Vec<String> = files
        .iter()
        .filter_map(|(name, path)| check(path).map(|why| format!("{name}: {why}")))
        .collect();
    assert!(
        failures.is_empty(),
        "{} of {} files failed:\n{}",
        failures.len(),
        files.len(),
        failures.join("\n")
    );
}
