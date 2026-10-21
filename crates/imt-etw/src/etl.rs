//! Reading `.etl` files (PerfView, WPR/xperf, `logman`, any ETW logger) with
//! the ETW consumer API: [`scan`] lists the processes a trace contains, and
//! [`convert`] writes one of them as NetTrace through the same converter as
//! live sessions (with a symbolisation step before the stream ends).

use crate::convert::{Converter, Options, Stats};
use crate::nettrace::{system_time, TraceHeader};
use crate::scan::{EtlProcess, Order, Scanner};
use crate::sys::{self, LogHeader, Source};
use crate::{EtwError, RawEvent};
use imt_plugin_profiling::nettrace::ext::{ImageLoad, NativeSymbol};
use std::io::BufWriter;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

/// FILETIME of 1970-01-01.
const UNIX_EPOCH_FILETIME: i64 = 116_444_736_000_000_000;

/// A clearer message for the errors a damaged or foreign file causes.
fn file_error(e: EtwError) -> EtwError {
    match e {
        EtwError::Win32 { call, code, .. } if matches!(code, 2 | 3) => EtwError::Win32 {
            call,
            code,
            message: "the file was not found".into(),
        },
        EtwError::Win32 { call, code, .. } if matches!(code, 11 | 13 | 87 | 1392 | 0x1069 | 161) => EtwError::Win32 {
            call,
            code,
            message: "the file is not a valid .etl trace, or it is damaged or truncated".into(),
        },
        other => other,
    }
}

/// The processes in `path`, in `order`.
pub fn scan(path: &Path, order: Order, cancel: &AtomicBool) -> Result<Vec<EtlProcess>, EtwError> {
    let mut scanner = Scanner::default();
    sys::consume(Source::File(path), |_| {}, &mut |e: &RawEvent<'_>| {
        scanner.event(e);
        !cancel.load(Ordering::Relaxed)
    })
    .map_err(file_error)?;
    if cancel.load(Ordering::Relaxed) {
        return Err(EtwError::Unsupported("cancelled".into()));
    }
    if scanner.events == 0 {
        return Err(EtwError::Unsupported(
            "the trace contains no events (is the file empty or truncated?)".into(),
        ));
    }
    Ok(scanner.finish_in(order))
}

/// Names the code addresses of a capture: given its images and distinct addresses, the symbols it found.
pub type Symbolize<'a> = &'a mut dyn FnMut(&[ImageLoad], &[u64]) -> Vec<NativeSymbol>;

/// What a conversion produced.
#[derive(Debug, Clone, Default)]
pub struct Converted {
    pub bytes: u64,
    pub stats: Stats,
    pub truncated: bool,
    pub notes: Vec<String>,
    /// Addresses named by the symbolisation step.
    pub symbols: usize,
}

/// Unix milliseconds of a FILETIME.
pub fn filetime_ms(ft: i64) -> i64 {
    (ft - UNIX_EPOCH_FILETIME) / 10_000
}

/// Converts the events of `process` in `path` into a NetTrace file at `out`, writing what `options` asks for and
/// the symbols `symbolize` finds for the collected addresses.
pub fn convert(
    path: &Path,
    process: &EtlProcess,
    out: &Path,
    max_bytes: u64,
    cancel: &AtomicBool,
    options: Options,
    symbolize: Symbolize<'_>,
) -> Result<Converted, EtwError> {
    let file = std::fs::File::create(out)?;
    let mut writer = Some(BufWriter::with_capacity(1 << 20, file));
    let header = std::cell::Cell::new(LogHeader::default());
    let mut conv: Option<Converter<BufWriter<std::fs::File>>> = None;
    let mut failed: Option<std::io::Error> = None;
    let mut truncated = false;
    let consumed = sys::consume(Source::File(path), |h| header.set(h), &mut |e: &RawEvent<'_>| {
        if cancel.load(Ordering::Relaxed) {
            return false;
        }
        if conv.is_none() {
            // The first event fixes the clock origin; the log header gives the wall-clock start.
            let Some(w) = writer.take() else { return false };
            let header = header.get();
            let h = TraceHeader {
                sync_time: system_time(filetime_ms(header.start_time)).unwrap_or([1970, 1, 4, 1, 0, 0, 0, 0]),
                sync_ticks: e.timestamp,
                ticks_per_second: header.ticks_per_second(),
                pointer_size: process.pointer_size,
                process_id: process.pid,
                processors: header.processors.max(1),
                sampling_ns: 1_000_000,
            };
            match Converter::with_options(w, h, options) {
                Ok(c) => conv = Some(c),
                Err(err) => {
                    failed = Some(err);
                    return false;
                }
            }
        }
        let Some(c) = conv.as_mut() else { return false };
        if let Err(err) = c.event(e) {
            failed = Some(err);
            return false;
        }
        if c.bytes() >= max_bytes {
            truncated = true;
            return false;
        }
        true
    });
    let panicked = consumed.map_err(file_error)?;
    if let Some(e) = failed {
        return Err(EtwError::Io(e));
    }
    if cancel.load(Ordering::Relaxed) {
        return Err(EtwError::Unsupported("cancelled".into()));
    }
    let mut conv = conv.ok_or_else(|| EtwError::Unsupported("the trace contains no events (is the file empty or truncated?)".into()))?;
    let symbols = symbolize(conv.images(), &conv.addresses());
    conv.symbols(&symbols)?;
    let (w, stats) = conv.finish()?;
    let file = w.into_inner().map_err(|e| EtwError::Io(e.into_error()))?;
    let bytes = file.metadata()?.len();
    let mut notes = Vec::new();
    if truncated {
        notes.push(format!(
            "The conversion reached the size limit of {} MB and was cut",
            max_bytes >> 20
        ));
    }
    if panicked {
        notes.push("Some events could not be converted (a bug in IMT); the capture may be incomplete".into());
    }
    let lost = header.get().events_lost;
    if lost > 0 {
        notes.push(format!("The trace lost {lost} events while it was recorded"));
    }
    if stats.samples == 0 && stats.clr_events == 0 && stats.heap_ops == 0 {
        notes.push(format!(
            "The trace has no CPU samples, heap or .NET events for process {}",
            process.pid
        ));
    }
    Ok(Converted {
        bytes,
        stats,
        truncated,
        notes,
        symbols: symbols.len(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_and_damaged_files_are_reported_not_panics() {
        let dir = tempfile::tempdir().unwrap();
        let cancel = AtomicBool::new(false);
        let missing = dir.path().join("nope.etl");
        let e = scan(&missing, Order::Samples, &cancel).unwrap_err();
        assert!(e.to_string().contains("not found") || e.to_string().contains("not a valid"), "{e}");
        let junk = dir.path().join("junk.etl");
        std::fs::write(&junk, vec![0x5a; 4096]).unwrap();
        let e = scan(&junk, Order::DotnetFirst, &cancel).unwrap_err();
        assert!(!e.to_string().is_empty());
        let p = EtlProcess {
            pid: 1,
            name: "x".into(),
            command_line: None,
            samples: 0,
            clr_events: 0,
            pointer_size: 8,
        };
        let mut none = |_: &[ImageLoad], _: &[u64]| Vec::new();
        assert!(convert(
            &junk,
            &p,
            &dir.path().join("o.nettrace"),
            1 << 20,
            &cancel,
            Options::default(),
            &mut none
        )
        .is_err());
        assert_eq!(filetime_ms(UNIX_EPOCH_FILETIME + 10_000), 1);
    }
}
