//! Downloading and installing inference runtimes against a local mock of GitHub releases: the happy
//! path, resume after cancel, checksum and host checks, zip-slip and link attacks, size limits and the
//! disk-space check.

mod common;
use common::{blob, partial, sha256_hex, Mock, Resp};
use debrief_ai::models::{DiskSpace, DownloadOptions, ModelSource, Progress};
use debrief_ai::runtimes::unpack::{unpack, UnpackLimits};
use debrief_ai::runtimes::{GithubReleases, ReleaseConfig, RuntimeCatalog, RuntimeStage, RuntimeStore};
use debrief_ai::AiError;
use std::io::Write;
use std::path::Path;
use std::sync::{Arc, Mutex};
use std::time::Duration;

const SERVER: &str = if cfg!(windows) { "llama-server.exe" } else { "llama-server" };

struct Disk(Option<u64>);
impl DiskSpace for Disk {
    fn available_bytes(&self, _: &Path) -> Option<u64> {
        self.0
    }
}

fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        w.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

/// A .tar.gz with regular files and symbolic links (`(name, None, Some(target))`).
/// `(name, contents, link target)`.
type TarEntry<'a> = (&'a str, Option<&'a [u8]>, Option<&'a str>);

fn tgz_of(entries: &[TarEntry]) -> Vec<u8> {
    let gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    let mut b = tar::Builder::new(gz);
    for (name, data, link) in entries {
        let mut h = tar::Header::new_gnu();
        match (data, link) {
            (Some(d), _) => {
                h.set_size(d.len() as u64);
                h.set_mode(0o755);
                h.set_entry_type(tar::EntryType::Regular);
                b.append_data(&mut h, name, *d).unwrap();
            }
            (None, Some(t)) => {
                h.set_size(0);
                h.set_entry_type(tar::EntryType::Symlink);
                b.append_link(&mut h, name, t).unwrap();
            }
            _ => unreachable!(),
        }
    }
    b.into_inner().unwrap().finish().unwrap()
}

fn catalog(assets: &[(&str, &[u8])]) -> RuntimeCatalog {
    let assets: Vec<String> = assets
        .iter()
        .map(|(name, data)| {
            format!(
                r#"{{"name":"{name}","url":"https://github.com/ggml-org/llama.cpp/releases/download/b7/{name}","size":{},"sha256":"{}"}}"#,
                data.len(),
                sha256_hex(data)
            )
        })
        .collect();
    RuntimeCatalog::parse(&format!(
        r#"{{"version":1,"release":"v9","tag":"b7","platforms":{{"test":[{{"id":"cpu","assets":[{}]}}]}}}}"#,
        assets.join(",")
    ))
    .unwrap()
}

/// GitHub: the release URL redirects to the "CDN", which serves byte ranges.
fn github(files: Vec<(String, Vec<u8>)>, delay_ms: u64) -> Mock {
    Mock::start(move |req, port| {
        let name = req
            .path
            .split('?')
            .next()
            .unwrap_or("")
            .rsplit('/')
            .next()
            .unwrap_or("")
            .to_string();
        let Some((_, data)) = files.iter().find(|(n, _)| *n == name) else {
            return Resp::new(404, "");
        };
        if req.path.starts_with("/ggml-org/") {
            return Resp::redirect(&format!("http://127.0.0.1:{port}/cdn/{name}?sig=secret"));
        }
        let mut r = partial(req, data);
        r.delay_ms = delay_ms;
        r
    })
}

fn source(mock: &Mock, catalog: &RuntimeCatalog) -> GithubReleases {
    GithubReleases::new(
        ReleaseConfig {
            loopback_base: Some(mock.url()),
            chunk_bytes: 4096,
            ..ReleaseConfig::default()
        },
        catalog.clone(),
    )
    .unwrap()
}

fn opts() -> DownloadOptions {
    DownloadOptions {
        max_bytes: 10_000_000,
        margin_bytes: 1000,
        attempts: 2,
        backoff: Duration::from_millis(1),
        ..DownloadOptions::default()
    }
}

fn install(
    store: &RuntimeStore,
    c: &RuntimeCatalog,
    src: &dyn ModelSource,
    disk: &Disk,
    p: &Progress,
) -> (debrief_ai::Result<debrief_ai::runtimes::InstalledRuntime>, Vec<RuntimeStage>) {
    install_checked(store, c, src, disk, p, None)
}

fn install_checked(
    store: &RuntimeStore,
    c: &RuntimeCatalog,
    src: &dyn ModelSource,
    disk: &Disk,
    p: &Progress,
    check: Option<debrief_ai::runtimes::store::RuntimeCheck<'_>>,
) -> (debrief_ai::Result<debrief_ai::runtimes::InstalledRuntime>, Vec<RuntimeStage>) {
    let mut stages = vec![];
    let r = store.install(c, &c.builds("test")[0], src, disk, opts(), p, &mut |s| stages.push(s), check);
    (r, stages)
}

