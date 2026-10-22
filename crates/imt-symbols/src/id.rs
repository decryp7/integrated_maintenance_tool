//! A PDB's identity as symbol stores file it: its file name, GUID and age (the module's CodeView `RSDS` record).
//! Parsed once from untrusted text (a dump's module list) or a parsed image ([`crate::pe::PdbId`]); afterwards a
//! [`PdbKey`] is always a safe path component and URL segment with a well-formed key, so lookups, cache paths and
//! downloads cannot be steered by a hostile file. Symbols are only ever matched by this identity, never by a file
//! name alone.

use crate::pe::PdbId;
use std::fmt;
use std::path::{Path, PathBuf};

/// Longest PDB file name accepted.
pub const MAX_NAME: usize = 255;

/// Why a module's PDB identity is unusable.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum KeyError {
    #[error("the module names no PDB file")]
    NoPdb,
    #[error("\"{0}\" is not a usable PDB file name")]
    UnsafeName(String),
    #[error("\"{0}\" is not a PDB identifier (32 hex digits of GUID and the age)")]
    BadId(String),
}

/// A PDB file name (`busy.pdb`) that is one path component of safe characters ending in `.pdb`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PdbName(String);

impl PdbName {
    /// The file name of `path` (the PDB path a linker recorded, often an absolute build path), if it is safe.
    pub fn parse(path: &str) -> Result<PdbName, KeyError> {
        let name = path.trim().rsplit(['\\', '/']).next().unwrap_or_default();
        if name.is_empty() {
            return Err(KeyError::NoPdb);
        }
        let ok = name.len() <= MAX_NAME
            && name.to_ascii_lowercase().ends_with(".pdb")
            && !name.starts_with('.')
            && name
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '+' | ' ' | '(' | ')'));
        if ok {
            Ok(PdbName(name.to_string()))
        } else {
            Err(KeyError::UnsafeName(name.chars().take(MAX_NAME).collect()))
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PdbName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// A PDB as a symbol store files it: `<name>/<GUID><age>/<name>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PdbKey {
    name: PdbName,
    /// The GUID as stored (Data1..Data3 little-endian, then the Data4 bytes).
    guid: [u8; 16],
    age: u32,
}

/// The symbol-store key of a GUID (stored layout) and age: Data1, Data2, Data3 as upper-case hex numbers, the Data4
/// bytes, then the age in hex.
pub(crate) fn store_key(g: &[u8; 16], age: u32) -> String {
    let d1 = u32::from_le_bytes([g[0], g[1], g[2], g[3]]);
    let d2 = u16::from_le_bytes([g[4], g[5]]);
    let d3 = u16::from_le_bytes([g[6], g[7]]);
    let mut s = format!("{d1:08X}{d2:04X}{d3:04X}");
    for b in &g[8..] {
        s.push_str(&format!("{b:02X}"));
    }
    s.push_str(&format!("{age:X}"));
    s
}

impl PdbKey {
    pub fn new(name: PdbName, guid: [u8; 16], age: u32) -> PdbKey {
        PdbKey { name, guid, age }
    }

    /// The identity a parsed image records.
    pub fn from_pdb_id(id: &PdbId) -> Result<PdbKey, KeyError> {
        Ok(PdbKey::new(PdbName::parse(&id.path)?, id.guid, id.age))
    }

    /// From a dump module's debug file (`D:\build\busy.pdb`) and identifier (`<32 hex digits of GUID><age in hex>`,
    /// as breakpad and symbol stores spell it; dashes and braces are ignored, case does not matter).
    pub fn parse(debug_file: &str, debug_id: &str) -> Result<PdbKey, KeyError> {
        let name = PdbName::parse(debug_file)?;
        let bad = || KeyError::BadId(debug_id.chars().take(80).collect());
        let hex: String = debug_id.trim().chars().filter(|c| !matches!(c, '-' | '{' | '}')).collect();
        if !(33..=40).contains(&hex.len()) || !hex.bytes().all(|b| b.is_ascii_hexdigit()) {
            return Err(bad());
        }
        let num = |range: std::ops::Range<usize>| u32::from_str_radix(&hex[range], 16).map_err(|_| bad());
        let mut guid = [0u8; 16];
        guid[..4].copy_from_slice(&num(0..8)?.to_le_bytes());
        guid[4..6].copy_from_slice(&(num(8..12)? as u16).to_le_bytes());
        guid[6..8].copy_from_slice(&(num(12..16)? as u16).to_le_bytes());
        for (i, b) in guid[8..].iter_mut().enumerate() {
            *b = num(16 + 2 * i..18 + 2 * i)? as u8;
        }
        let age = num(32..hex.len())?;
        Ok(PdbKey::new(name, guid, age))
    }

