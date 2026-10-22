//! The plugin adapter: every built-in capture importer as a profile importer of the plugin contract. The
//! profiling plugin's process is the profile "worker" (ADR 0017 §8): captures are parsed there, under its memory
//! cap, and cross to Debrief encoded with `debrief_profile::wire`.

use crate::{builtin_importers, PLUGIN_ID};
use anyhow::Result;
use debrief_plugin_api::{Contributions, Plugin, ProfileImporter};
use debrief_profile::api::CaptureKind;
use debrief_profile::{CaptureImporter, Limits, ProfileError};
use std::sync::Arc;

/// Memory of the profiling process: heap snapshots with millions of objects need room.
const MEMORY_LIMIT: u64 = 3 << 30;

pub struct ProfilingPlugin;

impl Plugin for ProfilingPlugin {
    fn id(&self) -> &str {
        PLUGIN_ID
    }

    fn name(&self) -> &str {
        "Profiling"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        for importer in builtin_importers() {
            c.add_profile_importer(Adapter(importer));
        }
        Ok(())
    }

    fn memory_limit(&self) -> Option<u64> {
        Some(MEMORY_LIMIT)
    }
}

/// A capture importer of `debrief-profile` as a profile importer of the contract.
struct Adapter(Arc<dyn CaptureImporter>);

impl ProfileImporter for Adapter {
    fn id(&self) -> &str {
        self.0.id()
    }

    fn heap(&self) -> bool {
        self.0.produces() == CaptureKind::Heap
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        self.0.sniff(file_name, head)
    }

    fn import_encoded(&self, file_name: &str, bytes: &[u8], limits: &serde_json::Value) -> Vec<u8> {
        let result = serde_json::from_value::<Limits>(limits.clone())
            .map_err(|e| ProfileError::invalid(format!("invalid limits: {e}")))
            .and_then(|limits| self.0.import(file_name, bytes, &limits));
        debrief_profile::wire::encode_result(&result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contributes_every_importer() {
        let mut c = Contributions::default();
        ProfilingPlugin.register(&mut c).unwrap();
        let m = c.manifest(&ProfilingPlugin);
        m.check().unwrap();
        assert_eq!(m.profile_importers.len(), builtin_importers().len());
        assert_eq!(&*m.profile_importers[0].id, builtin_importers()[0].id());
        assert!(m.profile_importers.iter().any(|p| p.heap));
        assert_eq!(m.memory_limit, Some(MEMORY_LIMIT));
    }

    #[test]
    fn captures_and_errors_are_encoded_for_the_host() {
        let a = Adapter(builtin_importers().into_iter().find(|i| i.id() == "folded").unwrap());
        let limits = serde_json::to_value(Limits::default()).unwrap();
        let ok = a.import_encoded("x.folded", b"main;work 3\n", &limits);
        assert!(matches!(
            debrief_profile::wire::decode_result(&ok, &Limits::default()),
            Ok(debrief_profile::Capture::Profile(_))
        ));
        let bad = a.import_encoded("x.folded", b"main;work 3\n", &serde_json::json!("nope"));
        assert!(debrief_profile::wire::decode_result(&bad, &Limits::default()).is_err());
        assert!(!a.heap() && a.sniff("x.folded", b"main;work 3\n") > 0);
    }
}
