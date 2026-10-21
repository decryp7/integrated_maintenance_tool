//! What a PE image (`.exe`, `.dll`) says about itself: machine, image size, time stamp and the identity of its
//! PDB (the CodeView `RSDS` record of the debug directory), which names the PDB in a symbol store. Pure; reads
//! through a bounded [`ImageSource`] so a huge or hostile file costs at most a few reads; never panics.

use std::fmt;

/// Machine types IMT symbolises.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Machine {
    X86,
    X64,
    Arm64,
    Other(u16),
}

impl Machine {
    fn from_raw(m: u16) -> Machine {
        match m {
            0x014c => Machine::X86,
            0x8664 => Machine::X64,
            0xaa64 => Machine::Arm64,
            other => Machine::Other(other),
        }
    }

    /// Pointer size in bytes.
    pub fn pointer_size(self) -> u32 {
        match self {
            Machine::X86 => 4,
            _ => 8,
        }
    }
}

/// The identity of a PDB: what a symbol store files it under.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct PdbId {
    /// The GUID as stored (Data1..Data3 little-endian, Data4 bytes).
    pub guid: [u8; 16],
    pub age: u32,
    /// The PDB path the linker recorded (often an absolute build path).
    pub path: String,
}

impl PdbId {
    /// The PDB's file name (`busy.pdb`).
    pub fn file_name(&self) -> &str {
        self.path.rsplit(['\\', '/']).next().unwrap_or(&self.path)
    }

    /// The symbol-store key: the GUID in upper-case hex (Data1, Data2, Data3 as numbers, then Data4) followed by
    /// the age in hex, as `symstore` and symbol servers use it (`<name>/<key>/<name>`).
    pub fn key(&self) -> String {
        let g = &self.guid;
        let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
        let d2 = u16::from_le_bytes([g[4], g[5]]);
        let d3 = u16::from_le_bytes([g[6], g[7]]);
        let mut s = format!("{d1:08X}{d2:04X}{d3:04X}");
        for b in &g[8..] {
            s.push_str(&format!("{b:02X}"));
        }
        s.push_str(&format!("{:X}", self.age));
        s
    }
}

/// What [`parse`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PeInfo {
    pub machine: Machine,
    /// `SizeOfImage`: the mapped size.
    pub image_size: u32,
    /// The COFF header's `TimeDateStamp`.
    pub timestamp: u32,
    /// The image's preferred base address.
    pub image_base: u64,
    /// The CodeView PDB identity, when the image has one.
    pub pdb: Option<PdbId>,
}

/// Why a file is not a PE image the parser can read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PeError {
    #[error("not a PE image ({0})")]
    NotPe(&'static str),
    #[error("the PE image is truncated or damaged ({0})")]
    Damaged(&'static str),
}

/// Bytes of an image by offset; reads past the end return fewer bytes.
pub trait ImageSource {
    /// Fills `out` from `offset`; returns how many bytes were read.
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> usize;
}

impl ImageSource for &[u8] {
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> usize {
        let Ok(start) = usize::try_from(offset) else { return 0 };
        let Some(src) = self.get(start..) else { return 0 };
        let n = src.len().min(out.len());
        out[..n].copy_from_slice(&src[..n]);
        n
    }
}

impl ImageSource for std::fs::File {
    fn read_at(&mut self, offset: u64, out: &mut [u8]) -> usize {
        use std::io::{Read, Seek, SeekFrom};
        if self.seek(SeekFrom::Start(offset)).is_err() {
            return 0;
        }
        let mut n = 0;
        while n < out.len() {
            match self.read(&mut out[n..]) {
                Ok(0) | Err(_) => break,
                Ok(k) => n += k,
            }
        }
        n
    }
}

/// Most section headers read.
const MAX_SECTIONS: usize = 96;
/// Most debug directory entries read.
const MAX_DEBUG_ENTRIES: usize = 32;
/// Longest PDB path kept.
const MAX_PDB_PATH: usize = 1024;
/// `IMAGE_DEBUG_TYPE_CODEVIEW`.
const DEBUG_TYPE_CODEVIEW: u32 = 2;

fn read_vec(src: &mut dyn ImageSource, offset: u64, len: usize) -> Vec<u8> {
    let mut v = vec![0u8; len];
    let n = src.read_at(offset, &mut v);
    v.truncate(n);
    v
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(at..at + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(at..at + 4)?.try_into().ok()?))
}

