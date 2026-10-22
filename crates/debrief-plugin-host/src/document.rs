//! Document importers of a plugin process (the dumps plugin, ADR 0023): the file is lent by ranges (a full dump
//! may be gigabytes; the plugin reads the parts it needs), parsed in the plugin's sandboxed process under its
//! memory cap, and the document crosses as bytes that `debrief_dump::wire` decodes and checks against the limits.
//! Registered on `debrief_dump::IMPORTERS_POINT`.

use crate::process::{Lent, PluginProcess, RangeSource};
use debrief_core::guard::{CallKind, Health};
use debrief_core::PluginRegistry;
use debrief_dump::{Document, DumpError, DumpImporter, ImportOptions, Produces, ReadAt};
use debrief_ipc::{IpcError, Json};
use debrief_plugin_api::manifest::DocumentImporterCap;
use debrief_plugin_api::protocol::{DocumentDone, PluginRequest};
use std::sync::Arc;
use std::time::Duration;

/// How long sniffing a file may take.
const SNIFF_DEADLINE: Duration = Duration::from_secs(10);

/// The engine's file, readable by the host's pump.
struct Ranges<'a>(&'a dyn ReadAt);

impl RangeSource for Ranges<'_> {
    fn len(&self) -> u64 {
        self.0.len()
    }

    fn read_at(&self, offset: u64, len: usize) -> std::io::Result<Vec<u8>> {
        self.0.read_at(offset, len)
    }
}

struct RemoteDocumentImporter {
    process: Arc<PluginProcess>,
    health: Arc<Health>,
    cap: DocumentImporterCap,
    produces: Produces,
}

impl RemoteDocumentImporter {
    fn failed(&self, e: IpcError) -> DumpError {
        let id = self.process.id();
        if e.is_process_failure() || matches!(&e, IpcError::Remote(r) if r.code == debrief_ipc::peer::code::PANIC) {
            self.health.fail(id, &format!("document importer \"{}\" failed: {e}", self.cap.id));
        }
        DumpError::invalid(format!("the dump reader failed: {e}"))
    }
}

impl DumpImporter for RemoteDocumentImporter {
    fn id(&self) -> &str {
        &self.cap.id
    }

    fn produces(&self) -> Produces {
        self.produces
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        let req = PluginRequest::SniffDocument {
            importer: self.cap.id.clone(),
            file_name: file_name.into(),
            head: head.to_vec(),
        };
        self.process.call(&req, Some(SNIFF_DEADLINE), false).unwrap_or(0)
    }

    fn import(&self, file_name: &str, file: Arc<dyn ReadAt>, options: &ImportOptions) -> debrief_dump::Result<Document> {
        let cap = options.limits.max_encoded_bytes;
        let options_json = serde_json::to_value(options).map_err(|e| DumpError::invalid(e.to_string()))?;
        let source = Ranges(file.as_ref());
        let lent = Lent {
            ranges: vec![&source as &dyn RangeSource],
            ..Lent::default()
        };
        let budget = self.health.budgets().get(CallKind::Import).map(crate::remote::deadline_for);
        let (importer, name) = (self.cap.id.clone(), file_name.to_string());
        let mut encoded: Vec<u8> = Vec::new();
        let done: DocumentDone = self
            .process
            .call_lending(
                |refs, _| PluginRequest::ImportDocument {
                    importer,
                    file_name: name,
                    content: refs[0],
                    options: Json(options_json),
                },
                &lent,
                budget,
                |piece| {
                    if encoded.len() as u64 + piece.len() as u64 > cap {
                        return Err(IpcError::Protocol(format!("the document is larger than {cap} bytes")));
                    }
                    encoded.extend_from_slice(&piece);
                    Ok(())
                },
            )
            .map_err(|e| self.failed(e))?;
        if done.bytes != encoded.len() as u64 {
            return Err(DumpError::invalid("the document arrived incomplete"));
        }
        debrief_dump::wire::decode_result(&encoded, &options.limits)
    }
}

/// What a manifest's `produces` names, if the engine knows it.
fn produces(name: &str) -> Option<Produces> {
    match name {
        "dump" => Some(Produces::Dump),
        "umdh" => Some(Produces::Umdh),
        _ => None,
    }
}

/// Contributes the process' document importers to `registry` (declaring the extension point when no one did).
/// Importers of a kind this engine does not know are skipped with a log line.
pub(crate) fn register(registry: &mut PluginRegistry, process: &Arc<PluginProcess>, health: &Arc<Health>) -> anyhow::Result<()> {
    let _ = registry.declare_extension_point::<dyn DumpImporter>(debrief_dump::IMPORTERS_POINT);
    for cap in &process.manifest().document_importers {
        let Some(kind) = produces(&cap.produces) else {
            log::warn!(
                "plugin \"{}\": document importer \"{}\" produces \"{}\", which Debrief does not know",
                process.id(),
                cap.id,
                cap.produces
            );
            continue;
        };
        registry.contribute::<dyn DumpImporter>(
            debrief_dump::IMPORTERS_POINT,
            Arc::new(RemoteDocumentImporter {
                process: process.clone(),
                health: health.clone(),
                cap: cap.clone(),
                produces: kind,
            }),
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn known_kinds_only() {
        assert_eq!(produces("dump"), Some(Produces::Dump));
        assert_eq!(produces("umdh"), Some(Produces::Umdh));
        assert_eq!(produces("pcap"), None);
        let v = vec![1u8, 2, 3];
        let r = Ranges(&v);
        assert_eq!((r.len(), r.read_at(1, 9).unwrap()), (3, vec![2, 3]));
    }
}
