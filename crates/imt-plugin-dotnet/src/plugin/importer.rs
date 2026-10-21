//! The `.etl` importer: opening or dropping an `.etl` file converts the
//! busiest .NET process in it (else the busiest process) and opens it like a
//! nettrace. The other processes are listed in the notes; the tool's
//! `etl-processes` / `etl-convert` let the user pick another one.

use imt_profile::{Capture, CaptureImporter, Limits, ProfileError, Result};

/// Importer id.
pub const FORMAT: &str = "etl";

/// Windows ETW trace (`.etl`) importer.
pub struct EtlImporter;

/// ETL files start with a WMI buffer header; the extension is what identifies them.
fn is_etl(file_name: &str) -> bool {
    file_name.to_ascii_lowercase().ends_with(".etl")
}

impl CaptureImporter for EtlImporter {
    fn id(&self) -> &str {
        FORMAT
    }

    fn name(&self) -> &str {
        "Windows ETW trace (.etl)"
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        if is_etl(file_name) && head.len() >= 64 {
            90
        } else {
            0
        }
    }

    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture> {
        if bytes.len() as u64 > limits.max_input_bytes {
            return Err(ProfileError::LimitExceeded {
                what: "file size",
                limit: limits.max_input_bytes,
                hint: "record a shorter trace",
            });
        }
        sys::import(file_name, bytes, limits)
    }
}

/// The `.etl` importer as a profile importer of the plugin contract: the capture is converted in the plugin's
/// process and crosses encoded with `imt_profile::wire`.
pub(crate) struct Adapter;

impl imt_plugin_api::ProfileImporter for Adapter {
    fn id(&self) -> &str {
        FORMAT
    }

    fn heap(&self) -> bool {
        false
    }

    fn sniff(&self, file_name: &str, head: &[u8]) -> u8 {
        EtlImporter.sniff(file_name, head)
    }

    fn import_encoded(&self, file_name: &str, bytes: &[u8], limits: &serde_json::Value) -> Vec<u8> {
        let result = serde_json::from_value::<Limits>(limits.clone())
            .map_err(|e| ProfileError::invalid(format!("invalid limits: {e}")))
            .and_then(|limits| EtlImporter.import(file_name, bytes, &limits));
        imt_profile::wire::encode_result(&result)
    }
}

#[cfg(windows)]
mod sys {
    use super::FORMAT;
    use crate::etw::{etl, scan};
    use imt_plugin_profiling::nettrace::NettraceImporter;
    use imt_profile::{Capture, CaptureImporter, Limits, ProfileError, Result};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    /// A temporary file removed on drop.
    struct Temp(std::path::PathBuf);

    impl Drop for Temp {
        fn drop(&mut self) {
            let _ = std::fs::remove_file(&self.0);
        }
    }

    fn temp(ext: &str) -> Temp {
        static N: AtomicU64 = AtomicU64::new(0);
        Temp(std::env::temp_dir().join(format!(
            "imt-etl-{}-{}.{ext}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        )))
    }

    fn err(e: impl std::fmt::Display) -> ProfileError {
        ProfileError::malformed(FORMAT, e.to_string())
    }

    pub(super) fn import(file_name: &str, bytes: &[u8], limits: &Limits) -> Result<Capture> {
        // The ETW consumer API reads files by path.
        let input = temp("etl");
        std::fs::write(&input.0, bytes).map_err(err)?;
        let cancel = AtomicBool::new(false);
        let procs = etl::scan(&input.0, &cancel).map_err(err)?;
        let Some(chosen) = scan::default_process(&procs) else {
            return Err(ProfileError::malformed(
                FORMAT,
                "the trace has no process with CPU samples or .NET events",
            ));
        };
        let out = temp("nettrace");
        let converted = etl::convert(&input.0, chosen, &out.0, limits.max_input_bytes, &cancel).map_err(err)?;
        let nettrace = std::fs::read(&out.0).map_err(err)?;
        let stem = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
        let name = format!("{} {} ({})", stem.trim_end_matches(".etl"), chosen.name, chosen.pid);
        let mut capture = NettraceImporter.import(&name, &nettrace, limits)?;
        let mut notes = converted.notes;
        let others: Vec<String> = procs
            .iter()
            .filter(|p| p.pid != chosen.pid && (p.samples > 0 || p.is_dotnet()))
            .take(10)
            .map(|p| format!("{} ({}, {} samples)", p.name, p.pid, p.samples))
            .collect();
        if !others.is_empty() {
            notes.push(format!(
                "Showing {} ({}); the trace also has {}. Use \"Open process from .etl…\" to pick another",
                chosen.name,
                chosen.pid,
                others.join(", ")
            ));
        }
        match &mut capture {
            Capture::Profile(p) => {
                p.name = name;
                p.notes.extend(notes);
            }
            Capture::Heap(h) => h.notes.extend(notes),
        }
        Ok(capture)
    }
}

#[cfg(not(windows))]
mod sys {
    use super::FORMAT;
    use imt_profile::{Capture, Limits, ProfileError, Result};

    pub(super) fn import(_: &str, _: &[u8], _: &Limits) -> Result<Capture> {
        Err(ProfileError::unsupported(FORMAT, "ETL import needs Windows"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sniffs_by_extension_and_refuses_damaged_files() {
        assert_eq!(EtlImporter.sniff("trace.ETL", &[0; 64]), 90);
        assert_eq!(EtlImporter.sniff("trace.etl", &[0; 10]), 0);
        assert_eq!(EtlImporter.sniff("trace.nettrace", &[0; 64]), 0);
        assert_eq!(EtlImporter.id(), "etl");
        assert!(!EtlImporter.name().is_empty());
        let e = EtlImporter.import("x.etl", &[0x11; 4096], &Limits::default()).unwrap_err();
        assert!(!e.to_string().is_empty());
        if !cfg!(windows) {
            assert!(e.to_string().contains("ETL import needs Windows"));
        }
        let small = Limits {
            max_input_bytes: 10,
            ..Limits::default()
        };
        assert!(matches!(
            EtlImporter.import("x.etl", &[0; 64], &small),
            Err(ProfileError::LimitExceeded { .. })
        ));
    }

    #[test]
    fn the_adapter_encodes_results_and_typed_errors() {
        use imt_plugin_api::ProfileImporter;
        assert_eq!(Adapter.sniff("a.etl", &[0; 64]), 90);
        assert_eq!(Adapter.id(), "etl");
        let limits = serde_json::to_value(Limits {
            max_input_bytes: 10,
            ..Limits::default()
        })
        .unwrap();
        let bytes = Adapter.import_encoded("x.etl", &[0; 64], &limits);
        let e = imt_profile::wire::decode_result(&bytes, &Limits::default()).unwrap_err();
        assert!(e.to_string().contains("file size"), "{e}");
        let bad = Adapter.import_encoded("x.etl", &[0; 64], &serde_json::json!("nope"));
        assert!(imt_profile::wire::decode_result(&bad, &Limits::default()).is_err());
    }
}
