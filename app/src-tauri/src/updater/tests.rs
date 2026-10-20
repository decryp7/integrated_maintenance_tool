use super::error::{chain, classify, is_disk_full, looks_offline, network, Step, MAX_DOWNLOAD_BYTES};
use super::manifest::{self, MAX_MANIFEST_BYTES, MAX_NOTES_CHARS};
use super::section::{Channel, SharedSettings, UpdateSettings, UpdatesSection};
use super::state::{auto_check_due, Event};
use super::*;
use crate::test_support::TempDir;
use base64::Engine;
use imt_core::settings::{sanitize, validate, Configurable};
use serde_json::{json, Value};
use std::sync::atomic::{AtomicUsize, Ordering};

fn b64(s: impl AsRef<[u8]>) -> String {
    base64::engine::general_purpose::STANDARD.encode(s)
}

/// A key in the format `tauri signer generate` writes (base64 of the `.pub` file).
fn public_key() -> String {
    let mut raw = b"Ed".to_vec();
    raw.extend([7u8; 40]);
    b64(format!("untrusted comment: minisign public key: 0707070707070707\n{}\n", b64(raw)))
}

fn signature() -> String {
    b64("untrusted comment: signature from tauri secret key\nRUQ=\ntrusted comment: timestamp:1\tfile:IMT.exe\tversion:1.2.0\nAAAA\n")
}

fn manifest_json() -> Value {
    json!({
        "version": "1.2.0",
        "notes": "## Fixes\n- faster <b>grid</b>\u{0007}",
        "pub_date": "2026-10-17T12:00:00Z",
        "platforms": {
            "windows-x86_64-nsis": { "url": "https://example.net/r/IMT_1.2.0_x64-setup.exe", "signature": signature(), "size": 41_000_000 },
            "darwin-aarch64": { "url": "https://example.net/r/IMT.app.tar.gz", "signature": signature() }
        }
    })
}

fn info(size: Option<u64>) -> UpdateInfo {
    UpdateInfo {
        version: "1.2.0".into(),
        current_version: "1.1.0".into(),
        notes: "notes".into(),
        pub_date: None,
        size,
    }
}

// ---- latest.json ----

#[test]
fn manifest_parses_and_sanitizes() {
    let m = manifest::parse(&manifest_json().to_string()).unwrap();
    assert_eq!(m.version, semver::Version::new(1, 2, 0));
    // Plain text kept as is (markup is shown, not rendered); the bell is removed.
    assert_eq!(m.notes, "## Fixes\n- faster <b>grid</b>");
    assert_eq!(m.pub_date.as_deref(), Some("2026-10-17T12:00:00Z"));
    assert_eq!(m.platforms.len(), 2);
    let p = m.platform_by_url("https://example.net/r/IMT_1.2.0_x64-setup.exe").unwrap();
    assert_eq!(p.size, Some(41_000_000));
    assert_eq!(m.platform_by_url("https://example.net/r/IMT.app.tar.gz").unwrap().size, None);
    assert!(m.platform_by_url("https://elsewhere").is_none());

    // A "v" prefix, missing notes and date are fine.
    let mut v = manifest_json();
    v["version"] = json!("v2.0.0-rc.1");
    v.as_object_mut().unwrap().remove("notes");
    v["pub_date"] = Value::Null;
    let m = manifest::from_value(&v).unwrap();
    assert_eq!(m.version.to_string(), "2.0.0-rc.1");
    assert_eq!(m.notes, "");
    assert_eq!(m.pub_date, None);
}

#[test]
fn manifest_notes_are_bounded() {
    let long = "a".repeat(MAX_NOTES_CHARS + 50);
    let notes = manifest::sanitize_notes(&long);
    assert_eq!(notes.chars().count(), MAX_NOTES_CHARS + 1);
    assert!(notes.ends_with('…'));
    assert_eq!(manifest::sanitize_notes("  tab\there\r\nline\u{1b}[31m  "), "tab\there\nline[31m");
}

