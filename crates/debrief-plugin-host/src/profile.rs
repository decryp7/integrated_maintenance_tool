//! Profile importers of a plugin process (the profiling plugin, ADR 0017 §8: the profile "worker"): the capture is
//! parsed in the plugin's process, under its memory cap, and crosses as bytes that `debrief_profile::wire` decodes and
//! checks against the limits. Registered on `debrief_profile::IMPORTERS_POINT` like in-process importers.

use crate::process::{Lent, PluginProcess};
use debrief_core::guard::{CallKind, Health};
use debrief_core::PluginRegistry;
use debrief_ipc::{IpcError, Json};
use debrief_plugin_api::manifest::ProfileImporterCap;
use debrief_plugin_api::protocol::{PluginRequest, ProfileDone};
use debrief_profile::api::CaptureKind;
use debrief_profile::{Capture, CaptureImporter, Limits, ProfileError};
use std::sync::Arc;
use std::time::Duration;

/// How long sniffing a capture may take.
const SNIFF_DEADLINE: Duration = Duration::from_secs(10);
/// The encoded capture may be this many times the input (frames, stacks and samples are interned).
const ENCODED_FACTOR: u64 = 8;

struct RemoteCaptureImporter {
    process: Arc<PluginProcess>,
    health: Arc<Health>,
    cap: ProfileImporterCap,
}

impl RemoteCaptureImporter {
    fn failed(&self, e: IpcError) -> ProfileError {
        let id = self.process.id();
        if e.is_process_failure() || matches!(&e, IpcError::Remote(r) if r.code == debrief_ipc::peer::code::PANIC) {
            self.health.fail(id, &format!("profile importer \"{}\" failed: {e}", self.cap.id));
        }
        ProfileError::Importer {
            importer: self.cap.id.to_string(),
            message: e.to_string(),
        }
    }
}

impl CaptureImporter for RemoteCaptureImporter {
    fn id(&self) -> &str {
        &self.cap.id
    }

    fn name(&self) -> &str {
        &self.cap.id
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        let req = PluginRequest::SniffProfile {
            importer: self.cap.id.clone(),
            file_name: file_name.into(),
            head: head.to_vec(),
        };
        self.process.call(&req, Some(SNIFF_DEADLINE), false).unwrap_or(0)
    }

    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> debrief_profile::Result<Capture> {
        let cap = limits.max_input_bytes.saturating_mul(ENCODED_FACTOR).max(1 << 20);
        let limits_json = serde_json::to_value(limits).map_err(|e| ProfileError::invalid(e.to_string()))?;
        let mut encoded: Vec<u8> = Vec::new();
        let lent = Lent {
            files: vec![bytes],
            ..Lent::default()
        };
        let budget = self.health.budgets().get(CallKind::Import).map(crate::remote::deadline_for);
        let (importer, name) = (self.cap.id.clone(), file_name.to_string());
        let done: ProfileDone = self
            .process
            .call_lending(
                |refs, _| PluginRequest::ImportProfile {
                    importer,
                    file_name: name,
                    content: refs[0],
                    limits: Json(limits_json),
                },
                &lent,
                budget,
                |piece| {
                    if encoded.len() as u64 + piece.len() as u64 > cap {
                        return Err(IpcError::Protocol(format!("the capture is larger than {cap} bytes")));
                    }
                    encoded.extend_from_slice(&piece);
                    Ok(())
                },
            )
            .map_err(|e| self.failed(e))?;
        if done.bytes != encoded.len() as u64 {
            return Err(ProfileError::malformed("plugin capture", "the capture arrived incomplete"));
        }
        debrief_profile::wire::decode_result(&encoded, limits)
    }

    fn produces(&self) -> CaptureKind {
        if self.cap.heap {
            CaptureKind::Heap
        } else {
            CaptureKind::Profile
        }
    }
}

/// Contributes the process' profile importers to `registry` (declaring the extension point when no one did).
pub(crate) fn register(registry: &mut PluginRegistry, process: &Arc<PluginProcess>, health: &Arc<Health>) -> anyhow::Result<()> {
    let _ = registry.declare_extension_point::<dyn CaptureImporter>(debrief_profile::IMPORTERS_POINT);
    for cap in &process.manifest().profile_importers {
        registry.contribute::<dyn CaptureImporter>(
            debrief_profile::IMPORTERS_POINT,
            Arc::new(RemoteCaptureImporter {
                process: process.clone(),
                health: health.clone(),
                cap: cap.clone(),
            }),
        );
    }
    Ok(())
}
