//! The Hugging Face adapter and the downloader against a local mock server.

mod common;
use common::*;
use debrief_ai::models::*;
use debrief_ai::AiError;
use serde_json::json;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;

const REPO: &str = "org/repo";
const FILE: &str = "m.gguf";

struct Opts {
    sha: Option<String>,
    resolve_status: Option<u16>,
    redirect: Option<String>,
    /// Chunk requests answered with this status before behaving.
    fail_chunks: usize,
    fail_status: u16,
    cut_first: bool,
    on_chunk: Option<Arc<dyn Fn(usize) + Send + Sync>>,
}

impl Default for Opts {
    fn default() -> Self {
        Opts {
            sha: None,
            resolve_status: None,
            redirect: None,
            fail_chunks: 0,
            fail_status: 503,
            cut_first: false,
            on_chunk: None,
        }
    }
}

fn server(data: Arc<Vec<u8>>, o: Opts) -> Mock {
    let chunks = AtomicUsize::new(0);
    let sha = o.sha.clone().unwrap_or_else(|| sha256_hex(&data));
    Mock::start(move |req, port| {
        let p = req.path.as_str();
        if p == format!("/api/models/{REPO}") {
            Resp::json(200, json!({"id": REPO, "gated": false, "cardData": {"license": "apache-2.0"}}))
        } else if p == format!("/api/models/{REPO}/paths-info/main") {
            let want = String::from_utf8_lossy(&req.body).to_string();
            if want.contains(FILE) {
                Resp::json(
                    200,
                    json!([{"type":"file","path":FILE,"size":data.len(),"lfs":{"oid":sha,"size":data.len()}}]),
                )
            } else {
                Resp::json(200, json!([]))
            }
        } else if p == format!("/{REPO}/resolve/main/{FILE}") {
            if let Some(s) = o.resolve_status {
                return Resp::new(s, "denied");
            }
            Resp::redirect(&o.redirect.clone().unwrap_or_else(|| format!("http://localhost:{port}/cdn/{FILE}")))
        } else if p.starts_with("/cdn/") {
            let n = chunks.fetch_add(1, Ordering::SeqCst) + 1;
            if let Some(f) = &o.on_chunk {
                f(n);
            }
            if n <= o.fail_chunks {
                return Resp::new(o.fail_status, "busy");
            }
            let mut r = partial(req, &data);
            if o.cut_first && n == 1 {
                r.cut_after = Some(r.body.len() / 2);
            }
            r
        } else {
            Resp::new(404, "no")
        }
    })
}

fn hf(m: &Mock) -> HuggingFace {
    let mut cfg = HfConfig::local_test(&m.url());
    cfg.extra_hosts = vec!["localhost".into()];
    HuggingFace::new(cfg).unwrap()
}

struct Disk(Option<u64>);
impl DiskSpace for Disk {
    fn available_bytes(&self, _: &Path) -> Option<u64> {
        self.0
    }
}

fn opts() -> DownloadOptions {
    DownloadOptions {
        max_bytes: 10_000_000,
        margin_bytes: 1000,
        attempts: 3,
        backoff: Duration::from_millis(1),
    }
}

fn run(src: &dyn ModelSource, disk: &Disk, dir: &Path, p: &Progress) -> debrief_ai::Result<std::path::PathBuf> {
    let remote = src.describe(REPO, FILE)?;
    Downloader {
        source: src,
        disk,
        opts: opts(),
    }
    .download(&remote, dir, p)
}

#[test]
fn describes_a_file_with_licence_size_and_sha() {
    let data = Arc::new(blob(1000));
    let m = server(data.clone(), Opts::default());
    let r = hf(&m).describe(REPO, FILE).unwrap();
    assert_eq!(
        (r.size, r.sha256.as_str(), r.license.as_deref(), r.gated),
        (1000, sha256_hex(&data).as_str(), Some("apache-2.0"), false)
    );
    assert!(matches!(hf(&m).describe(REPO, "missing.gguf"), Err(AiError::NotFound(_))));
}