#[test]
fn manifest_rejects_invalid_release_information() {
    let with = |f: &dyn Fn(&mut Value)| {
        let mut v = manifest_json();
        f(&mut v);
        manifest::from_value(&v).unwrap_err()
    };
    let win = "windows-x86_64-nsis";
    let cases: Vec<(String, &str)> = vec![
        (manifest::parse("{not json").unwrap_err(), "not JSON"),
        (manifest::from_value(&json!([1, 2])).unwrap_err(), "object"),
        (with(&|v| drop(v.as_object_mut().unwrap().remove("version"))), "version"),
        (with(&|v| v["version"] = json!("latest")), "semantic version"),
        (with(&|v| v["version"] = json!(12)), "version"),
        (with(&|v| v["notes"] = json!({"html": "<script>"})), "notes"),
        (with(&|v| v["pub_date"] = json!("yesterday")), "pub_date"),
        (with(&|v| v["pub_date"] = json!(5)), "pub_date"),
        (with(&|v| drop(v.as_object_mut().unwrap().remove("platforms"))), "platforms"),
        (with(&|v| v["platforms"] = json!({})), "empty"),
        (
            with(&|v| v["platforms"] = json!({ "Windows X": v["platforms"][win].clone() })),
            "platform name",
        ),
        (with(&|v| v["platforms"][win] = json!("https://x")), "object"),
        (with(&|v| v["platforms"][win]["url"] = json!("http://example.net/IMT.exe")), "https"),
        (with(&|v| v["platforms"][win]["url"] = json!("not a url")), "URL"),
        (
            with(&|v| v["platforms"][win]["url"] = json!(format!("https://x/{}", "a".repeat(3000)))),
            "too long",
        ),
        (
            with(&|v| drop(v["platforms"][win].as_object_mut().unwrap().remove("signature"))),
            "signature",
        ),
        (with(&|v| v["platforms"][win]["signature"] = json!("")), "empty"),
        (with(&|v| v["platforms"][win]["signature"] = json!("%%%")), "base64"),
        (with(&|v| v["platforms"][win]["signature"] = json!(b64("hello"))), "minisign"),
        (with(&|v| v["platforms"][win]["signature"] = json!(b64([0xff, 0xfe]))), "minisign"),
        (with(&|v| v["platforms"][win]["signature"] = json!("A".repeat(9000))), "too long"),
        (with(&|v| v["platforms"][win]["size"] = json!(-1)), "size"),
        (with(&|v| v["platforms"][win]["size"] = json!("big")), "size"),
    ];
    for (error, expected) in cases {
        assert!(error.contains(expected), "{error:?} should mention {expected:?}");
    }
    let many: serde_json::Map<String, Value> = (0..65)
        .map(|i| (format!("p{i}"), manifest_json()["platforms"][win].clone()))
        .collect();
    assert!(with(&|v| v["platforms"] = Value::Object(many.clone())).contains("more than"));
    let huge = format!("{{\"notes\":\"{}\"}}", "x".repeat(MAX_MANIFEST_BYTES));
    assert!(manifest::parse(&huge).unwrap_err().contains("larger than"));
}

#[test]
fn manifest_parsing_never_panics() {
    let text = manifest_json().to_string();
    // Every prefix of a valid file, and bytes flipped through it.
    for end in 0..text.len() {
        if text.is_char_boundary(end) {
            let _ = manifest::parse(&text[..end]);
        }
    }
    let bytes = text.as_bytes();
    let mut seed = 0x2545_f491_u32;
    for _ in 0..2000 {
        let mut b = bytes.to_vec();
        for _ in 0..4 {
            seed ^= seed << 13;
            seed ^= seed >> 17;
            seed ^= seed << 5;
            let i = seed as usize % b.len();
            b[i] = (seed >> 8) as u8;
        }
        let _ = manifest::parse(&String::from_utf8_lossy(&b));
    }
    for v in [
        json!(null),
        json!(true),
        json!("x"),
        json!({"version": null}),
        json!({"version": "1.0.0", "platforms": []}),
    ] {
        assert!(manifest::from_value(&v).is_err());
    }
}

#[test]
fn public_keys_are_recognised() {
    assert!(manifest::is_public_key(&public_key()));
    assert!(manifest::is_public_key(&format!("  {}\n", public_key())));
    for bad in [
        "",
        "garbage",
        &b64("untrusted comment: x"),
        &b64("hello\nRWQ=\n"),
        &b64(format!("untrusted comment: x\n{}\n", b64("short"))),
    ] {
        assert!(!manifest::is_public_key(bad), "{bad}");
    }
}

#[test]
fn pubkey_comes_from_the_build_then_the_config() {
    let key = public_key();
    assert_eq!(resolve_pubkey(None, None), None);
    assert_eq!(resolve_pubkey(Some(""), Some("  ")), None);
    assert_eq!(resolve_pubkey(Some("not a key"), None), None);
    assert_eq!(resolve_pubkey(None, Some(&key)), Some(key.clone()));
    assert_eq!(resolve_pubkey(Some(&key), Some("not a key")), Some(key.clone()));
    assert_eq!(resolve_pubkey(Some(" "), Some(&key)), Some(key));
}

// ---- errors ----

