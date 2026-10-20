//! Checking a model file the user downloaded themselves (for example in the browser, when IMT's own download
//! is blocked by a proxy) against the catalog before it is added: a file named like a catalog model, or of
//! exactly its size, must have the catalog's SHA-256.

use super::download::sha256_file;
use super::source::{Phase, Progress};
use crate::catalog::{Catalog, CatalogModel};
use crate::error::{AiError, Result};
use std::path::Path;

/// Which catalog models a local file could be, from its name and size alone.
#[derive(Debug, Clone, PartialEq)]
pub enum LocalMatch<'a> {
    /// No catalog model has this name or size: it is the user's own model, added without a checksum.
    Unknown,
    /// Named like `model` but of another size: an incomplete download or a different file.
    WrongSize(&'a CatalogModel),
    /// It may be one of these (named like the first, or of the same size); its checksum decides.
    Candidates { named: bool, models: Vec<&'a CatalogModel> },
}

/// Matches a file name (compared case-insensitively) and size against the catalog. Pure.
pub fn match_local<'a>(catalog: &'a Catalog, file_name: &str, size: u64) -> LocalMatch<'a> {
    let named = catalog.models.iter().find(|m| m.file.eq_ignore_ascii_case(file_name));
    if let Some(m) = named {
        if m.size_bytes != size {
            return LocalMatch::WrongSize(m);
        }
    }
    let mut models: Vec<&CatalogModel> = named.into_iter().collect();
    for m in catalog.models.iter().filter(|m| m.size_bytes == size) {
        if !models.iter().any(|n| n.id == m.id) {
            models.push(m);
        }
    }
    if models.is_empty() {
        LocalMatch::Unknown
    } else {
        LocalMatch::Candidates {
            named: named.is_some(),
            models,
        }
    }
}

fn gb(bytes: u64) -> String {
    format!("{:.2} GB", bytes as f64 / 1e9)
}

/// Checks `path` against the catalog. Returns the catalog model it is (verified by SHA-256), or `None` for a
/// model IMT does not know. Hashing reads the whole file: call off the UI thread; `progress` reports it
/// (phase `Verifying`) and cancels it.
pub fn verify_local<'a>(catalog: &'a Catalog, path: &Path, progress: &Progress) -> Result<Option<&'a CatalogModel>> {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let size = std::fs::metadata(path)?.len();
    match match_local(catalog, &name, size) {
        LocalMatch::Unknown => Ok(None),
        LocalMatch::WrongSize(m) => Err(AiError::Integrity(format!(
            "{name} should be {} ({} bytes) but is {} ({size} bytes): the download is incomplete or it is another file; \
             download it again",
            gb(m.size_bytes),
            m.size_bytes,
            gb(size)
        ))),
        LocalMatch::Candidates { named, models } => {
            progress.begin(Phase::Verifying, size, 0);
            let got = sha256_file(path, progress)?;
            if let Some(m) = models.iter().find(|m| m.sha256.eq_ignore_ascii_case(&got)) {
                return Ok(Some(*m));
            }
            if named {
                let m = models[0];
                return Err(AiError::Integrity(format!(
                    "{name} is not the original {}: its SHA-256 is {got}, expected {}; the file is damaged or was changed, \
                     download it again",
                    m.name, m.sha256
                )));
            }
            // Only the size matched: someone else's model of the same size.
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::gguf::qwen_like;
    use sha2::{Digest, Sha256};

    fn catalog_for(bytes: &[u8], file: &str) -> Catalog {
        let mut c = Catalog::embedded();
        let mut m = c.models[0].clone();
        m.id = "mine".into();
        m.file = file.into();
        m.size_bytes = bytes.len() as u64;
        m.sha256 = Sha256::digest(bytes).iter().map(|b| format!("{b:02x}")).collect();
        c.models = vec![m, c.models[1].clone()];
        c
    }

    #[test]
    fn matching_by_name_and_size() {
        let c = catalog_for(b"abc", "Model-Q4.gguf");
        assert!(matches!(match_local(&c, "other.gguf", 7), LocalMatch::Unknown));
        assert!(matches!(match_local(&c, "model-q4.GGUF", 7), LocalMatch::WrongSize(m) if m.id == "mine"));
        match match_local(&c, "model-q4.gguf", 3) {
            LocalMatch::Candidates { named, models } => assert!(named && models.len() == 1 && models[0].id == "mine"),
            m => panic!("{m:?}"),
        }
        match match_local(&c, "renamed.gguf", 3) {
            LocalMatch::Candidates { named, models } => assert!(!named && models[0].id == "mine"),
            m => panic!("{m:?}"),
        }
    }

    #[test]
    fn a_downloaded_catalog_model_is_verified_and_a_damaged_one_rejected() {
        let tmp = tempfile::tempdir().unwrap();
        let data = qwen_like();
        let c = catalog_for(&data, "Model-Q4.gguf");
        let good = tmp.path().join("Model-Q4.gguf");
        std::fs::write(&good, &data).unwrap();
        let p = Progress::new();
        assert_eq!(verify_local(&c, &good, &p).unwrap().map(|m| m.id.as_str()), Some("mine"));
        assert_eq!(p.snapshot().bytes, data.len() as u64);
        // renamed by the browser, same bytes: still recognised
        let renamed = tmp.path().join("Model-Q4 (1).gguf");
        std::fs::write(&renamed, &data).unwrap();
        assert_eq!(
            verify_local(&c, &renamed, &Progress::new()).unwrap().map(|m| m.id.as_str()),
            Some("mine")
        );
        // same name, one byte changed
        let mut bad = data.clone();
        let last = bad.len() - 1;
        bad[last] ^= 1;
        let damaged = tmp.path().join("sub").join("Model-Q4.gguf");
        std::fs::create_dir_all(damaged.parent().unwrap()).unwrap();
        std::fs::write(&damaged, &bad).unwrap();
        let e = verify_local(&c, &damaged, &Progress::new()).unwrap_err();
        assert!(
            matches!(&e, AiError::Integrity(m) if m.contains("SHA-256") && m.contains(&c.models[0].sha256)),
            "{e}"
        );
        // same size, other name, other bytes: an unknown model
        let other = tmp.path().join("someone-else.gguf");
        std::fs::write(&other, &bad).unwrap();
        assert_eq!(verify_local(&c, &other, &Progress::new()).unwrap(), None);
        // incomplete
        let part = tmp.path().join("p").join("Model-Q4.gguf");
        std::fs::create_dir_all(part.parent().unwrap()).unwrap();
        std::fs::write(&part, &data[..10]).unwrap();
        assert!(verify_local(&c, &part, &Progress::new())
            .unwrap_err()
            .to_string()
            .contains("incomplete"));
        // cancelled while hashing
        let cancel = Progress::new();
        cancel.cancel();
        assert_eq!(verify_local(&c, &good, &cancel).unwrap_err(), AiError::Cancelled);
        // missing file
        assert!(matches!(
            verify_local(&c, &tmp.path().join("nope.gguf"), &Progress::new()),
            Err(AiError::Io(_))
        ));
    }
}
