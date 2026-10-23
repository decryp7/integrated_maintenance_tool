//! Reads a model's GGUF header from the model host without downloading the model: only the first few MiB are
//! requested (the key-value section sits at the start of the file, ahead of the tensors), which is enough to know
//! the architecture and so how much memory the context will need (`crate::fit`).

use super::source::{ModelSource, RemoteFile};
use crate::error::{AiError, Result};
use crate::gguf::{self, GgufInfo};
use std::io::Cursor;
use std::sync::atomic::AtomicBool;

/// Prefix sizes tried in turn: most headers fit the first; models with a large vocabulary need more.
const STEPS: [u64; 3] = [4 << 20, 16 << 20, 48 << 20];

/// The header of `remote`, read from the first bytes only. Errors are those of the host (offline, gated, ...) or
/// of the header (not a GGUF, truncated within 48 MiB).
pub fn read_remote_header(source: &dyn ModelSource, remote: &RemoteFile) -> Result<GgufInfo> {
    let mut buf: Vec<u8> = Vec::new();
    let cancel = AtomicBool::new(false);
    for step in STEPS {
        let want = step.min(remote.size);
        if (buf.len() as u64) < want {
            let target = want as usize;
            let stopped = source.fetch(
                remote,
                buf.len() as u64,
                &mut |bytes| {
                    let room = target - buf.len();
                    buf.extend_from_slice(&bytes[..bytes.len().min(room)]);
                    // Enough: ending the transfer early is how a prefix is read.
                    if buf.len() >= target { Err(AiError::Cancelled) } else { Ok(()) }
                },
                &cancel,
            );
            match stopped {
                Ok(()) | Err(AiError::Cancelled) => {}
                Err(e) => return Err(e),
            }
        }
        match gguf::parse(Cursor::new(&buf[..]), remote.size) {
            Err(AiError::ModelFile(m)) if m.contains("truncated") && (buf.len() as u64) < remote.size => continue,
            other => return other,
        }
    }
    Err(AiError::ModelFile("the model header is unusually large".into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::gguf::{Kv, build, qwen_like};
    use std::sync::Mutex;
    use std::sync::atomic::Ordering;

    /// Serves a byte vector and records how much was requested.
    struct Fake {
        data: Vec<u8>,
        sent: Mutex<u64>,
        fail: bool,
    }
    impl ModelSource for Fake {
        fn describe(&self, _: &str, _: &str) -> Result<RemoteFile> {
            unreachable!()
        }
        fn fetch(&self, _: &RemoteFile, offset: u64, sink: &mut dyn FnMut(&[u8]) -> Result<()>, cancel: &AtomicBool) -> Result<()> {
            if self.fail {
                return Err(AiError::Network("offline".into()));
            }
            for c in self.data[offset as usize..].chunks(64 * 1024) {
                assert!(!cancel.load(Ordering::Relaxed));
                *self.sent.lock().unwrap() += c.len() as u64;
                sink(c)?;
            }
            Ok(())
        }
    }
    fn remote(size: u64) -> RemoteFile {
        RemoteFile {
            repo: "o/r".into(),
            file: "m.gguf".into(),
            size,
            sha256: String::new(),
            license: None,
            gated: false,
        }
    }

    #[test]
    fn reads_the_header_without_the_rest_of_the_file() {
        let mut data = qwen_like();
        data.extend(vec![0u8; 20 << 20]);
        let f = Fake {
            data,
            sent: Mutex::new(0),
            fail: false,
        };
        let size = f.data.len() as u64;
        let info = read_remote_header(&f, &remote(size)).unwrap();
        assert_eq!(info.architecture, "qwen3");
        assert_eq!(info.block_count, Some(36));
        assert!(*f.sent.lock().unwrap() <= 4 << 20, "only a prefix is requested");
    }

    #[test]
    fn a_header_larger_than_the_first_step_is_read_with_a_second_request() {
        let big: Vec<String> = (0..300_000).map(|i| format!("token-{i:08}-padding-padding")).collect();
        let data = build(
            "qwen3",
            &[("qwen3.block_count", Kv::U32(36)), ("tokenizer.ggml.tokens", Kv::StrArr(big))],
            1 << 20,
        );
        let size = data.len() as u64;
        assert!(size > 4 << 20);
        let f = Fake {
            data,
            sent: Mutex::new(0),
            fail: false,
        };
        let info = read_remote_header(&f, &remote(size)).unwrap();
        assert_eq!(info.block_count, Some(36));
        assert!(*f.sent.lock().unwrap() > 4 << 20);
    }

    #[test]
    fn host_errors_and_non_models_are_reported() {
        let off = Fake {
            data: vec![],
            sent: Mutex::new(0),
            fail: true,
        };
        assert!(matches!(read_remote_header(&off, &remote(1 << 20)), Err(AiError::Network(_))));
        let html = Fake {
            data: b"<!DOCTYPE html>".to_vec(),
            sent: Mutex::new(0),
            fail: false,
        };
        assert!(read_remote_header(&html, &remote(15)).is_err());
    }

    #[test]
    fn a_header_that_never_ends_is_refused() {
        let data = vec![b'G', b'G', b'U', b'F', 3, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0];
        let f = Fake {
            data,
            sent: Mutex::new(0),
            fail: false,
        };
        assert!(read_remote_header(&f, &remote(1 << 30)).is_err());
    }
}
