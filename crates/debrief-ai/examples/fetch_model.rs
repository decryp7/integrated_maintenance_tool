//! Downloads a catalog model with Debrief's own downloader (resumable, checksum-verified).
//!
//!   cargo run -p debrief-ai --example fetch_model -- <catalog-id> [--dir <folder>]
//!
//! Without an id it lists the catalog.

use debrief_ai::catalog::Catalog;
use debrief_ai::models::*;
use std::path::PathBuf;
use std::sync::Arc;

fn main() {
    let mut args = std::env::args().skip(1);
    let catalog = Catalog::embedded();
    let Some(id) = args.next() else {
        for m in &catalog.models {
            println!("{:32} {:>6.1} GB  {}/{}", m.id, m.size_bytes as f64 / 1e9, m.hf_repo, m.file);
        }
        return;
    };
    let mut dir = PathBuf::from("models");
    while let Some(a) = args.next() {
        if a == "--dir" {
            dir = PathBuf::from(args.next().expect("--dir needs a folder"));
        }
    }
    let m = catalog.find(&id).unwrap_or_else(|| panic!("unknown catalog id {id}")).clone();
    let hf = HuggingFace::new(HfConfig::default()).expect("model host");
    let remote = hf.describe(&m.hf_repo, &m.file).expect("describe");
    println!(
        "{} ({:.1} GB, licence {:?}) -> {}",
        m.file,
        remote.size as f64 / 1e9,
        remote.license,
        dir.display()
    );
    let progress = Arc::new(Progress::new());
    let p = progress.clone();
    let ticker = std::thread::spawn(move || {
        while p.snapshot().bytes < p.snapshot().total.max(1) && !p.is_cancelled() {
            let s = p.snapshot();
            eprintln!(
                "{:?} {:.0}% {:.0} MB/s",
                s.phase,
                100.0 * s.bytes as f64 / s.total.max(1) as f64,
                s.bytes_per_sec / 1e6
            );
            std::thread::sleep(std::time::Duration::from_secs(15));
        }
    });
    let opts = DownloadOptions {
        max_bytes: m.size_bytes.max(DownloadOptions::default().max_bytes),
        ..DownloadOptions::default()
    };
    let path = Downloader {
        source: &hf,
        disk: &SystemDisk,
        opts,
    }
    .download(&remote, &dir, &progress)
    .expect("download");
    progress.cancel();
    let _ = ticker.join();
    println!("ready: {}", path.display());
}
