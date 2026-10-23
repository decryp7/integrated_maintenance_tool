//! "Download manually" for a model or an inference runtime that Debrief cannot download itself (a TLS-inspecting
//! proxy such as Zscaler, a blocked host): which files to get, from where, with which SHA-256, and where Debrief
//! keeps them once checked. The UI names only the model or build id; the addresses come from the catalogs,
//! so the webview can never make Debrief open an arbitrary URL.

use super::commands::{check_link, core};
use super::core::AiCore;
use super::error::{AiCmdError, AiResult};
use debrief_ai::manual::{model_file, projector_file, runtime_files, ManualFile};
use debrief_ai::runtimes::builds::{is_build_id, label};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Runtime};

const MAX_ID_CHARS: usize = 64;
/// The revision model files are downloaded from (as `HfConfig::default().revision`).
const MODEL_REVISION: &str = "main";

/// What is downloaded by hand.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ManualKind {
    /// A catalog model (`id` = catalog id).
    Model,
    /// An inference runtime build of this platform (`id` = build id).
    Runtime,
}

/// The files of a manual download and where Debrief keeps them after the check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ManualDownload {
    pub kind: ManualKind,
    pub id: String,
    /// The model's name or the build's label.
    pub name: String,
    pub files: Vec<ManualFile>,
    /// The models folder, or the folder the build is unpacked into.
    pub folder: String,
}

impl AiCore {
    /// The files to download by hand for `id`. Fails for ids that are not in the catalogs.
    pub fn manual_download(&self, kind: ManualKind, id: &str) -> AiResult<ManualDownload> {
        if id.is_empty() || id.len() > MAX_ID_CHARS {
            return Err(AiCmdError::invalid("unknown download"));
        }
        match kind {
            ManualKind::Model => {
                let m = self
                    .catalog()
                    .find(id)
                    .ok_or_else(|| AiCmdError::not_found(format!("'{id}' is not in the model list")))?;
                Ok(ManualDownload {
                    kind,
                    id: m.id.clone(),
                    name: m.name.clone(),
                    // A model that reads images: its projector too (added with "Add projector file…").
                    files: std::iter::once(model_file(m, &self.settings().hf_endpoint, MODEL_REVISION))
                        .chain(projector_file(m, &self.settings().hf_endpoint, MODEL_REVISION))
                        .collect(),
                    folder: self.models_dir().display().to_string(),
                })
            }
            ManualKind::Runtime => {
                let rt = &self.deps.runtimes;
                let build = is_build_id(id)
                    .then(|| rt.catalog.find(&rt.platform, id))
                    .flatten()
                    .ok_or_else(|| AiCmdError::not_found(format!("there is no inference build '{id}' for this computer")))?;
                Ok(ManualDownload {
                    kind,
                    id: build.id.clone(),
                    name: label(&build.id),
                    files: runtime_files(build),
                    folder: rt.store.builds_dir(&rt.catalog.tag).join(&build.id).display().to_string(),
                })
            }
        }
    }

    /// The address of file `index` of a manual download, checked to be an https link to a public host.
    pub fn manual_download_url(&self, kind: ManualKind, id: &str, index: usize) -> AiResult<String> {
        let d = self.manual_download(kind, id)?;
        let f = d
            .files
            .get(index)
            .ok_or_else(|| AiCmdError::invalid("that file is not part of this download"))?;
        check_link(&f.url)?;
        Ok(f.url.clone())
    }
}

/// The files to download by hand for a model or a runtime build, with their sizes and SHA-256.
#[tauri::command]
pub async fn ai_manual_download<R: Runtime>(app: AppHandle<R>, kind: ManualKind, id: String) -> AiResult<ManualDownload> {
    core(&app)?.manual_download(kind, &id)
}

/// Opens file `index` of a manual download in the system browser. The address is computed here from the
/// catalogs; nothing the webview sends is opened.
#[tauri::command]
pub async fn ai_manual_download_open<R: Runtime>(app: AppHandle<R>, kind: ManualKind, id: String, index: usize) -> AiResult<()> {
    let url = core(&app)?.manual_download_url(kind, &id, index)?;
    crate::activity::domain_event("ai.manualDownload", None, &[("Id", id.as_str())]);
    crate::opener::open_url(&url)
}