fn u64_at(b: &[u8], at: usize) -> Option<u64> {
    Some(u64::from_le_bytes(b.get(at..at + 8)?.try_into().ok()?))
}

/// One section: virtual range and its file offset.
#[derive(Debug, Clone, Copy)]
struct Section {
    va: u32,
    vsize: u32,
    raw: u32,
    raw_size: u32,
}

fn rva_to_offset(sections: &[Section], rva: u32) -> Option<u64> {
    sections.iter().find_map(|s| {
        let size = s.vsize.max(s.raw_size);
        (rva >= s.va && rva - s.va < size).then(|| u64::from(s.raw) + u64::from(rva - s.va))
    })
}

/// Parses the headers and the CodeView record of the image in `src`.
pub fn parse(src: &mut dyn ImageSource) -> Result<PeInfo, PeError> {
    let dos = read_vec(src, 0, 64);
    if dos.len() < 64 || &dos[..2] != b"MZ" {
        return Err(PeError::NotPe("no MZ header"));
    }
    let pe_off = u64::from(u32_at(&dos, 0x3c).ok_or(PeError::Damaged("DOS header"))?);
    if pe_off > 1 << 20 {
        return Err(PeError::Damaged("PE header offset"));
    }
    // Signature, COFF header (20 bytes) and the optional header (at most 240 bytes for PE32+).
    let head = read_vec(src, pe_off, 4 + 20 + 240);
    if head.len() < 24 || &head[..4] != b"PE\0\0" {
        return Err(PeError::NotPe("no PE signature"));
    }
    let coff = &head[4..24];
    let machine = Machine::from_raw(u16_at(coff, 0).ok_or(PeError::Damaged("COFF header"))?);
    let sections_n = usize::from(u16_at(coff, 2).ok_or(PeError::Damaged("COFF header"))?);
    let timestamp = u32_at(coff, 4).ok_or(PeError::Damaged("COFF header"))?;
    let opt_size = usize::from(u16_at(coff, 16).ok_or(PeError::Damaged("COFF header"))?);
    let opt = head.get(24..24 + opt_size.min(240)).ok_or(PeError::Damaged("optional header"))?;
    let magic = u16_at(opt, 0).ok_or(PeError::Damaged("optional header"))?;
    // Field offsets of PE32 (0x10b) and PE32+ (0x20b).
    let (image_base, size_at, dirs_count_at, dirs_at) = match magic {
        0x10b => (u64::from(u32_at(opt, 28).ok_or(PeError::Damaged("image base"))?), 56, 92, 96),
        0x20b => (u64_at(opt, 24).ok_or(PeError::Damaged("image base"))?, 56, 108, 112),
        _ => return Err(PeError::NotPe("unknown optional header")),
    };
    let image_size = u32_at(opt, size_at).ok_or(PeError::Damaged("image size"))?;
    let dirs = u32_at(opt, dirs_count_at).unwrap_or(0) as usize;
    let mut info = PeInfo {
        machine,
        image_size,
        timestamp,
        image_base,
        pdb: None,
    };
    // Directory 6: debug.
    if dirs <= 6 {
        return Ok(info);
    }
    let (Some(debug_rva), Some(debug_size)) = (u32_at(opt, dirs_at + 6 * 8), u32_at(opt, dirs_at + 6 * 8 + 4)) else {
        return Ok(info);
    };
    if debug_rva == 0 || debug_size == 0 {
        return Ok(info);
    }
    let table = read_vec(src, pe_off + 24 + opt_size as u64, sections_n.min(MAX_SECTIONS) * 40);
    let sections: Vec<Section> = table
        .as_chunks::<40>()
        .0
        .iter()
        .filter_map(|s| {
            Some(Section {
                vsize: u32_at(s, 8)?,
                va: u32_at(s, 12)?,
                raw_size: u32_at(s, 16)?,
                raw: u32_at(s, 20)?,
            })
        })
        .collect();
    let Some(debug_off) = rva_to_offset(&sections, debug_rva) else {
        return Ok(info);
    };
    let entries = (debug_size as usize / 28).min(MAX_DEBUG_ENTRIES);
    let dir = read_vec(src, debug_off, entries * 28);
    for e in dir.as_chunks::<28>().0 {
        if u32_at(e, 12) != Some(DEBUG_TYPE_CODEVIEW) {
            continue;
        }
        let (Some(size), Some(raw)) = (u32_at(e, 16), u32_at(e, 24)) else {
            continue;
        };
        if size < 24 {
            continue;
        }
        let cv = read_vec(src, u64::from(raw), (size as usize).min(24 + MAX_PDB_PATH));
        if cv.len() < 24 || &cv[..4] != b"RSDS" {
            continue;
        }
        let mut guid = [0u8; 16];
        guid.copy_from_slice(&cv[4..20]);
        let age = u32_at(&cv, 20).unwrap_or(0);
        let name = &cv[24..];
        let end = name.iter().position(|&b| b == 0).unwrap_or(name.len());
        info.pdb = Some(PdbId {
            guid,
            age,
            path: String::from_utf8_lossy(&name[..end]).into_owned(),
        });
        break;
    }
    Ok(info)
}