#[test]
fn a_build_that_fails_its_check_never_replaces_the_one_in_use() {
    let old = zip_of(&[(SERVER, b"old build")]);
    let new = zip_of(&[(SERVER, b"new build")]);
    let dir = tempfile::tempdir().unwrap();
    let store = RuntimeStore::new(dir.path());
    let c_old = catalog(&[("old.zip", &old)]);
    let mock_old = github(vec![("old.zip".into(), old.clone())], 0);
    let rt = install(&store, &c_old, &source(&mock_old, &c_old), &Disk(None), &Progress::new())
        .0
        .unwrap();
    // The update of the same build: its check sees the unpacked staging copy and fails.
    let c_new = catalog(&[("new.zip", &new)]);
    let mock_new = github(vec![("new.zip".into(), new.clone())], 0);
    let seen = Mutex::new(None);
    let failing = |staging: &Path| {
        *seen.lock().unwrap() = Some(std::fs::read(staging.join(SERVER)).unwrap());
        Err(AiError::SelfTest("the new CPU inference runtime failed its self-test".into()))
    };
    let (r, stages) = install_checked(
        &store,
        &c_new,
        &source(&mock_new, &c_new),
        &Disk(None),
        &Progress::new(),
        Some(&failing),
    );
    assert!(matches!(r.unwrap_err(), AiError::SelfTest(m) if m.contains("self-test")));
    assert_eq!(seen.lock().unwrap().as_deref(), Some(&b"new build"[..]));
    assert!(stages.ends_with(&[RuntimeStage::Finishing, RuntimeStage::Testing]), "{stages:?}");
    assert_eq!(
        std::fs::read(rt.dir.join(SERVER)).unwrap(),
        b"old build",
        "the previous build stays in use"
    );
    assert!(!dir.path().join("b7/.staging-cpu").exists());
    assert!(
        dir.path().join("downloads/new.zip").is_file(),
        "the verified archive is kept for another try"
    );
    // Passing the check installs it (and the kept archive is not downloaded again).
    let before = mock_new.log().len();
    let passing = |_: &Path| Ok(());
    let (r, stages) = install_checked(
        &store,
        &c_new,
        &source(&mock_new, &c_new),
        &Disk(None),
        &Progress::new(),
        Some(&passing),
    );
    assert_eq!(std::fs::read(r.unwrap().dir.join(SERVER)).unwrap(), b"new build");
    assert!(stages.ends_with(&[RuntimeStage::Finishing, RuntimeStage::Testing]), "{stages:?}");
    assert!(mock_new.log().len() <= before + 1, "{:?}", &mock_new.log()[before..]);
}

