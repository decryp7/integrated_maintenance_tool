//! `.etl` files (WPR/xperf, PerfView, `logman`) as profiles (ADR 0022 §9): the process with the most CPU samples
//! is converted through `imt-etw` (with heap events and symbols from the settings' symbol path) and opened with
//! the nettrace importer; the notes name the other processes, which the tool's `etl-convert` opens instead.

use crate::settings::Knobs;
use imt_plugin_api::ProfileImporter;
use imt_profile::{Capture, Limits, ProfileError};
use std::sync::Arc;

/// The importer id.
pub const ID: &str = "etl";

pub struct EtlImporter {
    pub knobs: Arc<Knobs>,
}

impl ProfileImporter for EtlImporter {
    fn id(&self) -> &str {
        ID
    }

    fn heap(&self) -> bool {
        false
    }

    fn sniff(&self, file_name: &str, _head: &[u8]) -> u8 {
        if file_name.to_ascii_lowercase().ends_with(".etl") {
            80
        } else {
            0
        }
    }

    fn import_encoded(&self, file_name: &str, bytes: &[u8], limits: &serde_json::Value) -> Vec<u8> {
        let result = serde_json::from_value::<Limits>(limits.clone())
            .map_err(|e| ProfileError::invalid(format!("invalid limits: {e}")))
            .and_then(|limits| self.import(file_name, bytes, &limits));
        imt_profile::wire::encode_result(&result)
    }
}

impl EtlImporter {
    #[cfg(windows)]
    fn import(&self, file_name: &str, bytes: &[u8], limits: &Limits) -> imt_profile::Result<Capture> {
        windows_impl::import(&self.knobs, file_name, bytes, limits)
    }

    #[cfg(not(windows))]
    fn import(&self, _: &str, _: &[u8], _: &Limits) -> imt_profile::Result<Capture> {
        Err(ProfileError::invalid("ETL import needs Windows"))
    }
}

/// The process a converted `.etl` opens: the one with the most CPU samples (ties: the lower pid).
pub fn default_process(procs: &[imt_etw::scan::EtlProcess]) -> Option<&imt_etw::scan::EtlProcess> {
    procs.iter().max_by(|a, b| a.samples.cmp(&b.samples).then(b.pid.cmp(&a.pid)))
}

#[cfg(windows)]
mod windows_impl {
    use super::default_process;
    use crate::request::SymbolOptions;
    use crate::settings::Knobs;
    use crate::{symbolize, Limits as CaptureLimits};
    use imt_etw::convert::Options;
    use imt_etw::etl;
    use imt_etw::scan::Order;
    use imt_plugin_profiling::nettrace::ext::{ImageLoad, NativeSymbol};
    use imt_plugin_profiling::nettrace::NettraceImporter;
    use imt_profile::{Capture, CaptureImporter, Limits, ProfileError};
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

    /// Temporary files of one import, deleted on drop.
    struct Temp(Vec<std::path::PathBuf>);

    impl Drop for Temp {
        fn drop(&mut self) {
            for p in &self.0 {
                let _ = std::fs::remove_file(p);
            }
        }
    }

    pub(super) fn import(knobs: &Knobs, file_name: &str, bytes: &[u8], limits: &Limits) -> imt_profile::Result<Capture> {
        static N: AtomicU64 = AtomicU64::new(1);
        let io = |e: std::io::Error| ProfileError::invalid(format!("the .etl file could not be read: {e}"));
        let n = N.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir();
        let etl_path = dir.join(format!("imt-etl-{}-{n}.etl", std::process::id()));
        let out = dir.join(format!("imt-etl-{}-{n}.nettrace", std::process::id()));
        let _temp = Temp(vec![etl_path.clone(), out.clone()]);
        std::fs::write(&etl_path, bytes).map_err(io)?;
        let cancel = AtomicBool::new(false);
        let procs = etl::scan(&etl_path, Order::Samples, &cancel).map_err(|e| ProfileError::invalid(e.to_string()))?;
        let p = default_process(&procs)
            .ok_or_else(|| ProfileError::invalid("the trace contains no processes"))?
            .clone();
        let symbols = SymbolOptions::parse(&knobs.symbol_path(), knobs.symbol_server())
            .unwrap_or_default()
            .config(None);
        let max_addresses = CaptureLimits::default().max_addresses;
        let mut sym_notes = Vec::new();
        let mut name = |images: &[ImageLoad], addresses: &[u64]| -> Vec<NativeSymbol> {
            let (s, n) = symbolize::symbolize(&symbols, images, addresses, max_addresses);
            sym_notes = n;
            s
        };
        let options = Options { heap: true, max_addresses };
        let c = etl::convert(&etl_path, &p, &out, limits.max_input_bytes, &cancel, options, &mut name)
            .map_err(|e| ProfileError::invalid(e.to_string()))?;
        let converted = std::fs::read(&out).map_err(io)?;
        let mut capture = NettraceImporter.import(file_name, &converted, limits)?;
        if let Capture::Profile(profile) = &mut capture {
            profile.notes.push(format!("Process {} ({}) of the trace", p.name, p.pid));
            let others: Vec<String> = procs
                .iter()
                .filter(|o| o.pid != p.pid && o.samples > 0)
                .take(5)
                .map(|o| format!("{} ({}, {} samples)", o.name, o.pid, o.samples))
                .collect();
            if !others.is_empty() {
                profile.notes.push(format!(
                    "The trace has other processes: {}; open another one with Tools ▸ Profile native process… ▸ Open .etl",
                    others.join(", ")
                ));
            }
            profile.notes.extend(c.notes);
            profile.notes.extend(sym_notes);
        }
        Ok(capture)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_etw::scan::EtlProcess;

    fn p(pid: u32, samples: u64) -> EtlProcess {
        EtlProcess {
            pid,
            name: format!("p{pid}"),
            command_line: None,
            samples,
            clr_events: 0,
            pointer_size: 8,
        }
    }

    #[test]
    fn sniffs_by_extension_and_picks_the_busiest_process() {
        let i = EtlImporter { knobs: Arc::default() };
        assert_eq!(i.sniff("trace.ETL", b""), 80);
        assert_eq!(i.sniff("trace.nettrace", b""), 0);
        assert!(!i.heap());
        assert_eq!(i.id(), "etl");
        assert_eq!(default_process(&[p(5, 1), p(3, 9), p(2, 9)]).map(|p| p.pid), Some(2));
        assert!(default_process(&[]).is_none());
        let bad = i.import_encoded("x.etl", b"junk", &serde_json::json!("nope"));
        assert!(imt_profile::wire::decode_result(&bad, &Limits::default()).is_err());
        let limits = serde_json::to_value(Limits::default()).unwrap();
        let junk = i.import_encoded("x.etl", &[0x5a; 512], &limits);
        assert!(
            imt_profile::wire::decode_result(&junk, &Limits::default()).is_err(),
            "junk is an error, not a panic"
        );
    }
}