#[test]
fn downloads_in_chunks_and_verifies() {
    let data = Arc::new(blob(300_000));
    let m = server(data.clone(), Opts::default());
    let dir = tempfile::tempdir().unwrap();
    let p = Progress::new();
    let path = run(&hf(&m), &Disk(Some(u64::MAX)), dir.path(), &p).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), *data);
    assert!(!dir.path().join("m.gguf.part").exists());
    assert_eq!(m.count("/cdn/"), 6); // the address probe plus five chunks
                                     // the token-less client sent no Authorization header anywhere
    assert!(m.log().iter().all(|r| !r.headers.contains_key("authorization")));
    // an existing correct file is kept without downloading again
    let again = run(&hf(&m), &Disk(Some(0)), dir.path(), &p).unwrap();
    assert_eq!(again, path);
    assert_eq!(m.count("/cdn/"), 6); // the address probe plus five chunks
}

#[test]
fn resumes_from_a_part_file() {
    let data = Arc::new(blob(300_000));
    let m = server(data.clone(), Opts::default());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("m.gguf.part"), &data[..100_000]).unwrap();
    let p = Progress::new();
    let path = run(&hf(&m), &Disk(Some(u64::MAX)), dir.path(), &p).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *data);
    let first = m.log().into_iter().filter(|r| r.path.starts_with("/cdn/")).nth(1).unwrap();
    assert!(first.headers["range"].starts_with("bytes=100000-"), "{:?}", first.headers["range"]);
}

#[test]
fn an_oversized_part_file_is_discarded() {
    let data = Arc::new(blob(50_000));
    let m = server(data.clone(), Opts::default());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("m.gguf.part"), blob(80_000)).unwrap();
    let path = run(&hf(&m), &Disk(None), dir.path(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *data);
}

#[test]
fn checksum_mismatch_deletes_the_file() {
    let data = Arc::new(blob(100_000));
    let m = server(
        data,
        Opts {
            sha: Some("a".repeat(64)),
            ..Opts::default()
        },
    );
    let dir = tempfile::tempdir().unwrap();
    let err = run(&hf(&m), &Disk(None), dir.path(), &Progress::new()).unwrap_err();
    assert!(matches!(err, AiError::Integrity(_)), "{err}");
    assert!(err.to_string().contains("deleted"));
    assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
}

#[test]
fn a_corrupt_existing_file_is_replaced() {
    let data = Arc::new(blob(100_000));
    let m = server(data.clone(), Opts::default());
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("m.gguf"), blob(100_000).iter().map(|b| b ^ 1).collect::<Vec<u8>>()).unwrap();
    let path = run(&hf(&m), &Disk(None), dir.path(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *data);
}

#[test]
fn redirects_to_other_hosts_are_refused() {
    for target in [
        "https://evil.example.com/steal",
        "http://evil.example.com/x",
        "http://localhost.evil.com/x",
        "https://huggingface.co.evil.com/x",
        "ftp://huggingface.co/x",
    ] {
        let m = server(
            Arc::new(blob(1000)),
            Opts {
                redirect: Some(target.into()),
                ..Opts::default()
            },
        );
        let dir = tempfile::tempdir().unwrap();
        let err = run(&hf(&m), &Disk(None), dir.path(), &Progress::new()).unwrap_err();
        assert!(matches!(err, AiError::Refused(_) | AiError::Network(_)), "{target}: {err}");
        assert!(!dir.path().join("m.gguf").exists());
    }
    // a redirect that never ends
    let m = Mock::start(|req, port| {
        if req.path.contains("/api/") {
            Resp::json(
                200,
                json!([{"type":"file","path":FILE,"size":10,"lfs":{"oid":"b".repeat(64),"size":10}}]),
            )
        } else {
            Resp::redirect(&format!("http://127.0.0.1:{port}/loop"))
        }
    });
    let src = hf(&m);
    let remote = src.describe(REPO, FILE).unwrap();
    let err = Downloader {
        source: &src,
        disk: &Disk(None),
        opts: opts(),
    }
    .download(&remote, tempfile::tempdir().unwrap().path(), &Progress::new())
    .unwrap_err();
    assert!(err.to_string().contains("redirects"), "{err}");
    // a redirect without a target
    let m = Mock::start(|_, _| Resp::new(302, ""));
    assert!(hf(&m).describe(REPO, FILE).unwrap_err().to_string().contains("redirect"));
}

