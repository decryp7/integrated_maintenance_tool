//! "Download manually": when IMT cannot download a model or an inference runtime itself (a TLS-inspecting
//! proxy, a blocked host), the user downloads the exact files in their browser and hands them to IMT, which
//! checks them against the catalog. This computes what to download; the addresses come only from the
//! catalogs, never from the UI.

use crate::catalog::CatalogModel;
use crate::runtimes::RuntimeBuild;
use serde::Serialize;

/// One file to download by hand.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualFile {
    pub name: String,
    /// An https address the browser downloads the file from.
    pub url: String,
    pub size_bytes: u64,
    /// The expected SHA-256 (lowercase hex), when known.
    pub sha256: Option<String>,
}

fn sha(s: &str) -> Option<String> {
    (s.len() == 64 && s.chars().all(|c| c.is_ascii_hexdigit())).then(|| s.to_ascii_lowercase())
}

/// The model file of a catalog entry on `endpoint` (Hugging Face or the configured mirror) at `revision`.
pub fn model_file(m: &CatalogModel, endpoint: &str, revision: &str) -> ManualFile {
    ManualFile {
        name: m.file.clone(),
        url: format!(
            "{}/{}/resolve/{}/{}?download=true",
            endpoint.trim_end_matches('/'),
            m.hf_repo,
            revision,
            m.file
        ),
        size_bytes: m.size_bytes,
        sha256: sha(&m.sha256),
    }
}

/// The image projector of a catalog entry that reads images (ADR 0025), from the same repository; it is added with
/// "Add projector file…" in the model manager once downloaded.
pub fn projector_file(m: &CatalogModel, endpoint: &str, revision: &str) -> Option<ManualFile> {
    let p = m.projector.as_ref()?;
    Some(ManualFile {
        name: p.file.clone(),
        url: format!(
            "{}/{}/resolve/{}/{}?download=true",
            endpoint.trim_end_matches('/'),
            m.hf_repo,
            revision,
            p.file
        ),
        size_bytes: p.size_bytes,
        sha256: sha(&p.sha256),
    })
}

/// The archives of a runtime build, from the pinned GitHub release.
pub fn runtime_files(b: &RuntimeBuild) -> Vec<ManualFile> {
    b.assets
        .iter()
        .map(|a| ManualFile {
            name: a.name.clone(),
            url: a.url.clone(),
            size_bytes: a.size,
            sha256: sha(&a.sha256),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::Catalog;
    use crate::runtimes::{current_platform, RuntimeCatalog};

    #[test]
    fn model_addresses_come_from_the_catalog_and_the_endpoint() {
        let c = Catalog::embedded();
        let m = c.find("qwen3.5-9b-q4km").unwrap();
        let f = model_file(m, "https://huggingface.co/", "main");
        assert_eq!(
            f.url,
            format!(
                "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/{}?download=true",
                m.file
            )
        );
        assert_eq!((f.name.as_str(), f.size_bytes), (m.file.as_str(), m.size_bytes));
        assert_eq!(f.sha256.as_deref(), Some(m.sha256.to_ascii_lowercase().as_str()));
        let p = projector_file(m, "https://huggingface.co", "main").unwrap();
        assert_eq!(
            p.url,
            "https://huggingface.co/unsloth/Qwen3.5-9B-GGUF/resolve/main/mmproj-F16.gguf?download=true"
        );
        assert_eq!(p.size_bytes, m.projector.as_ref().unwrap().size_bytes);
        assert!(projector_file(c.find("qwen3.5-2b-q4km").unwrap(), "https://huggingface.co", "main").is_none());
        let mut odd = m.clone();
        odd.sha256 = "unknown".into();
        assert_eq!(model_file(&odd, "https://mirror.corp", "v1").sha256, None);
        assert!(model_file(&odd, "https://mirror.corp", "v1")
            .url
            .starts_with("https://mirror.corp/unsloth/"));
    }

    #[test]
    fn runtime_archives_are_the_pinned_release_assets() {
        let cat = RuntimeCatalog::embedded();
        let platform = if cat.platforms.contains_key(&current_platform()) {
            current_platform()
        } else {
            "windows-x64".into()
        };
        for b in &cat.platforms[&platform] {
            let files = runtime_files(b);
            assert_eq!(files.len(), b.assets.len());
            for (f, a) in files.iter().zip(&b.assets) {
                assert!(f.url.starts_with("https://github.com/ggml-org/llama.cpp/releases/download/"));
                assert_eq!(f.sha256.as_deref(), Some(a.sha256.as_str()));
                assert_eq!(f.size_bytes, a.size);
            }
        }
    }
}
