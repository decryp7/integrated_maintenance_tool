//! The list of models IMT recommends, as data (`data/local-models.json`). The user can replace it
//! with a newer file; entries that do not validate are skipped with a warning, never trusted.

use serde::{Deserialize, Serialize};
use std::path::Path;

pub const EMBEDDED_CATALOG: &str = include_str!("../data/local-models.json");

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogModel {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub family: String,
    /// Total parameters in billions.
    pub params_b: f64,
    #[serde(default)]
    pub active_params_b: Option<f64>,
    pub quant: String,
    pub hf_repo: String,
    pub file: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub license: String,
    /// The model family is trained for tool calling and follows JSON schemas well.
    #[serde(default)]
    pub tool_calling: bool,
    #[serde(default = "default_ctx")]
    pub context_default: usize,
    #[serde(default)]
    pub min_vram_gb: f64,
    #[serde(default)]
    pub min_ram_gb: f64,
    /// Mixture of experts: expert tensors can stay in system RAM.
    #[serde(default)]
    pub moe: bool,
    #[serde(default)]
    pub recommended_for: Vec<String>,
    #[serde(default)]
    pub notes: String,
}

fn default_ctx() -> usize {
    4096
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub version: u32,
    #[serde(default)]
    pub updated: String,
    #[serde(default)]
    pub note: String,
    pub models: Vec<CatalogModel>,
}

/// A file name that is safe to use inside the models folder.
pub fn is_safe_file_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && name.to_lowercase().ends_with(".gguf")
        && !name.starts_with('.')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
        && !name.contains("..")
}

fn problem(m: &CatalogModel) -> Option<String> {
    if !is_safe_file_name(&m.file) {
        return Some(format!("{}: unsafe file name", m.id));
    }
    let parts: Vec<&str> = m.hf_repo.split('/').collect();
    let ok_part = |p: &str| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if parts.len() != 2 || !parts.iter().all(|p| ok_part(p)) || m.hf_repo.contains("..") {
        return Some(format!("{}: invalid repository", m.id));
    }
    if m.sha256.len() != 64 || !m.sha256.chars().all(|c| c.is_ascii_hexdigit()) {
        return Some(format!("{}: invalid sha256", m.id));
    }
    if m.size_bytes == 0 || !(m.params_b.is_finite() && m.params_b > 0.0) {
        return Some(format!("{}: invalid size", m.id));
    }
    None
}

impl Catalog {
    pub fn embedded() -> Catalog {
        // The embedded file is covered by tests, so a failure here is a build defect, not user data.
        Catalog::parse(EMBEDDED_CATALOG).map(|(c, _)| c).unwrap_or(Catalog {
            version: 1,
            updated: String::new(),
            note: String::new(),
            models: vec![],
        })
    }

    /// Parses catalog JSON; returns the valid entries and a warning per skipped one.
    pub fn parse(json: &str) -> Result<(Catalog, Vec<String>), String> {
        let mut c: Catalog = serde_json::from_str(json).map_err(|e| format!("the model catalog is not valid: {e}"))?;
        if c.version != 1 {
            return Err(format!("unsupported catalog version {}", c.version));
        }
        let mut warnings = Vec::new();
        c.models.retain(|m| match problem(m) {
            Some(w) => {
                warnings.push(w);
                false
            }
            None => true,
        });
        Ok((c, warnings))
    }

    /// Loads a user-updated catalog, falling back to the embedded one when it is missing or broken.
    pub fn load(path: &Path) -> (Catalog, Vec<String>) {
        match std::fs::read_to_string(path) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Catalog::embedded(), vec![]),
            Err(e) => (
                Catalog::embedded(),
                vec![format!("cannot read {}: {e}; using the built-in list", path.display())],
            ),
            Ok(text) => match Catalog::parse(&text) {
                Ok((c, w)) if !c.models.is_empty() => (c, w),
                Ok((_, mut w)) => {
                    w.push("the catalog has no usable models; using the built-in list".into());
                    (Catalog::embedded(), w)
                }
                Err(e) => (Catalog::embedded(), vec![format!("{e}; using the built-in list")]),
            },
        }
    }

    pub fn find(&self, id: &str) -> Option<&CatalogModel> {
        self.models.iter().find(|m| m.id == id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_catalog_is_valid_and_complete() {
        let (c, warnings) = Catalog::parse(EMBEDDED_CATALOG).unwrap();
        assert!(warnings.is_empty(), "{warnings:?}");
        assert!(c.models.len() >= 6);
        let q = c.find("qwen3.5-9b-q4km").unwrap();
        assert_eq!(q.hf_repo, "unsloth/Qwen3.5-9B-GGUF");
        assert!(q.tool_calling);
        assert_eq!(Catalog::embedded(), c);
        assert!(c.find("nope").is_none());
    }

    #[test]
    fn bad_entries_are_skipped_with_warnings() {
        let mut c = Catalog::embedded();
        let good = c.models[0].clone();
        let mut bad_name = good.clone();
        bad_name.id = "a".into();
        bad_name.file = "../evil.gguf".into();
        let mut bad_repo = good.clone();
        bad_repo.id = "b".into();
        bad_repo.hf_repo = "no-slash".into();
        let mut bad_sha = good.clone();
        bad_sha.id = "c".into();
        bad_sha.sha256 = "zz".into();
        let mut bad_size = good.clone();
        bad_size.id = "d".into();
        bad_size.size_bytes = 0;
        c.models = vec![good, bad_name, bad_repo, bad_sha, bad_size];
        let (parsed, w) = Catalog::parse(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(parsed.models.len(), 1);
        assert_eq!(w.len(), 4);
    }

    #[test]
    fn rejects_unknown_versions_and_garbage() {
        assert!(Catalog::parse("{").is_err());
        assert!(Catalog::parse("{\"version\":2,\"models\":[]}").unwrap_err().contains("version"));
    }

    #[test]
    fn load_falls_back() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("c.json");
        assert_eq!(Catalog::load(&p).0, Catalog::embedded());
        assert!(Catalog::load(&p).1.is_empty());
        std::fs::write(&p, "garbage").unwrap();
        let (c, w) = Catalog::load(&p);
        assert_eq!(c, Catalog::embedded());
        assert!(w[0].contains("built-in"));
        std::fs::write(&p, "{\"version\":1,\"models\":[]}").unwrap();
        assert!(Catalog::load(&p).1[0].contains("no usable"));
        let mut mine = Catalog::embedded();
        mine.models.truncate(1);
        mine.updated = "later".into();
        std::fs::write(&p, serde_json::to_string(&mine).unwrap()).unwrap();
        assert_eq!(Catalog::load(&p).0.updated, "later");
        assert!(Catalog::load(dir.path()).1[0].contains("cannot read"));
    }

    #[test]
    fn file_name_rules() {
        assert!(is_safe_file_name("Qwen3.5-9B-Q4_K_M.gguf"));
        for bad in [
            "",
            "a/b.gguf",
            "a\\b.gguf",
            "..gguf",
            ".hidden.gguf",
            "x.bin",
            "a b.gguf",
            "a..b.gguf",
        ] {
            assert!(!is_safe_file_name(bad), "{bad}");
        }
        assert!(!is_safe_file_name(&format!("{}.gguf", "a".repeat(300))));
    }
}
