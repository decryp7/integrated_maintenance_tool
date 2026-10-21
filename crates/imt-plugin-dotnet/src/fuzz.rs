//! Entry points for the fuzz targets and the "never panics" properties: each
//! turns arbitrary bytes into input for one decoder and runs it. Not API.
//!
//! ETW events go through `imt-etw`'s converter (`imt_etw::fuzz::etw_events`, the `fuzz_native_etw` target).

use crate::eventpipe::protocol;
use crate::nettrace::{self, TraceHeader};

/// ETW events through the shared converter and scanner (kept for the properties of this crate).
pub fn etw_events(data: &[u8]) {
    imt_etw::fuzz::etw_events(data);
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
