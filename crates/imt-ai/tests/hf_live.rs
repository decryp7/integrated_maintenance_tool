//! Real Hugging Face download. Ignored by default (it uses the network and downloads 0.5 GB):
//!
//!   IMT_TEST_HF=1 cargo test -p imt-ai --test hf_live -- --ignored --nocapture

use imt_ai::catalog::Catalog;
use imt_ai::models::*;

#[test]
#[ignore]
fn downloads_and_verifies_the_smallest_catalog_model() {
    if std::env::var("IMT_TEST_HF").as_deref() != Ok("1") {
        panic!("set IMT_TEST_HF=1 to run this test");
    }
    let catalog = Catalog::embedded();
    let m = catalog.models.iter().min_by_key(|m| m.size_bytes).unwrap();
    let hf = HuggingFace::new(HfConfig::default()).unwrap();
    // metadata first: the host agrees with the catalog about size and checksum
    let remote = hf.describe(&m.hf_repo, &m.file).unwrap();
    println!(
        "{} {} bytes sha256 {} licence {:?}",
        m.file, remote.size, remote.sha256, remote.license
    );
    assert_eq!((remote.size, remote.sha256.as_str()), (m.size_bytes, m.sha256.as_str()));
    let dir = tempfile::tempdir().unwrap();
    let progress = Progress::new();
    let path = Downloader {
        source: &hf,
        disk: &SystemDisk,
        opts: DownloadOptions::default(),
    }
    .download(&remote, dir.path(), &progress)
    .unwrap();
    assert_eq!(std::fs::metadata(&path).unwrap().len(), m.size_bytes);
    let info = imt_ai::gguf::inspect(&path).unwrap();
    println!("downloaded and verified: {} ({})", info.architecture, path.display());
}
