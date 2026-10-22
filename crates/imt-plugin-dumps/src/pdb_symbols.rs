//! Function names from PDB files in local symbol folders: a pure-Rust [`SymbolProvider`] (the `pdb` crate), so
//! dumps symbolise on every platform. PDBs are found **by the module's identity** (PDB name, GUID and age), never
//! by a file name alone: folders are searched in the symbol store layout (`<folder>/<name>.pdb/<GUID><age>/<name>.pdb`,
//! what `symchk`, Visual Studio caches and IMT's own symbol cache write) and flat (`<folder>/<name>.pdb`), and a PDB
//! whose GUID and age do not match the module is not used (ADR 0032). Procedures (with their length) come from the
//! module streams, public symbols fill the gaps. Nothing is downloaded here: the host fills IMT's symbol cache from
//! the user's symbol servers and passes it as a folder.

use imt_dump::model::SymbolStatus;
use imt_dump::symbols::{PdbKey, Symbol, SymbolProvider};
use pdb::FallibleIterator;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

/// Largest PDB opened.
const MAX_PDB_BYTES: u64 = 2 << 30;
/// Symbols kept per PDB.
const MAX_SYMBOLS: usize = 4_000_000;
/// PDBs kept open (parsed) at once.
const MAX_TABLES: usize = 256;
/// A public symbol (no length) covers at most this many bytes after its start.
const PUBLIC_REACH: u64 = 1 << 20;
/// Longest name kept.
const MAX_NAME: usize = 1024;

/// One PDB's functions, sorted by address.
#[derive(Debug, Default)]
pub struct Table {
    /// `(rva, length (0 = unknown), name)`.
    entries: Vec<(u32, u32, String)>,
}

impl Table {
    /// The function containing `rva`.
    pub fn lookup(&self, rva: u64) -> Option<Symbol> {
        let rva32 = u32::try_from(rva).ok()?;
        let i = self.entries.partition_point(|e| e.0 <= rva32).checked_sub(1)?;
        let (start, len, name) = &self.entries[i];
        let offset = rva - u64::from(*start);
        let inside = if *len > 0 {
            offset < u64::from(*len)
        } else {
            offset < PUBLIC_REACH
        };
        inside.then(|| Symbol {
            function: name.clone(),
            offset,
            file: None,
            line: None,
        })
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

/// `?Add@Cache@@QEAAXH@Z` → `Cache::Add` (MSVC decorated names, the simple cases); other names unchanged.
pub fn undecorate(name: &str) -> String {
    let Some(rest) = name.strip_prefix('?') else {
        return name.to_string();
    };
    if rest.starts_with('?') || rest.starts_with('$') {
        return name.to_string();
    }
    let Some(end) = rest.find("@@") else { return name.to_string() };
    let parts: Vec<&str> = rest[..end].split('@').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() {
        return name.to_string();
    }
    parts.into_iter().rev().collect::<Vec<_>>().join("::")
}

/// Why a PDB file was not used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refused {
    /// Another build's PDB (its GUID or age differ from the module's).
    Mismatch(String),
    /// Not readable as a PDB.
    Unreadable(String),
}

/// Reads the functions of the PDB at `path`, which must be `expected`'s build (same GUID and age).
pub fn read_table(path: &Path, expected: &PdbKey) -> Result<Table, Refused> {
    let unreadable = |e: &dyn std::fmt::Display| Refused::Unreadable(format!("{}: {e}", path.display()));
    let meta = std::fs::metadata(path).map_err(|e| unreadable(&e))?;
    if meta.len() > MAX_PDB_BYTES {
        return Err(unreadable(&format!("larger than {} MB", MAX_PDB_BYTES >> 20)));
    }
    let file = std::fs::File::open(path).map_err(|e| unreadable(&e))?;
    let mut pdb = pdb::PDB::open(file).map_err(|e| unreadable(&e))?;
    let info = pdb.pdb_information().map_err(|e| unreadable(&e))?;
    let age = pdb.debug_information().map(|d| d.age().unwrap_or(info.age)).unwrap_or(info.age);
    let id = format!("{}{:X}", info.guid.simple(), age).to_ascii_uppercase();
    if id != expected.key() {
        return Err(Refused::Mismatch(format!(
            "{} is another build (id {id}, the module wants {})",
            path.display(),
            expected.key()
        )));
    }
    read_functions(&mut pdb).map_err(|e| unreadable(&e))
}

/// The procedures and public functions of an opened PDB.
fn read_functions<'s, S: pdb::Source<'s> + 's>(pdb: &mut pdb::PDB<'s, S>) -> Result<Table, pdb::Error> {
    let map = pdb.address_map()?;
    let mut entries: Vec<(u32, u32, String)> = Vec::new();
    let add = |rva: Option<pdb::Rva>, len: u32, name: String, entries: &mut Vec<(u32, u32, String)>| {
        if let Some(r) = rva {
            if entries.len() < MAX_SYMBOLS {
                entries.push((r.0, len, imt_dump::limits::clip(&name, MAX_NAME)));
            }
        }
    };
    // Procedures with their lengths, from every module.
    if let Ok(dbi) = pdb.debug_information() {
        if let Ok(mut modules) = dbi.modules() {
            while let Ok(Some(m)) = modules.next() {
                let Ok(Some(info)) = pdb.module_info(&m) else { continue };
                let Ok(mut symbols) = info.symbols() else { continue };
                while let Ok(Some(s)) = symbols.next() {
                    if let Ok(pdb::SymbolData::Procedure(p)) = s.parse() {
                        add(p.offset.to_rva(&map), p.len, p.name.to_string().into_owned(), &mut entries);
                    }
                }
            }
        }
    }
    // Public symbols where no procedure starts.
    let procedures: std::collections::HashSet<u32> = entries.iter().map(|e| e.0).collect();
    if let Ok(globals) = pdb.global_symbols() {
        let mut it = globals.iter();
        while let Ok(Some(s)) = it.next() {
            if let Ok(pdb::SymbolData::Public(p)) = s.parse() {
                if p.function {
                    let rva = p.offset.to_rva(&map);
                    if rva.is_some_and(|r| !procedures.contains(&r.0)) {
                        add(rva, 0, undecorate(&p.name.to_string()), &mut entries);
                    }
                }
            }
        }
    }
    entries.sort_by_key(|e| e.0);
    entries.dedup_by_key(|e| e.0);
    Ok(Table { entries })
}

