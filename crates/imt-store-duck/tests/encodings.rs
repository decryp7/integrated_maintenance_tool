//! Text encodings on the store: the generated samples (`samples/encodings`, invented text) are detected at import
//! with a confidence, read as their text, and a per-source choice re-imports the file and survives a reopen; the
//! profile's `[files] encoding` is the default; bad labels are refused with what to fix.

mod common;

use common::{store_with, TempDir};
use imt_core::batch::SourceId;
use imt_core::ingest::EncodingOrigin;
use imt_core::ports::{Ingest, PageQuery, Projects, Query};
use imt_core::Profile;
use imt_store_duck::DuckStore;
use std::path::PathBuf;

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/encodings")
}

fn messages(store: &DuckStore) -> Vec<String> {
    let mut rows = store
        .page(&PageQuery {
            limit: 100,
            ..PageQuery::default()
        })
        .unwrap()
        .rows;
    rows.sort_by_key(|r| r.line);
    rows.into_iter().map(|r| r.message).collect()
}

fn source(store: &DuckStore, file: &str) -> imt_core::api::SourceInfo {
    store
        .summary()
        .unwrap()
        .sources
        .into_iter()
        .find(|s| s.name.ends_with(file))
        .unwrap()
}

#[test]
fn every_sample_encoding_is_detected_and_decoded() {
    let dir = TempDir::new("enc-store");
    let (store, _) = store_with(&dir, Profile::default_profile(), &[samples()]).unwrap();
    let expect = [
        ("shift-jis.log", "Shift_JIS", "サービスを起動しました"),
        ("gbk.log", "GBK", "服务已启动"),
        ("euc-kr.log", "EUC-KR", "서비스를 시작했습니다"),
        ("big5.log", "Big5", "服務已啟動"),
        ("windows-1251.log", "windows-1251", "Служба запущена"),
        ("windows-1252.log", "windows-1252", "Dienst gestartet"),
        ("utf16le-bom.log", "UTF-16LE", "Service started"),
        ("utf16be-nomark.log", "UTF-16BE", "Service started"),
    ];
    for (file, label, first) in expect {
        let s = source(&store, file);
        let e = s.encoding.clone().unwrap_or_else(|| panic!("{file} has no encoding"));
        assert_eq!(e.label, label, "{file}");
        assert!(e.confidence >= 40, "{file}: {}", e.confidence);
        assert_eq!(s.records, 6, "{file}");
        let page = store
            .page(&PageQuery {
                limit: 100,
                ..PageQuery::default()
            })
            .unwrap();
        assert!(
            page.rows.iter().any(|r| r.message == first && r.source == SourceId(s.id)),
            "{file}: {first} not found"
        );
    }
    let bom = source(&store, "utf16le-bom.log").encoding.unwrap();
    assert_eq!((bom.origin, bom.confidence), (EncodingOrigin::Bom, 100));
}

#[test]
fn a_chosen_encoding_rereads_the_source_and_is_kept() {
    let dir = TempDir::new("enc-choice");
    let logs = TempDir::new("enc-choice-logs");
    std::fs::copy(samples().join("windows-1251.log"), logs.0.join("ru.log")).unwrap();
    let (store, _) = store_with(&dir, Profile::default_profile(), std::slice::from_ref(&logs.0)).unwrap();
    let id = source(&store, "ru.log").id;
    // Read as Windows-1252 the Cyrillic bytes are accented Latin letters: wrong, but exactly what the user asked for.
    store
        .set_source_encoding(SourceId(id), Some("windows-1252"))
        .unwrap()
        .wait()
        .unwrap();
    let s = source(&store, "ru.log");
    let e = s.encoding.unwrap();
    assert_eq!(
        (e.label.as_str(), e.confidence, e.origin),
        ("windows-1252", 100, EncodingOrigin::Manual)
    );
    assert!(!messages(&store).iter().any(|m| m.contains("Служба")));
    // The choice outlives a reopen and a reload.
    let path = store.store_dir();
    drop(store);
    let reopened = DuckStore::open(&path, Profile::default_profile(), imt_core::PluginRegistry::new()).unwrap();
    assert_eq!(source(&reopened, "ru.log").encoding.unwrap().origin, EncodingOrigin::Manual);
    // Read again (the file changed): the choice still applies.
    reopened.refresh(&[logs.0.join("ru.log")]).unwrap().wait().unwrap();
    reopened.open(std::slice::from_ref(&logs.0)).unwrap().wait().unwrap();
    let e = source(&reopened, "ru.log").encoding.unwrap();
    assert_eq!((e.label.as_str(), e.origin), ("windows-1252", EncodingOrigin::Manual));
    // Back to automatic: detected again.
    let id = source(&reopened, "ru.log").id;
    reopened.set_source_encoding(SourceId(id), None).unwrap().wait().unwrap();
    let e = source(&reopened, "ru.log").encoding.unwrap();
    assert_eq!((e.label.as_str(), e.origin), ("windows-1251", EncodingOrigin::Detected));
    assert!(messages(&reopened).iter().any(|m| m == "Служба запущена"));
}

#[test]
fn bad_choices_are_refused_with_what_to_fix() {
    let dir = TempDir::new("enc-bad");
    let (store, _) = store_with(&dir, Profile::default_profile(), &[samples()]).unwrap();
    let id = source(&store, "gbk.log").id;
    let err = store.set_source_encoding(SourceId(id), Some("klingon")).err().unwrap();
    assert!(err.to_string().contains("klingon"), "{err}");
    assert!(matches!(
        store.set_source_encoding(SourceId(9_999), Some("GBK")),
        Err(imt_core::ports::StoreError::NotFound(_))
    ));
    // The refused request changed nothing.
    assert_eq!(source(&store, "gbk.log").encoding.unwrap().origin, EncodingOrigin::Detected);
}

#[test]
fn the_profiles_encoding_is_the_default_for_every_file() {
    let dir = TempDir::new("enc-profile");
    let logs = TempDir::new("enc-profile-logs");
    std::fs::copy(samples().join("shift-jis.log"), logs.0.join("a.log")).unwrap();
    let mut profile = Profile::default_profile();
    profile.files.encoding = "Shift_JIS".into();
    let (store, _) = store_with(&dir, profile, std::slice::from_ref(&logs.0)).unwrap();
    let e = source(&store, "a.log").encoding.unwrap();
    assert_eq!(
        (e.label.as_str(), e.origin, e.confidence),
        ("Shift_JIS", EncodingOrigin::Profile, 100)
    );
    assert!(messages(&store).iter().any(|m| m == "サービスを起動しました"));
}

#[test]
fn a_profile_with_an_unknown_encoding_does_not_compile() {
    let mut profile = Profile::default_profile();
    profile.files.encoding = "klingon".into();
    let err = imt_core::profile::CompiledProfile::compile(profile).err().unwrap();
    assert!(format!("{err:#}").contains("files.encoding"), "{err:#}");
}