/// Parses the image file at `path`.
pub fn parse_file(path: &std::path::Path) -> Result<PeInfo, PeError> {
    let mut f = std::fs::File::open(path).map_err(|_| PeError::NotPe("the file cannot be opened"))?;
    parse(&mut f)
}

impl fmt::Display for Machine {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Machine::X86 => f.write_str("x86"),
            Machine::X64 => f.write_str("x64"),
            Machine::Arm64 => f.write_str("arm64"),
            Machine::Other(m) => write!(f, "machine 0x{m:x}"),
        }
    }
}

/// A minimal PE image (tests, fuzz seeds): PE32 or PE32+ with one section holding a debug directory and an
/// `RSDS` record for `pdb`.
#[doc(hidden)]
pub fn synthetic(pe32_plus: bool, pdb: Option<(&[u8; 16], u32, &str)>) -> Vec<u8> {
    let mut b = vec![0u8; 0x400];
    b[..2].copy_from_slice(b"MZ");
    b[0x3c..0x40].copy_from_slice(&0x80u32.to_le_bytes());
    b[0x80..0x84].copy_from_slice(b"PE\0\0");
    let opt_size: u16 = if pe32_plus { 240 } else { 224 };
    let coff = 0x84;
    let machine: u16 = if pe32_plus { 0x8664 } else { 0x014c };
    b[coff..coff + 2].copy_from_slice(&machine.to_le_bytes());
    b[coff + 2..coff + 4].copy_from_slice(&1u16.to_le_bytes());
    b[coff + 4..coff + 8].copy_from_slice(&0x6000_0000u32.to_le_bytes());
    b[coff + 16..coff + 18].copy_from_slice(&opt_size.to_le_bytes());
    let opt = coff + 20;
    let (magic, dirs_count_at, dirs_at): (u16, usize, usize) = if pe32_plus { (0x20b, 108, 112) } else { (0x10b, 92, 96) };
    b[opt..opt + 2].copy_from_slice(&magic.to_le_bytes());
    if pe32_plus {
        b[opt + 24..opt + 32].copy_from_slice(&0x1_4000_0000u64.to_le_bytes());
    } else {
        b[opt + 28..opt + 32].copy_from_slice(&0x40_0000u32.to_le_bytes());
    }
    b[opt + 56..opt + 60].copy_from_slice(&0x3000u32.to_le_bytes());
    b[opt + dirs_count_at..opt + dirs_count_at + 4].copy_from_slice(&16u32.to_le_bytes());
    // One section .rdata: VA 0x1000, raw 0x200, size 0x200.
    let sec = opt + opt_size as usize;
    b[sec..sec + 6].copy_from_slice(b".rdata");
    b[sec + 8..sec + 12].copy_from_slice(&0x200u32.to_le_bytes());
    b[sec + 12..sec + 16].copy_from_slice(&0x1000u32.to_le_bytes());
    b[sec + 16..sec + 20].copy_from_slice(&0x200u32.to_le_bytes());
    b[sec + 20..sec + 24].copy_from_slice(&0x200u32.to_le_bytes());
    if let Some((guid, age, path)) = pdb {
        // Debug directory at RVA 0x1000 (file 0x200), CodeView record at file 0x240.
        b[opt + dirs_at + 48..opt + dirs_at + 52].copy_from_slice(&0x1000u32.to_le_bytes());
        b[opt + dirs_at + 52..opt + dirs_at + 56].copy_from_slice(&28u32.to_le_bytes());
        let mut cv = b"RSDS".to_vec();
        cv.extend_from_slice(guid);
        cv.extend_from_slice(&age.to_le_bytes());
        cv.extend_from_slice(path.as_bytes());
        cv.push(0);
        let d = 0x200;
        b[d + 12..d + 16].copy_from_slice(&DEBUG_TYPE_CODEVIEW.to_le_bytes());
        b[d + 16..d + 20].copy_from_slice(&(cv.len() as u32).to_le_bytes());
        b[d + 20..d + 24].copy_from_slice(&0x1040u32.to_le_bytes());
        b[d + 24..d + 28].copy_from_slice(&0x240u32.to_le_bytes());
        b[0x240..0x240 + cv.len()].copy_from_slice(&cv);
    }
    b
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: [u8; 16] = [
        0xb9, 0xdb, 0x44, 0x38, 0x17, 0x20, 0x67, 0x49, 0xbe, 0x7a, 0xa4, 0xa2, 0xc2, 0x04, 0x30, 0xfa,
    ];

    #[test]
    fn reads_both_header_kinds_and_the_pdb_identity() {
        for plus in [false, true] {
            let img = synthetic(plus, Some((&GUID, 2, r"D:\build\busy.pdb")));
            let info = parse(&mut img.as_slice()).unwrap();
            assert_eq!(info.machine, if plus { Machine::X64 } else { Machine::X86 });
            assert_eq!(info.machine.pointer_size(), if plus { 8 } else { 4 });
            assert_eq!(info.image_size, 0x3000);
            assert_eq!(info.timestamp, 0x6000_0000);
            assert_eq!(info.image_base, if plus { 0x1_4000_0000 } else { 0x40_0000 });
            let pdb = info.pdb.unwrap();
            assert_eq!(pdb.file_name(), "busy.pdb");
            assert_eq!(pdb.key(), "3844DBB920174967BE7AA4A2C20430FA2");
        }
        let none = parse(&mut synthetic(true, None).as_slice()).unwrap();
        assert!(none.pdb.is_none());
        assert_eq!(Machine::X64.to_string(), "x64");
        assert_eq!(Machine::Other(0x1c0).to_string(), "machine 0x1c0");
    }

    #[test]
    fn hostile_and_truncated_input_is_an_error_never_a_panic() {
        assert_eq!(parse(&mut [].as_slice()), Err(PeError::NotPe("no MZ header")));
        assert!(parse(&mut b"MZ".repeat(40).as_slice()).is_err());
        let img = synthetic(true, Some((&GUID, 1, "x.pdb")));
        for n in 0..img.len() {
            let _ = parse(&mut &img[..n]);
        }
        let mut bad = img.clone();
        bad[0x3c..0x40].copy_from_slice(&u32::MAX.to_le_bytes());
        assert_eq!(parse(&mut bad.as_slice()), Err(PeError::Damaged("PE header offset")));
        let mut bad = img.clone();
        bad[0x98..0x9a].copy_from_slice(&0x999u16.to_le_bytes());
        assert_eq!(parse(&mut bad.as_slice()), Err(PeError::NotPe("unknown optional header")));
    }

    #[test]
    fn the_test_binary_itself_has_a_pdb() {
        let exe = std::env::current_exe().unwrap();
        let Ok(info) = parse_file(&exe) else {
            // Not a PE (Linux, macOS): nothing to check.
            assert!(!std::hint::black_box(cfg!(windows)), "Windows test binaries are PE images");
            return;
        };
        assert!(info.image_size > 0);
        if cfg!(all(windows, target_env = "msvc")) {
            let pdb = info.pdb.expect("MSVC test binaries are linked with a PDB");
            assert!(pdb.file_name().to_ascii_lowercase().ends_with(".pdb"), "{pdb:?}");
            assert_eq!(pdb.key().len(), 32 + format!("{:X}", pdb.age).len());
        }
        assert!(parse_file(std::path::Path::new("/definitely/not/here.dll")).is_err());
    }
}