#[test]
fn plugin_errors_are_classified() {
    use tauri_plugin_updater::Error as E;
    let kind = |e: E, step: Step| classify(&e, step).kind();
    assert_eq!(kind(E::ReleaseNotFound, Step::Check), "noRelease");
    assert_eq!(kind(E::TargetsNotFound(vec!["linux-x86_64".into()]), Step::Check), "noPlatform");
    assert_eq!(kind(E::TargetNotFound("x".into()), Step::Check), "noPlatform");
    let bad_json = serde_json::from_str::<Value>("{").unwrap_err();
    assert_eq!(kind(E::Serialization(bad_json), Step::Check), "invalidManifest");
    assert_eq!(kind(E::InsecureTransportProtocol, Step::Check), "invalidManifest");
    assert_eq!(
        kind(
            E::SignedVersionMismatch {
                signed: "1.0.0".into(),
                announced: "9.0.0".into()
            },
            Step::Download
        ),
        "signatureInvalid"
    );
    assert_eq!(kind(E::MissingSignedVersion, Step::Download), "signatureInvalid");
    assert_eq!(kind(E::SignatureUtf8("x".into()), Step::Download), "signatureInvalid");
    assert_eq!(
        kind(E::Base64(base64::DecodeError::InvalidLength(3)), Step::Download),
        "signatureInvalid"
    );
    assert_eq!(
        kind(E::Network("Download request failed with status: 404".into()), Step::Download),
        "unreachable"
    );
    assert_eq!(
        kind(E::Io(std::io::Error::from(std::io::ErrorKind::StorageFull)), Step::Install),
        "diskFull"
    );
    assert_eq!(kind(E::Io(std::io::Error::other("locked")), Step::Install), "installFailed");
    assert_eq!(kind(E::AuthenticationFailed, Step::Install), "installFailed");
    assert_eq!(kind(E::FormatDate, Step::Check), "unreachable");
    assert_eq!(kind(E::EmptyEndpoints, Step::Check), "notAvailable");
}

#[test]
fn disk_full_and_offline_are_recognised() {
    assert!(is_disk_full(&std::io::Error::from(std::io::ErrorKind::StorageFull)));
    #[cfg(windows)]
    assert!(is_disk_full(&std::io::Error::from_raw_os_error(112)));
    #[cfg(not(windows))]
    assert!(is_disk_full(&std::io::Error::from_raw_os_error(28)));
    assert!(!is_disk_full(&std::io::Error::from(std::io::ErrorKind::PermissionDenied)));

    assert!(looks_offline(
        "error sending request: client error (Connect): dns error: No such host is known. (os error 11001)"
    ));
    assert!(looks_offline("failed to lookup address information: Name or service not known"));
    assert!(!looks_offline("connection refused"));
    assert_eq!(network(true, "dns error".into()), UpdateError::Offline);
    assert_eq!(network(false, "dns error".into()).kind(), "unreachable");
    assert_eq!(network(true, "connection refused".into()).kind(), "unreachable");

    let io = std::io::Error::other("inner");
    let outer = std::io::Error::other(io);
    assert!(chain(&outer).contains("inner"));
}

#[test]
fn errors_serialize_with_message_and_next_step() {
    let all = [
        UpdateError::NotAvailable,
        UpdateError::Offline,
        UpdateError::Unreachable { detail: "timeout".into() },
        UpdateError::NoRelease,
        UpdateError::NoPlatform { detail: "x".into() },
        UpdateError::InvalidManifest { detail: "x".into() },
        UpdateError::SignatureInvalid { detail: "x".into() },
        UpdateError::TooLarge { limit: MAX_DOWNLOAD_BYTES },
        UpdateError::DiskFull,
        UpdateError::InstallFailed { detail: "x".into() },
        UpdateError::Busy { detail: "Wait.".into() },
    ];
    for e in &all {
        let v = serde_json::to_value(e).unwrap();
        assert_eq!(v["kind"], e.kind());
        assert!(!v["message"].as_str().unwrap().is_empty());
        assert!(!v["nextStep"].as_str().unwrap().is_empty());
        assert!(e.to_string().starts_with(&e.message()));
    }
    assert!(UpdateError::TooLarge { limit: MAX_DOWNLOAD_BYTES }.message().contains("512 MB"));
    assert_eq!(serde_json::to_value(UpdateError::Offline).unwrap()["detail"], Value::Null);
    assert!(UpdateError::Unreachable { detail: "timeout".into() }
        .to_string()
        .contains("timeout"));
    let refused = serde_json::to_value(UpdateError::SignatureInvalid { detail: "bad".into() }).unwrap();
    assert!(refused["message"].as_str().unwrap().contains("refused"));
}

