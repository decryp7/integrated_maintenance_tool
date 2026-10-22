//! Which installed runtimes and models have an update, from the pinned catalogs only.
//!
//! - A **runtime** update: a build installed from an older release than the one Debrief pins, while the pinned
//!   release offers the same build for this platform and it is not installed yet. The new build comes with
//!   the SHA-256 of every asset from `llama-runtimes.json`; nothing else is ever downloaded.
//! - A **model** update: a catalog entry that lists an installed file in `supersedes` (an earlier revision or
//!   quantisation of the same model) while its own file is not installed. Before it is offered, the model host
//!   must confirm that it serves exactly the pinned file ([`confirm_model`]).

use super::policy::UpdateKind;
use crate::catalog::{Catalog, CatalogModel};
use crate::error::{AiError, Result};
use crate::models::ModelSource;
use crate::runtimes::builds::{label, tag_number};
use crate::runtimes::{InstalledRuntime, RuntimeCatalog};
use serde::Serialize;

/// One update that can be installed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateCandidate {
    pub kind: UpdateKind,
    /// Stable for one target version: what "Skip this version" remembers.
    pub key: String,
    /// The build id or the catalog model id (also what "Download manually" takes).
    pub id: String,
    /// The build's label or the model's name.
    pub name: String,
    /// The installed release tag or model file.
    pub from: String,
    /// The release tag or model file it updates to.
    pub to: String,
    pub download_bytes: u64,
}

/// Runtime updates: per build id, the newest older installed release when the pinned one is not installed.
pub fn runtime_candidates(installed: &[InstalledRuntime], catalog: &RuntimeCatalog, platform: &str) -> Vec<UpdateCandidate> {
    let Some(pinned) = tag_number(&catalog.tag) else {
        return vec![];
    };
    let mut out: Vec<UpdateCandidate> = Vec::new();
    for build in catalog.builds(platform) {
        if installed.iter().any(|r| r.id == build.id && r.tag == catalog.tag) {
            continue;
        }
        let older = installed
            .iter()
            .filter(|r| r.id == build.id)
            .filter_map(|r| tag_number(&r.tag).filter(|n| *n < pinned).map(|n| (n, r)))
            .max_by_key(|(n, _)| *n);
        if let Some((_, old)) = older {
            out.push(UpdateCandidate {
                kind: UpdateKind::Runtime,
                key: format!("runtime:{}@{}", build.id, catalog.tag),
                id: build.id.clone(),
                name: label(&build.id),
                from: old.tag.clone(),
                to: catalog.tag.clone(),
                download_bytes: build.download_bytes(),
            });
        }
    }
    out
}

/// Model updates: catalog entries replacing an installed file whose own file is not installed.
pub fn model_candidates(installed: &[String], catalog: &Catalog) -> Vec<UpdateCandidate> {
    catalog
        .models
        .iter()
        .filter(|m| !installed.contains(&m.file))
        .filter_map(|m| {
            let old = m.supersedes.iter().find(|f| installed.contains(f))?;
            Some(UpdateCandidate {
                kind: UpdateKind::Model,
                key: format!("model:{}@{}", m.id, m.sha256[..12.min(m.sha256.len())].to_ascii_lowercase()),
                id: m.id.clone(),
                name: m.name.clone(),
                from: old.clone(),
                to: m.file.clone(),
                download_bytes: m.size_bytes,
            })
        })
        .collect()
}