#[test]
fn gated_and_missing_repositories_give_clear_errors() {
    let data = Arc::new(blob(1000));
    let m = server(
        data.clone(),
        Opts {
            resolve_status: Some(401),
            ..Opts::default()
        },
    );
    let dir = tempfile::tempdir().unwrap();
    match run(&hf(&m), &Disk(None), dir.path(), &Progress::new()).unwrap_err() {
        AiError::Gated { url } => assert_eq!(url, format!("{}/{REPO}", m.url())),
        e => panic!("{e}"),
    }
    let all401 = Mock::start(|_, _| Resp::new(401, "{}"));
    assert!(matches!(hf(&all401).describe(REPO, FILE), Err(AiError::Gated { .. })));
    let all404 = Mock::start(|_, _| Resp::new(404, "{}"));
    assert!(matches!(hf(&all404).describe(REPO, FILE), Err(AiError::NotFound(_))));
    let all429 = Mock::start(|_, _| Resp::new(429, "{}"));
    assert!(hf(&all429).describe(REPO, FILE).unwrap_err().to_string().contains("rate limiting"));
    let all500 = Mock::start(|_, _| Resp::new(500, "{}"));
    assert!(hf(&all500).describe(REPO, FILE).unwrap_err().is_transient());
    let all418 = Mock::start(|_, _| Resp::new(418, "{}"));
    let e418 = hf(&all418).describe(REPO, FILE).unwrap_err();
    assert!(
        matches!(&e418, AiError::Connection(n) if n.kind == debrief_ai::net::NetKind::Http(418)) && !e418.is_transient(),
        "{e418}"
    );
    // a proxy answering a plain-http request with 407 wants a sign-in
    let all407 = Mock::start(|_, _| Resp::new(407, "{}"));
    let e407 = hf(&all407).describe(REPO, FILE).unwrap_err();
    assert!(
        matches!(&e407, AiError::Connection(n) if n.kind == debrief_ai::net::NetKind::ProxyAuth),
        "{e407}"
    );
}