// ---- state machine ----

#[test]
fn happy_path_through_the_phases() {
    let p = UpdatePhase::Idle;
    let p = p.next(Event::Check).unwrap();
    assert_eq!(p, UpdatePhase::Checking);
    let p = p.next(Event::Checked(Ok(Some(info(Some(100)))))).unwrap();
    assert!(matches!(p, UpdatePhase::Available { .. }));
    let p = p.next(Event::Download).unwrap();
    assert_eq!(
        p,
        UpdatePhase::Downloading {
            info: info(Some(100)),
            downloaded: 0,
            total: Some(100)
        }
    );
    let p = p.next(Event::Progress { chunk: 40, total: None }).unwrap();
    let p = p
        .next(Event::Progress {
            chunk: 60,
            total: Some(100),
        })
        .unwrap();
    assert!(matches!(
        p,
        UpdatePhase::Downloading {
            downloaded: 100,
            total: Some(100),
            ..
        }
    ));
    let p = p.next(Event::Downloaded(Ok(()))).unwrap();
    assert!(matches!(
        p,
        UpdatePhase::Ready {
            install_on_quit: false,
            ..
        }
    ));
    let p = p.next(Event::InstallOnQuit(true)).unwrap();
    assert!(matches!(p, UpdatePhase::Ready { install_on_quit: true, .. }));
    let p = p.next(Event::Install).unwrap();
    assert!(matches!(p, UpdatePhase::Installing { .. }));
    let p = p.next(Event::InstallFailed(UpdateError::DiskFull)).unwrap();
    assert!(matches!(
        &p,
        UpdatePhase::Failed {
            error: UpdateError::DiskFull,
            info: Some(_)
        }
    ));
    // Disk full can be retried.
    assert!(matches!(p.next(Event::Download).unwrap(), UpdatePhase::Downloading { .. }));
    assert_eq!(UpdatePhase::Checking.next(Event::Checked(Ok(None))).unwrap(), UpdatePhase::UpToDate);
}

#[test]
fn failures_cancel_and_refusals() {
    let failed = UpdatePhase::Checking.next(Event::Checked(Err(UpdateError::Offline))).unwrap();
    assert_eq!(
        failed,
        UpdatePhase::Failed {
            error: UpdateError::Offline,
            info: None
        }
    );
    assert_eq!(failed.next(Event::Check).unwrap(), UpdatePhase::Checking);
    assert_eq!(failed.next(Event::Download).unwrap_err().kind(), "busy");

    let downloading = UpdatePhase::Available { info: info(None) }.next(Event::Download).unwrap();
    assert_eq!(
        downloading.next(Event::Cancel).unwrap(),
        UpdatePhase::Available { info: info(None) }
    );
    let refused = downloading
        .next(Event::Downloaded(Err(UpdateError::SignatureInvalid { detail: "x".into() })))
        .unwrap();
    // A refused signature is not retried blindly: check again first.
    assert_eq!(refused.next(Event::Download).unwrap_err().kind(), "busy");
    assert_eq!(refused.next(Event::Check).unwrap(), UpdatePhase::Checking);

    let ready = downloading.next(Event::Downloaded(Ok(()))).unwrap();
    assert_eq!(ready.next(Event::Cancel).unwrap(), UpdatePhase::Available { info: info(None) });
    assert_eq!(ready.next(Event::Check).unwrap_err().kind(), "busy");
    assert_eq!(UpdatePhase::Checking.next(Event::Check).unwrap_err().kind(), "busy");
    assert_eq!(UpdatePhase::Idle.next(Event::Download).unwrap_err().kind(), "busy");
    assert_eq!(UpdatePhase::Idle.next(Event::Cancel).unwrap_err().kind(), "busy");
    assert_eq!(UpdatePhase::Idle.next(Event::Install).unwrap_err().kind(), "busy");
    assert_eq!(UpdatePhase::UpToDate.next(Event::InstallOnQuit(true)).unwrap_err().kind(), "busy");
    // Stale results after a cancel are ignored.
    let available = UpdatePhase::Available { info: info(None) };
    assert!(available.next(Event::Progress { chunk: 1, total: None }).is_err());
    assert!(available.next(Event::Downloaded(Ok(()))).is_err());
    assert!(available.next(Event::Checked(Ok(None))).is_err());
    assert!(available.next(Event::InstallFailed(UpdateError::DiskFull)).is_err());
}