#[test]
fn installs_a_zip_build_with_only_the_files_it_needs() {
    let server = blob(20_000);
    let archive = zip_of(&[
        (SERVER, &server),
        ("ggml.dll", b"lib"),
        ("llama-cli.exe", b"other tool"),
        ("../evil.dll", b"zip slip"),
        ("sub/dir/deep.dll", b"too deep"),
        ("LICENSE", b"MIT"),
    ]);
    let cudart = zip_of(&[("cudart64_13.dll", b"cuda runtime")]);
    let c = catalog(&[
        ("llama-b7-bin-win-cpu-x64.zip", &archive),
        ("cudart-llama-bin-win-cuda-13.4-x64.zip", &cudart),
    ]);
    let mock = github(
        vec![
            ("llama-b7-bin-win-cpu-x64.zip".into(), archive.clone()),
            ("cudart-llama-bin-win-cuda-13.4-x64.zip".into(), cudart.clone()),
        ],
        0,
    );
    let dir = tempfile::tempdir().unwrap();
    let store = RuntimeStore::new(dir.path());
    let (r, stages) = install(&store, &c, &source(&mock, &c), &Disk(None), &Progress::new());
    let rt = r.unwrap();
    assert_eq!((rt.id.as_str(), rt.tag.as_str(), rt.current), ("cpu", "b7", true));
    let mut files: Vec<String> = std::fs::read_dir(&rt.dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    files.sort();
    let mut want = vec!["LICENSE", "cudart64_13.dll", "ggml.dll", "runtime.json", SERVER];
    want.sort();
    assert_eq!(files, want);
    assert_eq!(std::fs::read(rt.dir.join(SERVER)).unwrap(), server);
    assert!(!dir.path().join("evil.dll").exists() && !dir.path().join("b7").join("evil.dll").exists());
    assert!(dir.path().join("b7/manifest.json").is_file());
    assert_eq!(
        stages,
        [
            RuntimeStage::Downloading {
                asset: "llama-b7-bin-win-cpu-x64.zip".into(),
                index: 1,
                count: 2
            },
            RuntimeStage::Downloading {
                asset: "cudart-llama-bin-win-cuda-13.4-x64.zip".into(),
                index: 2,
                count: 2
            },
            RuntimeStage::Unpacking {
                asset: "llama-b7-bin-win-cpu-x64.zip".into()
            },
            RuntimeStage::Unpacking {
                asset: "cudart-llama-bin-win-cuda-13.4-x64.zip".into()
            },
            RuntimeStage::Finishing
        ]
    );
    // The archives are gone, the build is listed, and the signed CDN address never shows up in errors.
    assert_eq!(std::fs::read_dir(dir.path().join("downloads")).unwrap().count(), 0);
    assert_eq!(store.list("b7").len(), 1);
    // Installing again replaces it.
    assert!(install(&store, &c, &source(&mock, &c), &Disk(None), &Progress::new()).0.is_ok());
    store.delete("b7", "cpu").unwrap();
    assert!(store.list("b7").is_empty());
}

#[test]
fn a_cancelled_download_resumes() {
    let archive = zip_of(&[(SERVER, &blob(60_000))]);
    let c = catalog(&[("a.zip", &archive)]);
    let mock = github(vec![("a.zip".into(), archive.clone())], 20);
    let dir = tempfile::tempdir().unwrap();
    let store = RuntimeStore::new(dir.path());
    let p = Arc::new(Progress::new());
    let canceller = {
        let p = p.clone();
        std::thread::spawn(move || {
            let deadline = std::time::Instant::now() + Duration::from_secs(20);
            while p.snapshot().bytes < 8192 && std::time::Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(5));
            }
            p.cancel();
        })
    };
    let (r, _) = install(&store, &c, &source(&mock, &c), &Disk(None), &p);
    canceller.join().unwrap();
    assert_eq!(r.unwrap_err(), AiError::Cancelled);
    assert!(dir.path().join("downloads/a.zip.part").is_file(), "the partial download is kept");
    assert!(store.list("b7").is_empty());
    let before = mock.log().len();
    let (r, _) = install(&store, &c, &source(&mock, &c), &Disk(None), &Progress::new());
    r.unwrap();
    let resumed = mock.log()[before..].iter().filter_map(common::range_of).any(|(a, _)| a > 0);
    assert!(resumed, "the second run continued where the first stopped");
}

#[test]
fn a_corrupt_download_is_refused_and_nothing_is_installed() {
    let archive = zip_of(&[(SERVER, b"x")]);
    let c = catalog(&[("a.zip", &archive)]);
    let mut tampered = archive.clone();
    let last = tampered.len() - 1;
    tampered[last] ^= 0xff;
    let mock = github(vec![("a.zip".into(), tampered)], 0);
    let dir = tempfile::tempdir().unwrap();
    let store = RuntimeStore::new(dir.path());
    let e = install(&store, &c, &source(&mock, &c), &Disk(None), &Progress::new())
        .0
        .unwrap_err();
    assert!(matches!(e, AiError::Integrity(_)), "{e:?}");
    assert!(store.list("b7").is_empty());
    assert!(!dir.path().join("b7").join("cpu").exists());
}

#[test]
fn only_github_hosts_and_catalog_assets_are_fetched() {
    let archive = zip_of(&[(SERVER, b"x")]);
    let c = catalog(&[("a.zip", &archive)]);
    let evil = Mock::start(|_, _| Resp::redirect("https://evil.example/a.zip"));
    let dir = tempfile::tempdir().unwrap();
    let store = RuntimeStore::new(dir.path());
    let e = install(&store, &c, &source(&evil, &c), &Disk(None), &Progress::new())
        .0
        .unwrap_err();
    assert!(matches!(&e, AiError::Refused(m) if m.contains("evil.example")), "{e:?}");
    let src = source(&evil, &c);
    assert!(matches!(src.describe("b7", "other.zip"), Err(AiError::Refused(_))));
    assert!(matches!(src.describe("b6", "a.zip"), Err(AiError::Refused(_))));
    let d = src.describe("b7", "a.zip").unwrap();
    assert_eq!((d.size, d.sha256.len()), (archive.len() as u64, 64));
}

