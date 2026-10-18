//! IMT plugin for web server logs and HTTP captures.
//!
//! * **W3C extended / IIS logs** (`#Fields:` header): the header drives the
//!   columns and may change mid-file. The time is `date` + `time` (UTC per the
//!   spec); fields keep the W3C names (`cs-uri-stem`, `sc-status` and
//!   `time-taken` as numbers, `-` placeholders omitted).
//! * **Apache / Nginx common and combined logs**, with an optional trailing
//!   request time. Fields: `ClientIp`, `User`, `Method`, `Path`, `Protocol`,
//!   `Status`, `Bytes`, `Referer`, `UserAgent`, `RequestTimeMs`. A trailing
//!   decimal is read as seconds (nginx `$request_time`), an integer as
//!   microseconds (Apache `%D`).
//! * **HAR** (`log.entries`): fields `Method`, `Url`, `Status`, `StatusText`,
//!   `MimeType`, `TimeMs`, `RequestBytes`, `ResponseBytes`, `ServerIp`.
//!
//! The level follows the HTTP status: 5xx Error, 4xx Warning, otherwise
//! Information (a HAR status of 0 is a failed request: Error).
//!
//! ```
//! let mut registry = imt_core::PluginRegistry::new();
//! registry.install(&imt_plugin_web::WebPlugin).unwrap();
//! ```

mod access;
mod har;
mod w3c;

pub use access::AccessLogImporter;
pub use har::HarImporter;
pub use w3c::W3cImporter;

use anyhow::Result;
use imt_core::model::Level;
use imt_core::{Plugin, PluginRegistry};

pub struct WebPlugin;

impl Plugin for WebPlugin {
    fn id(&self) -> &'static str {
        "net.decryptology.web"
    }

    fn name(&self) -> &'static str {
        "Web server logs"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    /// Stand-alone: no dependencies, no services.
    fn manifest(&self) -> imt_core::plugin::Manifest {
        imt_core::plugin::Manifest::new(self.id(), self.version())
    }

    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        registry.add_importer(Box::new(W3cImporter));
        registry.add_importer(Box::new(AccessLogImporter));
        registry.add_importer(Box::new(HarImporter));
        Ok(())
    }
}

/// 5xx Error, 4xx Warning, everything else Information.
pub(crate) fn level_for_status(status: i64) -> Level {
    match status {
        500..=599 => Level::Error,
        400..=499 => Level::Warning,
        _ => Level::Information,
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use imt_core::importers::ImportCtx;
    use imt_core::profile::{CompiledProfile, Profile};
    use std::path::Path;

    pub fn with_ctx<T>(f: impl FnOnce(&ImportCtx) -> T) -> T {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        f(&ImportCtx {
            profile: &p,
            path: Path::new("x"),
            file_date: None,
        })
    }

    /// Deterministic junk built from format-relevant fragments.
    pub fn garbage(pieces: &[&str], rounds: usize) -> Vec<String> {
        let mut x: u32 = 99;
        (0..rounds)
            .map(|_| {
                (0..60)
                    .map(|_| {
                        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                        pieces[(x >> 16) as usize % pieces.len()]
                    })
                    .collect()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::profile::Profile;
    use std::path::Path;

    #[test]
    fn plugin_registers_its_importers() {
        let mut registry = PluginRegistry::new();
        registry.install(&WebPlugin).unwrap();
        let info = &registry.plugins()[0];
        assert_eq!(info.id, "net.decryptology.web");
        assert_eq!(info.contributes.len(), 3);
        assert_eq!(WebPlugin.name(), "Web server logs");
        assert_eq!(WebPlugin.version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(level_for_status(503), Level::Error);
        assert_eq!(level_for_status(404), Level::Warning);
        assert_eq!(level_for_status(200), Level::Information);
        assert_eq!(level_for_status(0), Level::Information);
    }

    #[test]
    fn workspace_opens_the_fixtures_and_samples() {
        let mut registry = PluginRegistry::new();
        registry.install(&WebPlugin).unwrap();
        let mut ws = imt_core::Workspace::with_registry(Profile::default_profile(), registry).unwrap();
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures");
        ws.open(&[dir]).unwrap();
        let s = ws.summary();
        let mut ids: Vec<&str> = s.sources.iter().map(|x| x.importer.as_str()).collect();
        ids.sort();
        assert_eq!(ids, vec!["access-log", "har", "w3c"]);
        assert!(ws.records().iter().any(|r| r.fields.contains_key("cs-uri-stem")));
        assert!(ws.records().iter().any(|r| r.fields.contains_key("ClientIp")));
        assert!(ws.records().iter().any(|r| r.fields.contains_key("Url")));

        // In the repository samples these importers claim the files under samples/web and nothing else.
        let mut registry = PluginRegistry::new();
        registry.install(&WebPlugin).unwrap();
        let mut ws = imt_core::Workspace::with_registry(Profile::default_profile(), registry).unwrap();
        let samples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples");
        let _ = ws.open(&[samples]);
        let summary = ws.summary();
        let mut claimed: Vec<&str> = Vec::new();
        for src in &summary.sources {
            let in_web = src.path.replace('\\', "/").contains("/samples/web/");
            let ours = ["w3c", "access-log", "har"].contains(&src.importer.as_str());
            assert_eq!(ours, in_web, "{} claimed by {}", src.path, src.importer);
            if ours {
                claimed.push(src.importer.as_str());
            }
        }
        claimed.sort();
        claimed.dedup();
        assert_eq!(claimed, vec!["access-log", "har", "w3c"]);
    }
}