#[test]
fn transient_failures_are_retried_and_then_reported() {
    let data = Arc::new(blob(100_000));
    let m = server(
        data.clone(),
        Opts {
            fail_chunks: 2,
            ..Opts::default()
        },
    );
    let dir = tempfile::tempdir().unwrap();
    let path = run(&hf(&m), &Disk(None), dir.path(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *data);
    assert!(m.count("/cdn/") >= 4);

    let m = server(
        data.clone(),
        Opts {
            fail_chunks: 100,
            ..Opts::default()
        },
    );
    let dir = tempfile::tempdir().unwrap();
    let err = run(&hf(&m), &Disk(None), dir.path(), &Progress::new()).unwrap_err();
    assert!(err.is_transient(), "{err}");
}

#[test]
fn a_dropped_connection_resumes_where_it_stopped() {
    let data = Arc::new(blob(150_000));
    let m = server(
        data.clone(),
        Opts {
            cut_first: true,
            ..Opts::default()
        },
    );
    let dir = tempfile::tempdir().unwrap();
    let path = run(&hf(&m), &Disk(None), dir.path(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *data);
    let ranges: Vec<String> = m
        .log()
        .iter()
        .filter(|r| r.path.starts_with("/cdn/"))
        .map(|r| r.headers["range"].clone())
        .collect();
    assert!(ranges.len() >= 3 && ranges[1] != ranges[0], "{ranges:?}");
}

#[test]
fn cancel_keeps_the_part_file_for_resuming() {
    let data = Arc::new(blob(300_000));
    let progress = Arc::new(Progress::new());
    let flag = progress.cancel_flag();
    let m = server(
        data.clone(),
        Opts {
            on_chunk: Some(Arc::new(move |n| {
                if n == 3 {
                    flag.store(true, Ordering::SeqCst)
                }
            })),
            ..Opts::default()
        },
    );
    let dir = tempfile::tempdir().unwrap();
    let err = run(&hf(&m), &Disk(None), dir.path(), &progress).unwrap_err();
    assert_eq!(err, AiError::Cancelled);
    let part = std::fs::metadata(dir.path().join("m.gguf.part")).unwrap().len();
    assert!(part > 0 && part < 300_000);
    assert!(!dir.path().join("m.gguf").exists());
    // continuing completes it
    let path = run(&hf(&m), &Disk(None), dir.path(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(path).unwrap(), *data);
}

#[test]
fn not_enough_disk_space_stops_before_downloading() {
    let m = server(Arc::new(blob(100_000)), Opts::default());
    let dir = tempfile::tempdir().unwrap();
    let err = run(&hf(&m), &Disk(Some(50_000)), dir.path(), &Progress::new()).unwrap_err();
    assert!(matches!(err, AiError::Disk(_)), "{err}");
    assert_eq!(m.count("/cdn/"), 0);
    // the margin counts: exactly the file size is not enough
    let err = run(&hf(&m), &Disk(Some(100_500)), dir.path(), &Progress::new()).unwrap_err();
    assert!(matches!(err, AiError::Disk(_)));
}

#[test]
fn unsafe_names_sizes_and_checksums_are_refused() {
    let m = server(Arc::new(blob(1000)), Opts::default());
    let src = hf(&m);
    for (repo, file) in [
        ("../../etc", FILE),
        ("org/repo", "../x.gguf"),
        ("org/repo", "a/b.gguf"),
        ("org/repo", "x.bin"),
        ("onlyone", FILE),
        ("a/b/c", FILE),
    ] {
        assert!(matches!(src.describe(repo, file), Err(AiError::Refused(_))), "{repo} {file}");
    }
    let good = src.describe(REPO, FILE).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let dl = |remote: &RemoteFile, o: DownloadOptions| {
        Downloader {
            source: &src,
            disk: &Disk(None),
            opts: o,
        }
        .download(remote, dir.path(), &Progress::new())
    };
    let mut evil = good.clone();
    evil.file = "../evil.gguf".into();
    assert!(matches!(dl(&evil, opts()), Err(AiError::Refused(_))));
    assert!(matches!(
        src.fetch(&evil, 0, &mut |_| Ok(()), &Default::default()),
        Err(AiError::Refused(_))
    ));
    let mut big = good.clone();
    big.size = 99_000_000_000;
    assert!(dl(&big, opts()).unwrap_err().to_string().contains("limit"));
    let mut empty = good.clone();
    empty.size = 0;
    assert!(matches!(dl(&empty, opts()), Err(AiError::Refused(_))));
    let mut nosha = good.clone();
    nosha.sha256 = String::new();
    assert!(matches!(dl(&nosha, opts()), Err(AiError::Integrity(_))));
    // the host answering without a checksum
    let nolfs = Mock::start(|req, _| {
        if req.path.contains("paths-info") {
            Resp::json(200, json!([{"type":"file","path":FILE,"size":10}]))
        } else {
            Resp::json(200, json!({}))
        }
    });
    assert!(matches!(hf(&nolfs).describe(REPO, FILE), Err(AiError::Integrity(_))));
    let garbage = Mock::start(|_, _| Resp::new(200, "not json"));
    assert!(matches!(hf(&garbage).describe(REPO, FILE), Err(AiError::Malformed(_))));
}

#[test]
fn the_token_goes_only_to_the_model_host() {
    let data = Arc::new(blob(70_000));
    let m = server(data, Opts::default());
    let mut cfg = HfConfig::local_test(&m.url());
    cfg.extra_hosts = vec!["localhost".into()];
    cfg.token = Some("hf_secret_token".into());
    let src = HuggingFace::new(cfg).unwrap();
    let dir = tempfile::tempdir().unwrap();
    let remote = src.describe(REPO, FILE).unwrap();
    Downloader {
        source: &src,
        disk: &Disk(None),
        opts: opts(),
    }
    .download(&remote, dir.path(), &Progress::new())
    .unwrap();
    for r in m.log() {
        let auth = r.headers.get("authorization");
        if r.path.starts_with("/cdn/") {
            assert!(auth.is_none(), "token leaked to the CDN host");
        } else {
            assert_eq!(auth.map(String::as_str), Some("Bearer hf_secret_token"));
        }
    }
}

fn with(f: impl FnOnce(&mut HfConfig)) -> HfConfig {
    let mut c = HfConfig::default();
    f(&mut c);
    c
}

#[test]
fn endpoint_and_proxy_validation() {
    assert!(matches!(
        HuggingFace::new(with(|c| c.endpoint = "http://example.com".into())),
        Err(AiError::Refused(_))
    ));
    assert!(matches!(
        HuggingFace::new(with(|c| c.endpoint = "ftp://huggingface.co".into())),
        Err(AiError::Refused(_))
    ));
    assert!(HuggingFace::new(with(|c| c.endpoint = "not a url".into())).is_err());
    assert!(HuggingFace::new(with(|c| {
        c.endpoint = "https://hf-mirror.corp.example".into();
        c.extra_hosts = vec!["cdn.corp.example".into()];
    }))
    .is_ok());
    assert!(HuggingFace::new(with(|c| {
        c.proxy = Some("http://proxy.corp:3128".into());
    }))
    .is_ok());
    assert!(matches!(
        HuggingFace::new(with(|c| c.proxy = Some("::bad::".into()))),
        Err(AiError::Invalid(_))
    ));
}

#[test]
fn a_fresh_download_is_hashed_as_it_is_written_without_a_second_pass() {
    let data = Arc::new(blob(300_000));
    let m = server(data.clone(), Opts::default());
    let dir = tempfile::tempdir().unwrap();
    let p = Progress::new();
    let path = run(&hf(&m), &Disk(Some(u64::MAX)), dir.path(), &p).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), *data);
    let s = p.snapshot();
    // The bar never restarted for a verification pass: it ended as the download, full.
    assert_eq!((s.phase, s.bytes, s.total), (Phase::Downloading, 300_000, 300_000));
}

#[test]
fn a_fresh_download_with_a_wrong_checksum_is_still_rejected_and_deleted() {
    let data = Arc::new(blob(50_000));
    let m = server(
        data.clone(),
        Opts {
            sha: Some("0".repeat(64)),
            ..Opts::default()
        },
    );
    let dir = tempfile::tempdir().unwrap();
    let err = run(&hf(&m), &Disk(Some(u64::MAX)), dir.path(), &Progress::new()).unwrap_err();
    assert!(matches!(err, AiError::Integrity(_)));
    assert!(!dir.path().join("m.gguf").exists() && !dir.path().join("m.gguf.part").exists());
}

#[test]
fn a_download_that_resumed_after_a_cut_still_verifies() {
    let data = Arc::new(blob(300_000));
    let m = server(
        data.clone(),
        Opts {
            cut_first: true,
            ..Opts::default()
        },
    );
    let dir = tempfile::tempdir().unwrap();
    let path = run(&hf(&m), &Disk(Some(u64::MAX)), dir.path(), &Progress::new()).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), *data);
}
