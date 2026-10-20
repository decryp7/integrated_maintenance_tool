//! Where llama-server comes from: a custom path from Settings → AI, else the builds the user downloaded
//! (`IMT_LLAMA_DIR` = `<data>/ai/runtimes/<tag>/`, holding `<id>/llama-server`). Nothing is bundled with
//! IMT. IMT passes these in the worker's environment (see `imt_ai::settings::AiSettings::worker_env`).

use crate::variants::{installed_in, BuildChoice, Installed};
use imt_ai::worker::protocol::{code, RpcError};
use std::path::{Path, PathBuf};

/// Environment variable with a custom llama-server executable (setting `llama_server_path`).
pub const ENV_SERVER: &str = "IMT_LLAMA_SERVER";
/// Environment variable with the chosen build (setting `inference_build`): `auto` or an id.
pub const ENV_BUILD: &str = "IMT_LLAMA_BUILD";
/// Environment variable with the folder holding the downloaded builds (`<id>/llama-server`).
pub const ENV_DIR: &str = "IMT_LLAMA_DIR";

/// Where to look, parsed once from the environment at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LlamaConfig {
    pub custom: Option<PathBuf>,
    /// Folders that may hold `<id>/llama-server`, first match wins.
    pub roots: Vec<PathBuf>,
    pub choice: BuildChoice,
}

/// The installed builds, or why there are none.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Found {
    /// The folder the builds were found in (`None` for a custom path).
    pub root: Option<PathBuf>,
    pub builds: Vec<Installed>,
}

/// `data.reason` of the error when no build is installed, so IMT can offer the download.
pub const NO_RUNTIME: &str = "no_runtime";

pub fn not_found() -> RpcError {
    RpcError {
        code: code::LOAD_FAILED,
        message: "no inference runtime (llama-server) is installed; download one in the AI set-up, or set a custom llama-server path in Settings → AI".into(),
        data: Some(serde_json::json!({ "reason": NO_RUNTIME }).into()),
    }
}

impl LlamaConfig {
    /// From the worker's environment and its own location.
    pub fn from_env() -> LlamaConfig {
        let var = |k: &str| std::env::var(k).ok().filter(|v| !v.trim().is_empty());
        LlamaConfig {
            custom: var(ENV_SERVER).map(PathBuf::from),
            roots: var(ENV_DIR).map(PathBuf::from).into_iter().collect(),
            choice: BuildChoice::parse(&var(ENV_BUILD).unwrap_or_default()),
        }
    }

    /// Finds llama-server. A custom path that is not a file is an error (it is never silently ignored).
    pub fn find(&self) -> Result<Found, RpcError> {
        if let Some(p) = &self.custom {
            let exe = std::fs::canonicalize(p).ok().filter(|c| c.is_file()).ok_or_else(|| {
                RpcError::new(
                    code::LOAD_FAILED,
                    format!(
                        "the custom llama-server ({}) was not found; fix or clear the path in Settings → AI",
                        p.display()
                    ),
                )
            })?;
            return Ok(Found {
                root: None,
                builds: vec![Installed { id: "custom".into(), exe }],
            });
        }
        for root in &self.roots {
            let builds = installed_in(root);
            if !builds.is_empty() {
                return Ok(Found {
                    root: Some(root.clone()),
                    builds,
                });
            }
        }
        Err(not_found())
    }
}

/// The release the installed builds come from (`manifest.json`, written by `imt_ai::runtimes::RuntimeStore`).
pub fn installed_release(root: &Path) -> Option<String> {
    let text = std::fs::read_to_string(root.join("manifest.json")).ok()?;
    let v: serde_json::Value = serde_json::from_str(&text).ok()?;
    v.get("tag").and_then(|t| t.as_str()).map(str::to_string)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::variants::server_file_name;

    fn layout(ids: &[&str]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for id in ids {
            std::fs::create_dir_all(dir.path().join(id)).unwrap();
            std::fs::write(dir.path().join(id).join(server_file_name()), b"x").unwrap();
        }
        dir
    }

    #[test]
    fn first_root_with_builds_wins() {
        let empty = tempfile::tempdir().unwrap();
        let full = layout(&["cpu", "vulkan"]);
        let cfg = LlamaConfig {
            custom: None,
            roots: vec![empty.path().into(), full.path().into()],
            choice: BuildChoice::Auto,
        };
        let f = cfg.find().unwrap();
        assert_eq!(f.root.as_deref(), Some(full.path()));
        assert_eq!(f.builds.len(), 2);
    }

    #[test]
    fn nothing_found_says_what_to_do() {
        let cfg = LlamaConfig {
            custom: None,
            roots: vec![],
            choice: BuildChoice::Auto,
        };
        let e = cfg.find().unwrap_err();
        assert_eq!(e.code, code::LOAD_FAILED);
        assert!(e.message.contains("no inference runtime") && e.message.contains("Settings → AI"));
        assert_eq!(e.data.unwrap()["reason"], NO_RUNTIME);
    }

    #[test]
    fn a_custom_path_must_exist() {
        let full = layout(&["cpu"]);
        let mut cfg = LlamaConfig {
            custom: Some(full.path().join("nope")),
            roots: vec![full.path().into()],
            choice: BuildChoice::Auto,
        };
        assert!(cfg.find().unwrap_err().message.contains("custom llama-server"));
        // a folder is not an executable
        cfg.custom = Some(full.path().join("cpu"));
        assert!(cfg.find().is_err());
        cfg.custom = Some(full.path().join("cpu").join(server_file_name()));
        let f = cfg.find().unwrap();
        assert_eq!((f.root, f.builds[0].id.as_str()), (None, "custom"));
    }

    #[test]
    fn release_tag() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(installed_release(dir.path()), None);
        std::fs::write(dir.path().join("manifest.json"), "{ broken").unwrap();
        assert_eq!(installed_release(dir.path()), None);
        std::fs::write(dir.path().join("manifest.json"), r#"{"tag":"b11669"}"#).unwrap();
        assert_eq!(installed_release(dir.path()).as_deref(), Some("b11669"));
        // from_env never fails, whatever the environment holds
        let _ = LlamaConfig::from_env();
    }
}
