//! "Set up AI assistant": probe the hardware, recommend a model, download and verify it, validate the
//! file, load it (GPU first, CPU fallback), warm it up and remember the choice.

use crate::catalog::{Catalog, CatalogModel};
use crate::error::{AiError, Result};
use crate::gguf;
use crate::hardware::{Hardware, HardwareProbe};
use crate::llm::{ChatMessage, ChatRequest, LlmProvider};
use crate::models::source::{ModelSource, Phase, Progress};
use crate::models::{DiskSpace, DownloadOptions, Downloader, ModelStore};
use crate::recommend::{recommend, Recommendation, Recommendations};
use crate::runtime::{check_memory, effective_context, load_with_fallback, LoadRequest, ModelRuntime};
use crate::settings::AiSettings;
use serde::Serialize;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "stage", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum SetupStage {
    Probing,
    Recommending,
    Downloading {
        bytes: u64,
        total: u64,
        bytes_per_sec: f64,
        eta_secs: Option<u64>,
    },
    Verifying {
        bytes: u64,
        total: u64,
    },
    Validating,
    Loading {
        gpu: bool,
    },
    FellBackToCpu {
        reason: String,
    },
    WarmingUp,
    Ready {
        model: String,
        backend: String,
    },
}

/// What the user is asked to confirm before anything is downloaded.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupPlan {
    pub hardware: Hardware,
    pub recommendations: Recommendations,
    /// The model that will be set up unless the user picks another.
    pub chosen: Option<Recommendation>,
    pub license: Option<String>,
    pub gated: bool,
    /// Bytes still to download (0 when the file is already installed).
    pub download_bytes: u64,
    pub already_installed: bool,
    /// Why the licence could not be shown (offline, host down), if so.
    pub note: Option<String>,
}

pub struct SetupOutcome {
    pub provider: Arc<dyn LlmProvider>,
    pub settings: AiSettings,
    pub backend: String,
    pub notes: Vec<String>,
}

pub struct Setup<'a> {
    pub probe: &'a dyn HardwareProbe,
    pub source: &'a dyn ModelSource,
    pub disk: &'a dyn DiskSpace,
    pub runtime: &'a dyn ModelRuntime,
    pub store: &'a ModelStore,
    pub catalog: &'a Catalog,
    pub settings_path: &'a Path,
    pub settings: AiSettings,
    pub backoff: Duration,
}

