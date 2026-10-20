//! Keys of the workspace's result caches: the record generation plus a 64-bit hash of the request's JSON
//! form, streamed into the hasher (no text copy of the request is kept). A new generation (records
//! re-sorted, rewritten or evicted, a new profile) never meets an old entry, whatever the request.

use serde::Serialize;
use std::hash::{DefaultHasher, Hasher};

/// What a cached result was computed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) struct CacheKey {
    generation: u64,
    hash: u64,
}

impl CacheKey {
    /// The key of `request` at record `generation`.
    pub(crate) fn of<T: Serialize + ?Sized>(generation: u64, request: &T) -> serde_json::Result<CacheKey> {
        let mut sink = HashWriter(DefaultHasher::new());
        serde_json::to_writer(&mut sink, request)?;
        Ok(CacheKey {
            generation,
            hash: sink.0.finish(),
        })
    }
}

/// Feeds serialised bytes straight into a hasher.
struct HashWriter(DefaultHasher);

impl std::io::Write for HashWriter {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.0.write(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys_follow_the_request_and_the_generation() {
        let a = CacheKey::of(1, &("level", 3)).unwrap();
        assert_eq!(a, CacheKey::of(1, &("level", 3)).unwrap());
        assert_ne!(a, CacheKey::of(1, &("level", 4)).unwrap());
        assert_ne!(a, CacheKey::of(2, &("level", 3)).unwrap(), "a new generation misses");
        assert!(CacheKey::of(0, &f64::NAN).is_ok(), "NaN serialises as null");
        let mut map = std::collections::BTreeMap::new();
        map.insert(vec![1u8], 1);
        assert!(CacheKey::of(0, &map).is_err(), "a key that is not text cannot be JSON");
        let mut w = HashWriter(DefaultHasher::new());
        assert!(std::io::Write::flush(&mut w).is_ok());
    }
}