#[test]
fn a_build_without_a_key_refuses_everything() {
    for e in [
        Event::Check,
        Event::Download,
        Event::Cancel,
        Event::Install,
        Event::InstallOnQuit(true),
        Event::Checked(Ok(None)),
    ] {
        assert_eq!(UpdatePhase::Unavailable.next(e).unwrap_err(), UpdateError::NotAvailable);
    }
    assert!(!UpdatePhase::Unavailable.can_check());
}

#[test]
fn oversized_downloads_are_refused() {
    let big = UpdatePhase::Available {
        info: info(Some(MAX_DOWNLOAD_BYTES + 1)),
    };
    assert!(matches!(
        big.next(Event::Download).unwrap(),
        UpdatePhase::Failed {
            error: UpdateError::TooLarge { .. },
            ..
        }
    ));
    let downloading = UpdatePhase::Available { info: info(None) }.next(Event::Download).unwrap();
    let by_header = downloading
        .next(Event::Progress {
            chunk: 1,
            total: Some(MAX_DOWNLOAD_BYTES + 1),
        })
        .unwrap();
    assert!(matches!(
        by_header,
        UpdatePhase::Failed {
            error: UpdateError::TooLarge { .. },
            ..
        }
    ));
    let by_bytes = downloading
        .next(Event::Progress {
            chunk: MAX_DOWNLOAD_BYTES + 1,
            total: None,
        })
        .unwrap();
    assert!(matches!(
        &by_bytes,
        UpdatePhase::Failed {
            error: UpdateError::TooLarge { .. },
            ..
        }
    ));
    assert_eq!(by_bytes.next(Event::Download).unwrap_err().kind(), "busy");
}

#[test]
fn phases_serialize_as_tagged_objects() {
    let v = serde_json::to_value(UpdatePhase::Ready {
        info: info(Some(5)),
        install_on_quit: true,
    })
    .unwrap();
    assert_eq!(v["kind"], "ready");
    assert_eq!(v["installOnQuit"], true);
    assert_eq!(v["info"]["currentVersion"], "1.1.0");
    assert_eq!(v["info"]["size"], 5);
    let f = serde_json::to_value(UpdatePhase::Failed {
        error: UpdateError::Offline,
        info: None,
    })
    .unwrap();
    assert_eq!(f["kind"], "failed");
    assert_eq!(f["error"]["kind"], "offline");
    assert_eq!(serde_json::to_value(UpdatePhase::UpToDate).unwrap(), json!({ "kind": "upToDate" }));
}

#[test]
fn automatic_check_timing() {
    let day = 24 * 60 * 60;
    assert!(!auto_check_due(false, &UpdatePhase::Idle, 120, None), "off by default");
    assert!(auto_check_due(true, &UpdatePhase::Idle, 120, None));
    assert!(!auto_check_due(true, &UpdatePhase::Idle, 10, None), "not during start-up");
    assert!(!auto_check_due(true, &UpdatePhase::UpToDate, 120, Some(day - 1)));
    assert!(auto_check_due(true, &UpdatePhase::UpToDate, 120, Some(day)));
    assert!(auto_check_due(
        true,
        &UpdatePhase::Failed {
            error: UpdateError::Offline,
            info: None
        },
        120,
        Some(day + 5)
    ));
    assert!(!auto_check_due(
        true,
        &UpdatePhase::Available { info: info(None) },
        120,
        Some(day * 2)
    ));
    assert!(!auto_check_due(true, &UpdatePhase::Checking, 120, None));
    assert!(!auto_check_due(true, &UpdatePhase::Unavailable, 120, None));
}

// ---- settings ----

#[test]
fn settings_section_is_off_by_default_and_explains_what_is_sent() {
    let shared = SharedSettings::default();
    let section = UpdatesSection {
        settings: shared.clone(),
        endpoint_host: Some("dev.decryptology.net".into()),
    };
    let schema = section.settings_section();
    schema.check().unwrap();
    assert!(schema.description.contains("dev.decryptology.net"));
    assert!(schema.description.contains("no identifier"));
    let (defaults, errors) = sanitize(&schema, &Value::Null);
    assert!(errors.is_empty());
    section.apply_settings(&defaults).unwrap();
    assert_eq!(shared.get(), UpdateSettings::default());
    assert!(!shared.get().auto_check);

    let values = validate(&schema, &json!({ "autoCheck": true, "channel": "stable" })).unwrap();
    section.apply_settings(&values).unwrap();
    assert!(shared.get().auto_check);
    assert_eq!(shared.get().channel, Channel::Stable);
    assert!(validate(&schema, &json!({ "channel": "nightly" })).is_err());
    assert_eq!(Channel::parse("beta"), None);

    let unavailable = UpdatesSection {
        settings: SharedSettings::default(),
        endpoint_host: None,
    }
    .settings_section();
    assert!(unavailable.description.contains("not available in this build"));
}