    pub fn name(&self) -> &PdbName {
        &self.name
    }

    pub fn guid(&self) -> &[u8; 16] {
        &self.guid
    }

    pub fn age(&self) -> u32 {
        self.age
    }

    /// The symbol-store key: `<GUID><age>` in upper-case hex.
    pub fn key(&self) -> String {
        store_key(&self.guid, self.age)
    }

    /// The path in a symbol store or on a symbol server: `busy.pdb/<key>/busy.pdb`.
    pub fn store_path(&self) -> String {
        format!("{0}/{1}/{0}", self.name, self.key())
    }

    /// Where a symbol store (or IMT's cache) in `root` keeps this PDB.
    pub fn store_file(&self, root: &Path) -> PathBuf {
        root.join(self.name.as_str()).join(self.key()).join(self.name.as_str())
    }

    /// Whether a PDB recording `guid` and `age` is this one.
    pub fn matches(&self, guid: &[u8; 16], age: u32) -> bool {
        self.guid == *guid && self.age == age
    }
}

impl fmt::Display for PdbKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ({})", self.name, self.key())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const GUID: [u8; 16] = [
        0xb9, 0xdb, 0x44, 0x38, 0x17, 0x20, 0x67, 0x49, 0xbe, 0x7a, 0xa4, 0xa2, 0xc2, 0x04, 0x30, 0xfa,
    ];

    #[test]
    fn identifiers_round_trip_through_the_store_key() {
        let k = PdbKey::parse(r"D:\build\busy.pdb", "3844DBB920174967BE7AA4A2C20430FA2").unwrap();
        assert_eq!((k.guid(), k.age(), k.name().as_str()), (&GUID, 2, "busy.pdb"));
        assert_eq!(k.key(), "3844DBB920174967BE7AA4A2C20430FA2");
        assert_eq!(k.store_path(), "busy.pdb/3844DBB920174967BE7AA4A2C20430FA2/busy.pdb");
        assert_eq!(
            k.store_file(Path::new("c")),
            Path::new("c").join("busy.pdb").join(k.key()).join("busy.pdb")
        );
        assert_eq!(k.to_string(), "busy.pdb (3844DBB920174967BE7AA4A2C20430FA2)");
        // Case, dashes and braces do not matter; the age may have several digits.
        let lower = PdbKey::parse("busy.pdb", "{3844dbb9-2017-4967-be7a-a4a2c20430fa}2").unwrap();
        assert_eq!(lower, k);
        let aged = PdbKey::parse("busy.pdb", "3844DBB920174967BE7AA4A2C20430FA1f").unwrap();
        assert_eq!((aged.age(), aged.key().ends_with("1F")), (0x1f, true));
        assert!(k.matches(&GUID, 2) && !k.matches(&GUID, 3) && !k.matches(&[0; 16], 2));
        let id = PdbId {
            guid: GUID,
            age: 2,
            path: "/b/busy.pdb".into(),
        };
        assert_eq!(PdbKey::from_pdb_id(&id).unwrap(), k);
        assert_eq!(id.key(), k.key());
    }

    #[test]
    fn hostile_names_and_identifiers_are_refused() {
        assert_eq!(PdbName::parse(""), Err(KeyError::NoPdb));
        assert_eq!(PdbName::parse(r"C:\x\"), Err(KeyError::NoPdb));
        assert_eq!(PdbName::parse("libc.so.6"), Err(KeyError::UnsafeName("libc.so.6".into())));
        // Only the last component is taken, so a traversal never reaches a path.
        assert_eq!(PdbName::parse(r"..\..\evil.pdb").unwrap().as_str(), "evil.pdb");
        assert_eq!(PdbName::parse("evil.pdb/..").unwrap_err(), KeyError::UnsafeName("..".into()));
        for bad in [
            ".pdb",
            "a:b.pdb",
            "con\0.pdb",
            "a.dll",
            "%2e.pdb",
            &format!("{}.pdb", "a".repeat(MAX_NAME)),
        ] {
            assert!(PdbName::parse(bad).is_err(), "{bad}");
        }
        for id in [
            "",
            "3844DBB920174967BE7AA4A2C20430FA",
            "3844DBB920174967BE7AA4A2C20430FAZ",
            "3844DBB920174967BE7AA4A2C20430FA123456789",
            "é844DBB920174967BE7AA4A2C20430FA2",
        ] {
            assert!(matches!(PdbKey::parse("a.pdb", id), Err(KeyError::BadId(_))), "{id}");
        }
        assert_eq!(
            PdbKey::parse("a.dll", "3844DBB920174967BE7AA4A2C20430FA2").unwrap_err(),
            KeyError::UnsafeName("a.dll".into())
        );
    }
}
