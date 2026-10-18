//! Inference builds ("variants"): the prebuilt llama-server builds the user downloaded, one folder each
//! (`<data>/ai/runtimes/<tag>/<id>/`, see `imt_ai::runtimes`). Ids, labels and the order they are tried in
//! are shared with IMT (`imt_ai::runtimes::builds`); this module finds them on disk.

pub use imt_ai::runtimes::builds::{is_build_id as is_valid_id, label, needs_gpu, try_order, BuildChoice};
use std::path::{Path, PathBuf};

/// The executable's file name on this platform.
pub fn server_file_name() -> &'static str {
    if cfg!(windows) {
        "llama-server.exe"
    } else {
        "llama-server"
    }
}

/// An installed build on disk.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Installed {
    pub id: String,
    pub exe: PathBuf,
}

/// The builds under `root` (`root/<id>/llama-server[.exe]`), sorted by id. Unreadable roots give none.
pub fn installed_in(root: &Path) -> Vec<Installed> {
    let Ok(entries) = std::fs::read_dir(root) else {
        return Vec::new();
    };
    let mut out: Vec<Installed> = entries
        .flatten()
        .filter_map(|e| {
            let id = e.file_name().to_str()?.to_string();
            let exe = e.path().join(server_file_name());
            (is_valid_id(&id) && exe.is_file()).then_some(Installed { id, exe })
        })
        .collect();
    out.sort_by(|a, b| a.id.cmp(&b.id));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ids(v: &[&str]) -> Vec<String> {
        v.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn the_shared_rules_apply() {
        assert!(is_valid_id("cuda-13") && !is_valid_id("../x"));
        assert_eq!(try_order(&ids(&["cpu", "vulkan"]), &BuildChoice::Auto), ids(&["vulkan", "cpu"]));
        assert!(needs_gpu("vulkan") && label("cpu") == "CPU");
    }

    #[test]
    fn finds_installed_builds() {
        let dir = tempfile::tempdir().unwrap();
        for id in ["cpu", "vulkan", "Bad Name"] {
            std::fs::create_dir_all(dir.path().join(id)).unwrap();
            std::fs::write(dir.path().join(id).join(server_file_name()), b"x").unwrap();
        }
        std::fs::create_dir_all(dir.path().join("empty")).unwrap();
        std::fs::write(dir.path().join("manifest.json"), b"{}").unwrap();
        let found: Vec<String> = installed_in(dir.path()).into_iter().map(|i| i.id).collect();
        assert_eq!(found, ids(&["cpu", "vulkan"]));
        assert!(installed_in(&dir.path().join("missing")).is_empty());
    }
}