// ---- the updater with a fake server ----

#[derive(Default)]
struct Fake {
    /// What `check` returns (`None`: up to date).
    found: Mutex<Option<Result<Option<UpdateInfo>, UpdateError>>>,
    /// Chunks `download` reports; `None` makes it hang (for cancel).
    chunks: Mutex<Option<Vec<u64>>>,
    download_result: Mutex<Option<UpdateError>>,
    install_result: Mutex<Option<UpdateError>>,
    installs: Mutex<Vec<bool>>,
    discards: AtomicUsize,
}

impl UpdateClient for Fake {
    fn check(&self) -> BoxFuture<'_, Result<Option<UpdateInfo>, UpdateError>> {
        let r = self.found.lock().unwrap().clone().unwrap_or(Ok(None));
        Box::pin(async move { r })
    }
    fn download(&self, mut on_chunk: OnChunk) -> BoxFuture<'_, Result<(), UpdateError>> {
        let chunks = self.chunks.lock().unwrap().clone();
        let fail = self.download_result.lock().unwrap().clone();
        Box::pin(async move {
            let Some(chunks) = chunks else {
                on_chunk(1, Some(10));
                std::future::pending::<()>().await;
                return Ok(());
            };
            for c in chunks {
                on_chunk(c, Some(100));
            }
            fail.map_or(Ok(()), Err)
        })
    }
    fn install(&self, relaunch: bool) -> Result<(), UpdateError> {
        self.installs.lock().unwrap().push(relaunch);
        self.install_result.lock().unwrap().clone().map_or(Ok(()), Err)
    }
    fn discard(&self) {
        self.discards.fetch_add(1, Ordering::SeqCst);
    }
}

struct Rig {
    updates: Arc<Updates>,
    fake: Arc<Fake>,
    events: Arc<Mutex<Vec<UpdateStatus>>>,
    _dir: TempDir,
    dir: PathBuf,
}

fn rig(tag: &str, with_key: bool) -> Rig {
    let dir = TempDir::new(&format!("updater-{tag}"));
    let path = dir.0.clone();
    rig_in(dir, path, with_key)
}

fn rig_in(dir: TempDir, path: PathBuf, with_key: bool) -> Rig {
    let fake = Arc::new(Fake::default());
    let events = Arc::new(Mutex::new(Vec::new()));
    let sink = events.clone();
    let client: Option<Arc<dyn UpdateClient>> = with_key.then(|| fake.clone() as Arc<dyn UpdateClient>);
    let updates = Arc::new(Updates::new(
        client,
        "1.1.0".into(),
        with_key.then(|| "dev.decryptology.net".to_string()),
        SharedSettings::default(),
        Some(path.clone()),
        Box::new(move |s| sink.lock().unwrap().push(s.clone())),
    ));
    Rig {
        updates,
        fake,
        events,
        _dir: dir,
        dir: path,
    }
}