/// Where `key`'s PDB may be in `folder`: the symbol store layout first, then flat.
pub fn candidates(folder: &Path, key: &PdbKey) -> [PathBuf; 2] {
    [key.store_file(folder), folder.join(key.name().as_str())]
}

/// What looking for one PDB found.
type Found = (SymbolStatus, Option<Arc<Table>>);

/// PDBs from local folders, found by identity and parsed once per PDB.
pub struct PdbSymbols {
    folders: Vec<PathBuf>,
    tables: Mutex<HashMap<PdbKey, Found>>,
}

impl PdbSymbols {
    /// Absolute folders only; others are ignored.
    pub fn new(folders: impl IntoIterator<Item = PathBuf>) -> PdbSymbols {
        PdbSymbols {
            folders: folders.into_iter().filter(|f| f.is_absolute()).collect(),
            tables: Mutex::new(HashMap::new()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
    }

    fn find(&self, key: &PdbKey) -> Found {
        let mut refused = None;
        for path in self.folders.iter().flat_map(|f| candidates(f, key)) {
            if !path.is_file() {
                continue;
            }
            match read_table(&path, key) {
                Ok(t) => return (SymbolStatus::Loaded, Some(Arc::new(t))),
                // A mismatch says more than a damaged file; the first of each kind is kept.
                Err(Refused::Mismatch(detail)) if !matches!(refused, Some(SymbolStatus::Mismatch { .. })) => {
                    refused = Some(SymbolStatus::Mismatch { detail });
                }
                Err(Refused::Unreadable(detail)) if refused.is_none() => refused = Some(SymbolStatus::Unreadable { detail }),
                Err(_) => {}
            }
        }
        (refused.unwrap_or(SymbolStatus::Missing), None)
    }

    fn found(&self, key: &PdbKey) -> Found {
        if let Some(f) = self.tables.lock().unwrap_or_else(|p| p.into_inner()).get(key) {
            return f.clone();
        }
        let found = self.find(key);
        let mut tables = self.tables.lock().unwrap_or_else(|p| p.into_inner());
        if tables.len() < MAX_TABLES {
            tables.insert(key.clone(), found.clone());
        }
        found
    }
}

impl SymbolProvider for PdbSymbols {
    fn load(&self, key: &PdbKey) -> SymbolStatus {
        self.found(key).0
    }

    fn lookup(&self, key: &PdbKey, rva: u64) -> Option<Symbol> {
        self.found(key).1?.lookup(rva)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decorated_names_are_simplified() {
        assert_eq!(undecorate("?Add@Cache@@QEAAXH@Z"), "Cache::Add");
        assert_eq!(undecorate("?Get@Inner@Outer@@QEAAHXZ"), "Outer::Inner::Get");
        assert_eq!(undecorate("??0Cache@@QEAA@XZ"), "??0Cache@@QEAA@XZ");
        assert_eq!(undecorate("plain_c"), "plain_c");
        assert_eq!(undecorate("?nothing"), "?nothing");
        assert_eq!(undecorate("?@@"), "?@@");
    }

    #[test]
    fn tables_find_the_containing_function() {
        let t = Table {
            entries: vec![(0x1000, 0x20, "a".into()), (0x2000, 0, "b".into())],
        };
        assert_eq!(t.lookup(0x1010).unwrap().offset, 0x10);
        assert!(t.lookup(0x1020).is_none(), "past a procedure's length");
        assert_eq!(t.lookup(0x2100).unwrap().function, "b");
        assert!(t.lookup(0x2000 + PUBLIC_REACH).is_none());
        assert!(t.lookup(0x10).is_none());
        assert!(t.lookup(u64::MAX).is_none());
        assert_eq!((t.len(), t.is_empty()), (2, false));
    }

    fn key(name: &str) -> PdbKey {
        PdbKey::parse(name, "0123456789ABCDEF0123456789ABCDEF1").unwrap()
    }

    #[test]
    fn folders_are_searched_by_identity_in_store_and_flat_layout() {
        let k = key("app.pdb");
        let [store, flat] = candidates(Path::new("/sym"), &k);
        assert_eq!(store, Path::new("/sym/app.pdb/0123456789ABCDEF0123456789ABCDEF1/app.pdb"));
        assert_eq!(flat, Path::new("/sym/app.pdb"));
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.pdb"), b"not a pdb").unwrap();
        let s = PdbSymbols::new([dir.path().to_path_buf(), PathBuf::from("relative")]);
        assert!(!s.is_empty());
        assert!(matches!(s.load(&k), SymbolStatus::Unreadable { detail } if detail.contains("app.pdb")));
        assert!(s.lookup(&k, 0x1000).is_none());
        assert_eq!(s.load(&key("other.pdb")), SymbolStatus::Missing);
        let none = PdbSymbols::new(Vec::new());
        assert!(none.is_empty());
        assert_eq!(none.load(&k), SymbolStatus::Missing);
        assert!(none.lookup(&k, 0).is_none());
    }
    /// The test binary's own PDB (MSVC builds write one next to it) names this function.
    #[cfg(all(windows, target_env = "msvc", feature = "plugin"))]
    #[test]
    fn the_test_binary_pdb_names_its_functions() {
        #[inline(never)]
        fn marker_function_for_pdb_lookup() -> usize {
            std::hint::black_box(42)
        }
        let exe = std::env::current_exe().unwrap();
        let pdb = exe.with_extension("pdb");
        if !pdb.is_file() {
            eprintln!("no PDB next to the test binary; skipped");
            return;
        }
        // Its identity, from the image's CodeView record: the PDB is found by it and matches.
        let id = imt_symbols::pe::parse_file(&exe).unwrap().pdb.unwrap();
        let k = PdbKey::from_pdb_id(&id).unwrap();
        let table = read_table(&pdb, &k).unwrap();
        assert!(!table.is_empty());
        let other = PdbKey::new(k.name().clone(), [0; 16], 1);
        assert!(matches!(read_table(&pdb, &other), Err(Refused::Mismatch(m)) if m.contains("another build")));
        let base = {
            // The image base: the address of the module that holds this function.
            let f = marker_function_for_pdb_lookup as *const () as usize;
            let mut info = windows::Win32::System::Memory::MEMORY_BASIC_INFORMATION::default();
            // SAFETY: VirtualQuery only writes the struct we pass, with its size.
            let n = unsafe {
                windows::Win32::System::Memory::VirtualQuery(Some(f as *const std::ffi::c_void), &mut info, std::mem::size_of_val(&info))
            };
            assert!(n > 0);
            info.AllocationBase as usize
        };
        let rva = (marker_function_for_pdb_lookup as *const () as usize - base) as u64;
        let sym = table.lookup(rva).unwrap();
        assert!(sym.function.contains("marker_function_for_pdb_lookup"), "{}", sym.function);
        assert_eq!(marker_function_for_pdb_lookup(), 42);
    }
}
