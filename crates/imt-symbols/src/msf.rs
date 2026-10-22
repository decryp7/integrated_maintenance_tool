//! The identity a PDB file records (its GUID and age), read from the MSF 7.0 container with a few bounded reads:
//! the superblock, the stream directory, the first bytes of the PDB information stream (1, GUID) and of the debug
//! information stream (3, the age the image's CodeView record names). Pure; never panics on any input. A
//! downloaded PDB is kept only when this matches the module it was asked for.

use crate::pe::ImageSource;

/// The first bytes of an MSF 7.0 (PDB) file.
pub const PDB_MAGIC: &[u8] = b"Microsoft C/C++ MSF 7.00\r\n\x1aDS\0\0\0";

/// Largest stream directory read.
const MAX_DIRECTORY: u32 = 16 << 20;
/// Most streams a directory may list.
const MAX_STREAMS: u32 = 1 << 20;

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn read(src: &mut dyn ImageSource, offset: u64, len: usize) -> Option<Vec<u8>> {
    let mut v = vec![0u8; len];
    (src.read_at(offset, &mut v) == len).then_some(v)
}

/// The GUID (stored layout) and age of the PDB in `src`; `None` when it is not a readable PDB.
pub fn identity(src: &mut dyn ImageSource) -> Option<([u8; 16], u32)> {
    let sb = read(src, 0, 56)?;
    if !sb.starts_with(PDB_MAGIC) {
        return None;
    }
    let block = u32_at(&sb, 32)?;
    let blocks = u32_at(&sb, 40)?;
    let dir_bytes = u32_at(&sb, 44)?;
    let map = u32_at(&sb, 52)?;
    if !block.is_power_of_two() || !(512..=65_536).contains(&block) || !(4..=MAX_DIRECTORY).contains(&dir_bytes) {
        return None;
    }
    let at = |index: u32| (index < blocks).then(|| u64::from(index) * u64::from(block));
    // The directory's blocks, listed in the block map's block(s).
    let dir_blocks = dir_bytes.div_ceil(block) as usize;
    let list = read(src, at(map)?, dir_blocks * 4)?;
    let mut dir = Vec::with_capacity(dir_bytes as usize);
    for i in 0..dir_blocks {
        let want = (dir_bytes as usize - dir.len()).min(block as usize);
        dir.extend(read(src, at(u32_at(&list, i * 4)?)?, want)?);
    }
    let streams = u32_at(&dir, 0)?;
    if !(2..=MAX_STREAMS).contains(&streams) {
        return None;
    }
    let size = |i: u32| u32_at(&dir, 4 + 4 * i as usize).map(|s| if s == u32::MAX { 0 } else { s });
    // The first block of each stream: the block lists follow the sizes, in stream order.
    let mut next = 4 + 4 * streams as usize;
    let mut first = [None; 4];
    for i in 0..streams.min(4) {
        let n = size(i)?.div_ceil(block) as usize;
        if n > 0 {
            first[i as usize] = Some(u32_at(&dir, next)?);
        }
        next = next.checked_add(n.checked_mul(4)?)?;
    }
    // PDB information stream: version, signature, age, GUID.
    if size(1)? < 28 {
        return None;
    }
    let info = read(src, at(first[1]?)?, 28)?;
    let guid: [u8; 16] = info.get(12..28)?.try_into().ok()?;
    let mut age = u32_at(&info, 8)?;
    // The debug information stream's age is the one images record (the information stream's grows on rewrites).
    if streams > 3 && size(3)? >= 12 {
        if let Some(dbi) = first[3].and_then(at).and_then(|o| read(src, o, 12)) {
            if u32_at(&dbi, 0) == Some(u32::MAX) {
                age = u32_at(&dbi, 8)?;
            }
        }
    }
    Some((guid, age))
}

/// A minimal PDB recording `guid` and `age` (information and debug information streams only), for tests of IMT's
/// symbol handling. Not a PDB any tool can read symbols from.
#[doc(hidden)]
pub fn synthetic(guid: &[u8; 16], age: u32) -> Vec<u8> {
    const B: usize = 512;
    let mut f = vec![0u8; 7 * B];
    let put = |f: &mut Vec<u8>, at: usize, v: u32| f[at..at + 4].copy_from_slice(&v.to_le_bytes());
    f[..PDB_MAGIC.len()].copy_from_slice(PDB_MAGIC);
    put(&mut f, 32, B as u32);
    put(&mut f, 36, 1);
    put(&mut f, 40, 7);
    // Directory: 4 streams, sizes [0, 28, 0, 12], blocks of 1 and 3.
    let dir: [u32; 7] = [4, 0, 28, 0, 12, 5, 6];
    put(&mut f, 44, (dir.len() * 4) as u32);
    put(&mut f, 52, 3);
    put(&mut f, 3 * B, 4);
    for (i, v) in dir.iter().enumerate() {
        put(&mut f, 4 * B + 4 * i, *v);
    }
    put(&mut f, 5 * B, 20_000_404);
    put(&mut f, 5 * B + 8, age + 1);
    f[5 * B + 12..5 * B + 28].copy_from_slice(guid);
    put(&mut f, 6 * B, u32::MAX);
    put(&mut f, 6 * B + 4, 19_990_903);
    put(&mut f, 6 * B + 8, age);
    f
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_identity_is_the_guid_and_the_debug_information_age() {
        let g = [7u8; 16];
        let pdb = synthetic(&g, 3);
        assert_eq!(identity(&mut &pdb[..]), Some((g, 3)));
        // Without a debug information stream, the information stream's age.
        let mut no_dbi = pdb.clone();
        no_dbi[6 * 512..6 * 512 + 4].copy_from_slice(&0u32.to_le_bytes());
        assert_eq!(identity(&mut &no_dbi[..]), Some((g, 4)));
    }

    #[test]
    fn damaged_files_are_not_pdbs_and_never_panic() {
        let pdb = synthetic(&[1; 16], 1);
        assert_eq!(identity(&mut &b"<html>"[..]), None);
        for cut in [10, 56, 3 * 512, 4 * 512 + 8, 5 * 512 + 20] {
            assert_eq!(identity(&mut &pdb[..cut]), None, "{cut}");
        }
        let poke = |at: usize, v: u32| {
            let mut p = pdb.clone();
            p[at..at + 4].copy_from_slice(&v.to_le_bytes());
            identity(&mut &p[..])
        };
        assert_eq!(poke(32, 500), None, "block size");
        assert_eq!(poke(44, u32::MAX), None, "directory size");
        assert_eq!(poke(52, 99), None, "block map past the end");
        assert_eq!(poke(4 * 512, u32::MAX), None, "stream count");
        assert_eq!(poke(4 * 512 + 8, 4), None, "information stream too small");
        assert_eq!(poke(4 * 512 + 20, 1000), None, "a block past the end");
        // Any bytes after the magic: an answer or nothing, never a panic.
        let mut x = pdb.clone();
        for (i, b) in x.iter_mut().enumerate().skip(PDB_MAGIC.len()) {
            *b = (i * 31 % 251) as u8;
        }
        let _ = identity(&mut &x[..]);
    }
}