fn wait_for(updates: &Updates, what: &str, done: impl Fn(&UpdatePhase) -> bool) -> UpdatePhase {
    for _ in 0..500 {
        let phase = updates.status().phase;
        if done(&phase) {
            return phase;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("timed out waiting for {what}: {:?}", updates.status().phase);
}

#[test]
fn a_build_without_a_key_reports_not_available() {
    let r = rig("nokey", false);
    let status = r.updates.status();
    assert_eq!(status.phase, UpdatePhase::Unavailable);
    assert_eq!(status.endpoint_host, None);
    assert!(!status.auto_check);
    assert_eq!(
        crate::test_support::block(r.updates.check()).unwrap_err(),
        UpdateError::NotAvailable
    );
    assert_eq!(r.updates.download().unwrap_err(), UpdateError::NotAvailable);
    assert_eq!(r.updates.cancel().unwrap_err(), UpdateError::NotAvailable);
    assert_eq!(r.updates.install_on_quit(true).unwrap_err(), UpdateError::NotAvailable);
    assert_eq!(r.updates.prepare_restart().unwrap_err(), UpdateError::NotAvailable);
    assert!(!r.updates.auto_check_due());
    assert!(!r.updates.install_at_exit());
    assert!(r.events.lock().unwrap().is_empty());
    assert!(r.fake.installs.lock().unwrap().is_empty());
}

#[test]
fn check_download_and_install_on_quit() {
    let r = rig("flow", true);
    *r.fake.found.lock().unwrap() = Some(Ok(Some(info(Some(100)))));
    *r.fake.chunks.lock().unwrap() = Some(vec![30, 30, 40]);
    let status = crate::test_support::block(r.updates.check()).unwrap();
    assert!(matches!(status.phase, UpdatePhase::Available { .. }));
    assert!(status.last_checked.is_some());
    assert_eq!(status.releases_page, RELEASES_PAGE);
    // The check time is recorded for the daily automatic check.
    assert!(record::load(Some(&r.dir)).last_checked.is_some());

    let started = r.updates.download().unwrap();
    assert!(matches!(started.phase, UpdatePhase::Downloading { downloaded: 0, .. }));
    wait_for(&r.updates, "ready", |p| matches!(p, UpdatePhase::Ready { .. }));
    let kinds: Vec<String> = r
        .events
        .lock()
        .unwrap()
        .iter()
        .map(|s| serde_json::to_value(&s.phase).unwrap()["kind"].to_string())
        .collect();
    assert!(kinds.iter().any(|k| k.contains("checking")));
    assert!(kinds.iter().any(|k| k.contains("downloading")));

    // Nothing is installed at exit unless asked.
    assert!(!r.updates.install_at_exit());
    let s = r.updates.install_on_quit(true).unwrap();
    assert!(matches!(s.phase, UpdatePhase::Ready { install_on_quit: true, .. }));
    assert!(!r.updates.install_at_exit(), "installed without a relaunch");
    assert_eq!(*r.fake.installs.lock().unwrap(), vec![false]);
}

#[test]
fn restart_installs_and_relaunches() {
    let r = rig("restart", true);
    *r.fake.found.lock().unwrap() = Some(Ok(Some(info(None))));
    *r.fake.chunks.lock().unwrap() = Some(vec![100]);
    crate::test_support::block(r.updates.check()).unwrap();
    assert_eq!(r.updates.prepare_restart().unwrap_err().kind(), "busy");
    r.updates.download().unwrap();
    wait_for(&r.updates, "ready", |p| matches!(p, UpdatePhase::Ready { .. }));
    let s = r.updates.prepare_restart().unwrap();
    assert!(matches!(s.phase, UpdatePhase::Installing { .. }));
    // Windows' installer starts IMT itself; elsewhere IMT relaunches.
    assert_eq!(r.updates.install_at_exit(), !cfg!(windows));
    assert_eq!(*r.fake.installs.lock().unwrap(), vec![true]);
}

#[test]
fn a_failed_install_at_exit_is_shown_at_the_next_start() {
    let r = rig("failed-install", true);
    *r.fake.found.lock().unwrap() = Some(Ok(Some(info(None))));
    *r.fake.chunks.lock().unwrap() = Some(vec![10]);
    *r.fake.install_result.lock().unwrap() = Some(UpdateError::DiskFull);
    crate::test_support::block(r.updates.check()).unwrap();
    r.updates.download().unwrap();
    wait_for(&r.updates, "ready", |p| matches!(p, UpdatePhase::Ready { .. }));
    r.updates.prepare_restart().unwrap();
    assert!(r.updates.install_at_exit(), "the old version starts again");
    let Rig { _dir, dir, .. } = r;
    let next = rig_in(_dir, dir.clone(), true);
    assert_eq!(
        next.updates.status().phase,
        UpdatePhase::Failed {
            error: UpdateError::DiskFull,
            info: None
        }
    );
    assert!(next.updates.status().last_checked.is_some());
    // Shown once.
    assert!(record::load(Some(&dir)).failed_install.is_none());

    let other = record::FailedInstall::of("1.2.0", &UpdateError::InstallFailed { detail: "denied".into() });
    assert_eq!(other.error().detail().unwrap(), "IMT 1.2.0: denied");
}

#[test]
fn cancel_stops_the_download_and_keeps_the_update() {
    let r = rig("cancel", true);
    *r.fake.found.lock().unwrap() = Some(Ok(Some(info(None))));
    crate::test_support::block(r.updates.check()).unwrap();
    r.updates.download().unwrap();
    wait_for(&r.updates, "progress", |p| {
        matches!(p, UpdatePhase::Downloading { downloaded: 1, .. })
    });
    let s = r.updates.cancel().unwrap();
    assert_eq!(s.phase, UpdatePhase::Available { info: info(None) });
    assert!(r.fake.discards.load(Ordering::SeqCst) >= 1);
    assert_eq!(r.updates.cancel().unwrap_err().kind(), "busy");
    // A new download can start.
    *r.fake.chunks.lock().unwrap() = Some(vec![5]);
    r.updates.download().unwrap();
    wait_for(&r.updates, "ready", |p| matches!(p, UpdatePhase::Ready { .. }));
}

#[test]
fn download_errors_and_offline_checks_are_typed() {
    let r = rig("errors", true);
    *r.fake.found.lock().unwrap() = Some(Err(UpdateError::Offline));
    let s = crate::test_support::block(r.updates.check()).unwrap();
    assert_eq!(
        s.phase,
        UpdatePhase::Failed {
            error: UpdateError::Offline,
            info: None
        }
    );

    *r.fake.found.lock().unwrap() = Some(Ok(Some(info(None))));
    *r.fake.chunks.lock().unwrap() = Some(vec![10]);
    *r.fake.download_result.lock().unwrap() = Some(UpdateError::SignatureInvalid { detail: "bad".into() });
    crate::test_support::block(r.updates.check()).unwrap();
    r.updates.download().unwrap();
    let p = wait_for(&r.updates, "failure", |p| matches!(p, UpdatePhase::Failed { .. }));
    assert!(matches!(
        p,
        UpdatePhase::Failed {
            error: UpdateError::SignatureInvalid { .. },
            info: Some(_)
        }
    ));
    // Nothing to install.
    assert!(!r.updates.install_at_exit());
    assert!(r.fake.installs.lock().unwrap().is_empty());

    // Up to date.
    *r.fake.found.lock().unwrap() = None;
    assert_eq!(crate::test_support::block(r.updates.check()).unwrap().phase, UpdatePhase::UpToDate);
}

#[test]
fn oversized_download_is_stopped() {
    let r = rig("too-large", true);
    *r.fake.found.lock().unwrap() = Some(Ok(Some(info(None))));
    *r.fake.chunks.lock().unwrap() = Some(vec![MAX_DOWNLOAD_BYTES, 10, 10]);
    crate::test_support::block(r.updates.check()).unwrap();
    r.updates.download().unwrap();
    let p = wait_for(&r.updates, "too large", |p| matches!(p, UpdatePhase::Failed { .. }));
    assert!(matches!(
        p,
        UpdatePhase::Failed {
            error: UpdateError::TooLarge { .. },
            ..
        }
    ));
    std::thread::sleep(Duration::from_millis(50));
    // The finished transfer's bytes are dropped, not kept.
    assert!(matches!(r.updates.status().phase, UpdatePhase::Failed { .. }));
}

#[test]
fn corrupt_or_newer_records_are_ignored() {
    let dir = TempDir::new("updater-record");
    std::fs::write(dir.0.join("updates.json"), "{ not json").unwrap();
    assert_eq!(record::load(Some(&dir.0)), record::Record::default());
    std::fs::write(
        dir.0.join("updates.json"),
        r#"{"version": 99, "lastChecked": "2026-01-01T00:00:00Z"}"#,
    )
    .unwrap();
    assert_eq!(record::load(Some(&dir.0)), record::Record::default());
    assert_eq!(record::load(None), record::Record::default());
    record::save(None, &record::Record::default());
    let r = rig_in(dir, std::env::temp_dir().join("imt-updater-missing").join("x"), true);
    assert_eq!(r.updates.status().phase, UpdatePhase::Idle);
}

/// The download is a job: its id is in the status, `jobs_cancel` stops it (the update stays available),
/// and a downloaded update is dropped with `updates_discard`.
#[test]
fn the_download_is_a_job_that_jobs_cancel_stops() {
    use tauri::Manager;
    let r = rig("download-job", true);
    let app = crate::test_support::test_app(None);
    app.manage(r.updates.clone());
    *r.fake.found.lock().unwrap() = Some(Ok(Some(info(None))));
    crate::test_support::block(r.updates.check()).unwrap();
    let s = updates_download(app.handle().clone()).unwrap();
    let job = s.download_job.expect("the download's job");
    assert!(
        updates_discard(app.handle().clone()).is_err(),
        "a download in flight is cancelled through its job"
    );
    app.state::<crate::jobs::Jobs>().cancel(crate::jobs::JobId(job)).unwrap();
    wait_for(&r.updates, "cancelled", |p| matches!(p, UpdatePhase::Available { .. }));
    assert_eq!(crate::test_support::job_end(&app, job).state, crate::jobs::JobState::Cancelled);
    assert_eq!(r.updates.status().download_job, None);
    // Downloaded, then discarded.
    *r.fake.chunks.lock().unwrap() = Some(vec![5]);
    updates_download(app.handle().clone()).unwrap();
    wait_for(&r.updates, "ready", |p| matches!(p, UpdatePhase::Ready { .. }));
    let s = updates_discard(app.handle().clone()).unwrap();
    assert!(matches!(s.phase, UpdatePhase::Available { .. }));
}
