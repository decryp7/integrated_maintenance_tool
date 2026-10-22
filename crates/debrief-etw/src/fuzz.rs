//! Entry points for the fuzz targets and the "never panics" properties: arbitrary bytes become ETW events for the
//! converter and the `.etl` scanner, and the NetTrace they produce goes through the importer. Not API.

use crate::convert::{Converter, Options};
use crate::kernel::{self, guids, opcodes};
use crate::nettrace::{system_time, TraceHeader};
use crate::scan::{Order, Scanner};
use crate::{clr, heap, RawEvent};
use debrief_plugin_profiling::nettrace::ext::NativeSymbol;

/// Providers/opcodes the converter reacts to, picked by the first byte of each event.
const KINDS: [(u128, u8); 13] = [
    (clr::RUNTIME_GUID, 0),
    (clr::RUNDOWN_GUID, 0),
    (guids::PERF_INFO, opcodes::SAMPLED_PROFILE),
    (guids::STACK_WALK, opcodes::STACK_WALK),
    (guids::THREAD, opcodes::CSWITCH),
    (guids::THREAD, opcodes::START),
    (guids::THREAD, opcodes::READY_THREAD),
    (guids::THREAD, opcodes::THREAD_SET_NAME),
    (guids::IMAGE, opcodes::IMAGE_LOAD),
    (guids::PROCESS, opcodes::START),
    (heap::PROVIDER, heap::opcodes::ALLOC),
    (heap::PROVIDER, heap::opcodes::REALLOC),
    (heap::PROVIDER, heap::opcodes::FREE),
];

/// Splits `data` into events: [kind, id, flags, thread/time, len] + payload; feeds the converter (with heap events
/// and address collection) and the scanner, names a few addresses, then imports the NetTrace it wrote.
pub fn etw_events(data: &[u8]) {
    let header = TraceHeader {
        sync_time: system_time(0).unwrap_or_default(),
        sync_ticks: 0,
        ticks_per_second: 10_000_000,
        pointer_size: if data.first().is_some_and(|b| b & 1 == 1) { 4 } else { 8 },
        process_id: 7,
        processors: 1,
        sampling_ns: 1_000_000,
    };
    let options = Options {
        heap: true,
        max_addresses: 64,
    };
    let Ok(mut conv) = Converter::with_options(Vec::new(), header, options) else {
        return;
    };
    conv.seed_threads([1, 2]);
    let mut scanner = Scanner::default();
    let mut rest = data.get(1..).unwrap_or_default();
    let mut ts = 0i64;
    while rest.len() >= 5 {
        let (h, tail) = rest.split_at(5);
        let len = usize::from(h[4]).min(tail.len());
        let (payload, tail) = tail.split_at(len);
        rest = tail;
        let (provider, opcode) = KINDS[usize::from(h[0]) % KINDS.len()];
        ts += i64::from(h[3]);
        let stack: Vec<u64> = payload
            .chunks(8)
            .take(4)
            .map(|c| c.iter().fold(0u64, |a, b| a << 8 | u64::from(*b)))
            .collect();
        let e = RawEvent {
            provider,
            id: u16::from(h[1]),
            version: h[2] & 7,
            opcode,
            process_id: if h[2] & 8 == 0 { 7 } else { 9 },
            thread_id: u32::from(h[3] & 3),
            timestamp: ts,
            pointer_size: if h[2] & 16 == 0 { 8 } else { 4 },
            payload,
            stack: &stack,
        };
        if conv.event(&e).is_err() {
            return;
        }
        scanner.event(&e);
    }
    let _ = scanner.finish_in(Order::Samples);
    let _ = kernel::process(data, 8, 4, 64);
    let symbols: Vec<NativeSymbol> = conv
        .addresses()
        .into_iter()
        .take(8)
        .map(|a| NativeSymbol {
            address: a,
            start: a & !0xf,
            line: (a & 0xff) as u32,
            function: format!("f{a:x}"),
            file: String::new(),
        })
        .collect();
    if conv.symbols(&symbols).is_err() {
        return;
    }
    if let Ok((bytes, _)) = conv.finish() {
        use debrief_profile::CaptureImporter;
        let _ = debrief_plugin_profiling::nettrace::NettraceImporter.import("fuzz.nettrace", &bytes, &debrief_profile::Limits::default());
    }
}