#[test]
fn a_missing_server_or_too_little_space_is_explained() {
    let archive = zip_of(&[("ggml.dll", b"x")]);
    let c = catalog(&[("a.zip", &archive)]);
    let mock = github(vec![("a.zip".into(), archive.clone())], 0);
    let dir = tempfile::tempdir().unwrap();
    let store = RuntimeStore::new(dir.path());
    let e = install(&store, &c, &source(&mock, &c), &Disk(None), &Progress::new())
        .0
        .unwrap_err();
    assert!(matches!(&e, AiError::Integrity(m) if m.contains("llama-server")), "{e:?}");
    assert!(!dir.path().join("b7/.staging-cpu").exists(), "the staging folder is cleaned up");
    let e = install(&store, &c, &source(&mock, &c), &Disk(Some(10)), &Progress::new())
        .0
        .unwrap_err();
    assert!(matches!(e, AiError::Disk(_)));
}

#[test]
fn tar_links_must_stay_in_the_folder_and_sizes_are_capped() {
    let dir = tempfile::tempdir().unwrap();
    let good = dir.path().join("good.tar.gz");
    std::fs::write(
        &good,
        tgz_of(&[
            ("llama-b7/llama-server", Some(b"srv"), None),
            ("llama-b7/libllama.so.0.7.0", Some(b"lib"), None),
            ("llama-b7/libllama.so.0", None, Some("libllama.so.0.7.0")),
            ("llama-b7/libllama.so", None, Some("libllama.so.0")),
            ("llama-b7/llama-bench", Some(b"tool"), None),
        ]),
    )
    .unwrap();
    let out = tempfile::tempdir().unwrap();
    let u = unpack(&good, out.path(), &UnpackLimits::default()).unwrap();
    let mut files = u.files.clone();
    files.sort();
    assert_eq!(files, ["libllama.so", "libllama.so.0", "libllama.so.0.7.0", "llama-server"]);
    assert_eq!(std::fs::read(out.path().join("libllama.so")).unwrap(), b"lib");

    let evil = dir.path().join("evil.tar.gz");
    std::fs::write(&evil, tgz_of(&[("x/libz.so", None, Some("/etc/passwd"))])).unwrap();
    let e = unpack(&evil, tempfile::tempdir().unwrap().path(), &UnpackLimits::default()).unwrap_err();
    assert!(matches!(&e, AiError::Integrity(m) if m.contains("links outside")), "{e:?}");
    let up = dir.path().join("up.tar.gz");
    std::fs::write(&up, tgz_of(&[("x/libz.so", None, Some("../../libz.so"))])).unwrap();
    assert!(unpack(&up, tempfile::tempdir().unwrap().path(), &UnpackLimits::default()).is_err());

    let small = UnpackLimits {
        max_file_bytes: 2,
        ..UnpackLimits::default()
    };
    assert!(matches!(unpack(&good, tempfile::tempdir().unwrap().path(), &small), Err(AiError::Integrity(m)) if m.contains("too large")));
    let few = UnpackLimits {
        max_entries: 2,
        ..UnpackLimits::default()
    };
    assert!(unpack(&good, tempfile::tempdir().unwrap().path(), &few).is_err());
    let total = UnpackLimits {
        max_total_bytes: 4,
        ..UnpackLimits::default()
    };
    assert!(unpack(&good, tempfile::tempdir().unwrap().path(), &total).is_err());
    let other = dir.path().join("x.rar");
    std::fs::write(&other, b"x").unwrap();
    assert!(unpack(&other, out.path(), &UnpackLimits::default()).is_err());
    let broken = dir.path().join("broken.zip");
    std::fs::write(&broken, b"PK not really").unwrap();
    assert!(unpack(&broken, out.path(), &UnpackLimits::default()).is_err());
}

#[test]
fn one_install_per_build_at_a_time() {
    let archive = zip_of(&[(SERVER, &blob(40_000))]);
    let c = catalog(&[("a.zip", &archive)]);
    let mock = github(vec![("a.zip".into(), archive.clone())], 30);
    let dir = tempfile::tempdir().unwrap();
    let store = Arc::new(RuntimeStore::new(dir.path()));
    let src = Arc::new(source(&mock, &c));
    let errors = Arc::new(Mutex::new(vec![]));
    let threads: Vec<_> = (0..2)
        .map(|_| {
            let (store, src, c, errors) = (store.clone(), src.clone(), c.clone(), errors.clone());
            std::thread::spawn(move || {
                if let Err(e) = install(&store, &c, &*src, &Disk(None), &Progress::new()).0 {
                    errors.lock().unwrap().push(e);
                }
            })
        })
        .collect();
    std::thread::sleep(Duration::from_millis(100));
    assert!(store.delete("b7", "cpu").is_err());
    for t in threads {
        t.join().unwrap();
    }
    let errors = errors.lock().unwrap();
    assert_eq!(errors.len(), 1, "{errors:?}");
    assert!(matches!(&errors[0], AiError::Refused(m) if m.contains("already")));
    assert_eq!(store.list("b7").len(), 1);
}
