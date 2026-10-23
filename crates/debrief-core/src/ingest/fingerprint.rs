//! A cheap identity of a file's content, kept with its source so a renamed or moved file can be recognised when the
//! folder is read again (ADR 0050): the size and a hash of the first and the last 64 KiB.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

/// Bytes hashed at each end of a file.
const EDGE: u64 = 64 << 10;

/// Size and edge hash of a file. Equal fingerprints mean "very probably the same content", not proof: callers match
/// only when exactly one candidate has it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Fingerprint {
    pub size: u64,
    pub hash: u64,
}

fn fnv(mut h: u64, bytes: &[u8]) -> u64 {
    for b in bytes {
        h ^= u64::from(*b);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// The fingerprint of `path`, or `None` when it cannot be read.
pub fn fingerprint(path: &Path) -> Option<Fingerprint> {
    let mut f = std::fs::File::open(path).ok()?;
    let size = f.metadata().ok()?.len();
    let mut hash = 0xcbf2_9ce4_8422_2325u64;
    let mut buf = vec![0u8; EDGE.min(size) as usize];
    f.read_exact(&mut buf).ok()?;
    hash = fnv(hash, &buf);
    if size > EDGE {
        let tail = EDGE.min(size - EDGE);
        buf.truncate(tail as usize);
        buf.resize(tail as usize, 0);
        f.seek(SeekFrom::Start(size - tail)).ok()?;
        f.read_exact(&mut buf).ok()?;
        hash = fnv(hash, &buf);
    }
    Some(Fingerprint { size, hash })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_content_has_equal_fingerprints_and_an_edit_at_either_end_changes_it() {
        let d = std::env::temp_dir().join(format!("debrief-fp-{}", std::process::id()));
        std::fs::create_dir_all(&d).unwrap();
        let big: Vec<u8> = (0..300_000u32).map(|i| (i % 251) as u8).collect();
        let (a, b, c, e, s) = (d.join("a"), d.join("b"), d.join("c"), d.join("e"), d.join("s"));
        std::fs::write(&a, &big).unwrap();
        std::fs::write(&b, &big).unwrap();
        let mut head = big.clone();
        head[3] ^= 1;
        std::fs::write(&c, &head).unwrap();
        let mut tail = big.clone();
        *tail.last_mut().unwrap() ^= 1;
        std::fs::write(&e, &tail).unwrap();
        std::fs::write(&s, b"tiny").unwrap();
        let fp = |p: &Path| fingerprint(p).unwrap();
        assert_eq!(fp(&a), fp(&b));
        assert_ne!(fp(&a), fp(&c));
        assert_ne!(fp(&a), fp(&e));
        assert_eq!(fp(&s).size, 4);
        assert!(fingerprint(&d.join("missing")).is_none());
        let _ = std::fs::remove_dir_all(&d);
    }
}