impl Setup<'_> {
    /// Step 1: what would be set up, and what does it cost.
    pub fn plan(&self) -> Result<SetupPlan> {
        let hardware = self.probe.probe();
        let installed: Vec<String> = self.store.list().into_iter().map(|m| m.file_name).collect();
        let recommendations = recommend(&hardware, self.catalog, &installed);
        let chosen = recommendations.best.clone();
        let (mut license, mut gated, mut note) = (None, false, None);
        let mut download_bytes = 0;
        let already_installed = chosen.as_ref().is_some_and(|c| c.installed);
        if let Some(c) = &chosen {
            if !already_installed {
                download_bytes = c.model.size_bytes;
                match self.source.describe(&c.model.hf_repo, &c.model.file) {
                    Ok(r) => {
                        license = r.license;
                        gated = r.gated;
                    }
                    Err(e) => {
                        license = Some(c.model.license.clone());
                        note = Some(format!("Could not reach the model host to confirm the licence: {e}"));
                    }
                }
            } else {
                license = Some(c.model.license.clone());
            }
        }
        Ok(SetupPlan {
            hardware,
            recommendations,
            chosen,
            license,
            gated,
            download_bytes,
            already_installed,
            note,
        })
    }

    /// Step 2 (after the user confirms): download if needed, then activate.
    pub fn run(&self, choice: &CatalogModel, progress: &Progress, on_stage: &mut dyn FnMut(SetupStage)) -> Result<SetupOutcome> {
        on_stage(SetupStage::Probing);
        let hardware = self.probe.probe();
        on_stage(SetupStage::Recommending);
        let path = self.store.path_of(&choice.file)?;
        let installed_ok = path.is_file() && std::fs::metadata(&path).map(|m| m.len() == choice.size_bytes).unwrap_or(false);
        if !installed_ok {
            self.download(choice, progress, on_stage)?;
        }
        self.activate_inner(&choice.file, &hardware, progress, on_stage)
    }

    /// Activates a model that is already in the models folder (also used for models the user added).
    pub fn activate(&self, file_name: &str, progress: &Progress, on_stage: &mut dyn FnMut(SetupStage)) -> Result<SetupOutcome> {
        on_stage(SetupStage::Probing);
        let hardware = self.probe.probe();
        self.activate_inner(file_name, &hardware, progress, on_stage)
    }

    /// Downloads `choice` into the models folder and verifies it against the catalog, without loading it (also
    /// how a model update fetches its new file: the installed one is untouched until this succeeds). The file
    /// appears under its real name only once its SHA-256 matches.
    pub fn download(&self, choice: &CatalogModel, progress: &Progress, on_stage: &mut dyn FnMut(SetupStage)) -> Result<()> {
        let remote = self.source.describe(&choice.hf_repo, &choice.file)?;
        if remote.sha256 != choice.sha256.to_ascii_lowercase() || remote.size != choice.size_bytes {
            return Err(AiError::Integrity(
                "the file on the model host differs from the catalog (it was probably updated); update the model list and try again".into(),
            ));
        }
        let cap = (self.settings.max_download_gb as u64) * 1024 * 1024 * 1024;
        let dl = Downloader {
            source: self.source,
            disk: self.disk,
            opts: DownloadOptions {
                max_bytes: cap.max(choice.size_bytes),
                backoff: self.backoff,
                ..DownloadOptions::default()
            },
        };
        let dir = self.store.dir().to_path_buf();
        on_stage(SetupStage::Downloading {
            bytes: 0,
            total: remote.size,
            bytes_per_sec: 0.0,
            eta_secs: None,
        });
        std::thread::scope(|scope| {
            let worker = scope.spawn(|| dl.download(&remote, &dir, progress));
            while !worker.is_finished() {
                std::thread::sleep(Duration::from_millis(100));
                emit_progress(progress, on_stage);
            }
            emit_progress(progress, on_stage);
            worker
                .join()
                .unwrap_or_else(|_| Err(AiError::Io("the download stopped unexpectedly".into())))
        })?;
        Ok(())
    }

    fn activate_inner(
        &self,
        file_name: &str,
        hardware: &Hardware,
        progress: &Progress,
        on_stage: &mut dyn FnMut(SetupStage),
    ) -> Result<SetupOutcome> {
        on_stage(SetupStage::Validating);
        let path = self.store.path_of(file_name)?;
        let info = gguf::inspect(&path)?;
        check_memory(hardware, &info)?;
        let gpu = self.settings.gpu_offload.into();
        on_stage(SetupStage::Loading {
            gpu: gpu != crate::runtime::GpuMode::Cpu,
        });
        let req = LoadRequest {
            context: effective_context(&self.settings, &info),
            path,
            info,
            gpu,
            threads: self.settings.threads,
        };
        let (loaded, fallback) = load_with_fallback(self.runtime, req)?;
        let mut notes = Vec::new();
        if let Some(n) = fallback {
            on_stage(SetupStage::FellBackToCpu { reason: n.clone() });
            notes.push(n);
        }
        on_stage(SetupStage::WarmingUp);
        let warm = ChatRequest {
            system: "Reply with the single word OK.".into(),
            messages: vec![ChatMessage::user("ping")],
            max_tokens: 8,
            cancel: Some(progress.cancel_flag()),
            ..ChatRequest::default()
        };
        loaded.provider.chat(&warm, &mut |_| {})?;
        let mut settings = self.settings.clone();
        settings.active_model = Some(file_name.to_string());
        let settings = settings.clamp();
        settings.save(self.settings_path)?;
        on_stage(SetupStage::Ready {
            model: file_name.to_string(),
            backend: loaded.backend.clone(),
        });
        Ok(SetupOutcome {
            provider: loaded.provider,
            settings,
            backend: loaded.backend,
            notes,
        })
    }
}

fn emit_progress(p: &Progress, on_stage: &mut dyn FnMut(SetupStage)) {
    let s = p.snapshot();
    on_stage(match s.phase {
        Phase::Downloading => SetupStage::Downloading {
            bytes: s.bytes,
            total: s.total,
            bytes_per_sec: s.bytes_per_sec,
            eta_secs: s.eta_secs,
        },
        Phase::Verifying => SetupStage::Verifying {
            bytes: s.bytes,
            total: s.total,
        },
    });
}
