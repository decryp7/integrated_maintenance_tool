//! Entry points for the fuzz targets and the "never panics" properties: each
//! turns arbitrary bytes into input for one decoder and runs it. Not API.

use crate::etw::convert::Converter;
use crate::etw::kernel::{self, guids, opcodes};
use crate::etw::scan::Scanner;
use crate::etw::RawEvent;
use crate::eventpipe::protocol;
use crate::nettrace::{system_time, TraceHeader};
use crate::{clr, nettrace};

/// Providers/opcodes the converter reacts to, picked by the first byte of each event.
const KINDS: [(u128, u8); 10] = [
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
];

/// Splits `data` into events: [kind, id, ptr flag, thread, len] + payload; feeds the ETW
/// converter and the `.etl` scanner, then imports the NetTrace it wrote.
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
    let Ok(mut conv) = Converter::new(Vec::new(), header) else { return };
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
    let _ = scanner.finish();
    let _ = kernel::process(data, 8, 4, 64);
    if let Ok((bytes, _)) = conv.finish() {
        use imt_profile::CaptureImporter;
        let _ = imt_plugin_profiling::nettrace::NettraceImporter.import("fuzz.nettrace", &bytes, &imt_profile::Limits::default());
    }
}

/// Diagnostics IPC replies.
pub fn ipc_reply(data: &[u8]) {
    if let Ok((size, set, id)) = protocol::header(data) {
        let payload = data.get(protocol::HEADER_LEN..size.min(data.len())).unwrap_or_default();
        if let Ok(p) = protocol::ok_payload(set, id, payload) {
            let _ = protocol::session_id(p);
            let _ = protocol::parse_process_info(p, true);
            let _ = protocol::parse_process_info(p, false);
        }
    }
    let _ = protocol::parse_process_info(data, true);
    let _ = nettrace::NetTraceWriter::new(
        Vec::new(),
        &TraceHeader {
            sync_time: [0; 8],
            sync_ticks: 0,
            ticks_per_second: i64::from(data.first().copied().unwrap_or(1)),
            pointer_size: u32::from(data.get(1).copied().unwrap_or(8)),
            process_id: 0,
            processors: 0,
            sampling_ns: 0,
        },
    );
}