/// Asks the model host about the pinned file (no download): it must serve exactly the pinned size and
/// SHA-256. Network failures are returned as they are (the check is retried later); a different file is
/// [`AiError::Integrity`] (the update waits for a corrected catalog).
pub fn confirm_model(source: &dyn ModelSource, m: &CatalogModel) -> Result<()> {
    let remote = source.describe(&m.hf_repo, &m.file)?;
    if remote.sha256 != m.sha256.to_ascii_lowercase() || remote.size != m.size_bytes {
        return Err(AiError::Integrity(format!(
            "the model host serves a different {} than the model list pins; the update waits for a corrected list",
            m.file
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::RemoteFile;
    use crate::runtimes::{RuntimeAsset, RuntimeBuild};
    use std::path::PathBuf;
    use std::sync::atomic::AtomicBool;

    fn runtime_catalog(tag: &str) -> RuntimeCatalog {
        let asset = |name: &str| RuntimeAsset {
            name: name.into(),
            url: format!("https://github.com/ggml-org/llama.cpp/releases/download/{tag}/{name}"),
            size: 10,
            sha256: "0".repeat(64),
        };
        RuntimeCatalog {
            version: 1,
            release: "v1".into(),
            tag: tag.into(),
            commit: String::new(),
            published: String::new(),
            source: String::new(),
            platforms: [(
                "windows-x64".to_string(),
                vec![
                    RuntimeBuild {
                        id: "cpu".into(),
                        assets: vec![asset("cpu.zip")],
                    },
                    RuntimeBuild {
                        id: "cuda-13".into(),
                        assets: vec![asset("cuda.zip"), asset("cudart.zip")],
                    },
                    RuntimeBuild {
                        id: "vulkan".into(),
                        assets: vec![asset("vulkan.zip")],
                    },
                ],
            )]
            .into_iter()
            .collect(),
        }
    }

    fn rt(tag: &str, id: &str, current: bool) -> InstalledRuntime {
        InstalledRuntime {
            id: id.into(),
            tag: tag.into(),
            dir: PathBuf::new(),
            bytes: 1,
            current,
        }
    }

    #[test]
    fn older_installed_builds_get_the_pinned_release() {
        let c = runtime_catalog("b200");
        let installed = [
            rt("b100", "cpu", false),
            rt("b150", "cpu", false),
            rt("b100", "cuda-13", false),
            rt("b200", "cuda-13", true),
            rt("b300", "vulkan", false),
            rt("b100", "rocm", false),
        ];
        let u = runtime_candidates(&installed, &c, "windows-x64");
        assert_eq!(u.len(), 1, "{u:?}");
        let cpu = &u[0];
        assert_eq!(
            (cpu.key.as_str(), cpu.id.as_str(), cpu.from.as_str(), cpu.to.as_str()),
            ("runtime:cpu@b200", "cpu", "b150", "b200")
        );
        assert_eq!((cpu.kind, cpu.download_bytes, cpu.name.as_str()), (UpdateKind::Runtime, 10, "CPU"));
        assert!(runtime_candidates(&installed, &c, "plan9-mips").is_empty());
        assert!(runtime_candidates(&[], &c, "windows-x64").is_empty());
        let mut odd = c.clone();
        odd.tag = "release".into();
        assert!(
            runtime_candidates(&installed, &odd, "windows-x64").is_empty(),
            "an unnumbered tag offers nothing"
        );
    }

    fn catalog_with(supersedes: &[&str]) -> Catalog {
        let mut c = Catalog::embedded();
        c.models.truncate(2);
        c.models[0].supersedes = supersedes.iter().map(|s| s.to_string()).collect();
        c
    }

    #[test]
    fn a_replaced_installed_file_gets_its_catalog_entry() {
        let c = catalog_with(&["Old-Q4_K_M.gguf", "Older-Q4_K_M.gguf"]);
        let m = &c.models[0];
        let u = model_candidates(&["Older-Q4_K_M.gguf".into(), "mine.gguf".into()], &c);
        assert_eq!(u.len(), 1);
        assert_eq!(u[0].key, format!("model:{}@{}", m.id, &m.sha256[..12]));
        assert_eq!((u[0].from.as_str(), u[0].to.as_str()), ("Older-Q4_K_M.gguf", m.file.as_str()));
        assert_eq!((u[0].kind, u[0].download_bytes), (UpdateKind::Model, m.size_bytes));
        assert!(
            model_candidates(&["Older-Q4_K_M.gguf".into(), m.file.clone()], &c).is_empty(),
            "the new file is already installed"
        );
        assert!(model_candidates(&["mine.gguf".into()], &c).is_empty());
        assert!(model_candidates(&["Old-Q4_K_M.gguf".into()], &Catalog::embedded()).is_empty());
    }

    struct Host(std::result::Result<(u64, String), AiError>);
    impl ModelSource for Host {
        fn describe(&self, repo: &str, file: &str) -> Result<RemoteFile> {
            let (size, sha256) = self.0.clone()?;
            Ok(RemoteFile {
                repo: repo.into(),
                file: file.into(),
                size,
                sha256,
                license: None,
                gated: false,
            })
        }
        fn fetch(&self, _: &RemoteFile, _: u64, _: &mut dyn FnMut(&[u8]) -> Result<()>, _: &AtomicBool) -> Result<()> {
            panic!("a check never downloads")
        }
    }

    #[test]
    fn the_host_must_serve_the_pinned_file() {
        let m = Catalog::embedded().models[0].clone();
        assert_eq!(confirm_model(&Host(Ok((m.size_bytes, m.sha256.to_ascii_lowercase()))), &m), Ok(()));
        let e = confirm_model(&Host(Ok((m.size_bytes, "f".repeat(64)))), &m).unwrap_err();
        assert!(matches!(e, AiError::Integrity(_)), "{e}");
        assert!(matches!(
            confirm_model(&Host(Ok((1, m.sha256.clone()))), &m),
            Err(AiError::Integrity(_))
        ));
        let offline = confirm_model(&Host(Err(AiError::Network("offline".into()))), &m).unwrap_err();
        assert!(offline.is_transient());
    }
}
