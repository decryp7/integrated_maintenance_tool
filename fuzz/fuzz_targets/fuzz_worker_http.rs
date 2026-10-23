#![no_main]
//! The AI worker's HTTP/1.1 response parser (what llama-server sends): status line, headers, chunked and
//! length framing, line reads. Never panics and never reads past its limits.
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let Some((&mode, rest)) = data.split_first() else { return };
    let Ok((_, mut body)) = debrief_ai_worker::http::read_head(rest) else {
        return;
    };
    if mode % 2 == 0 {
        if let Ok(all) = body.read_all(4096) {
            assert!(all.len() <= 4096);
        }
    } else {
        while let Ok(Some(line)) = body.next_line(256) {
            assert!(line.len() <= 256 * 3);
        }
    }
});
