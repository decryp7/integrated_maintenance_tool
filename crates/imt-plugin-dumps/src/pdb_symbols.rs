//! Function names from PDB files in local symbol folders: a pure-Rust [`SymbolProvider`] (the `pdb` crate), so
//! dumps symbolise on every platform. Folders are searched in the symbol store layout
//! (`<folder>/<name>.pdb/<GUID><age>/<name>.pdb`, what `symchk` and Visual Studio caches write) and flat
//! (`<folder>/<name>.pdb`). With a module's debug identifier, a PDB whose GUID and age do not match is not used.
//! Procedures (with their length) come from the module streams, public symbols fill the gaps. Nothing is
//! downloaded: symbol servers are `imt-symbols`' job (ADR 0023).

use imt_dump::symbols::{ModuleRef, Symbol, SymbolProvider};
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

/// Reads the functions of the PDB at `path`; with `expected` (the module's `<GUID><age>`), refuses another
/// build's PDB.
pub fn read_table(path: &Path, expected: Option<&str>) -> Result<Table, String> {
    let meta = std::fs::metadata(path).map_err(|e| e.to_string())?;
    if meta.len() > MAX_PDB_BYTES {
        return Err(format!("{} is larger than {} MB", path.display(), MAX_PDB_BYTES >> 20));
    }
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut pdb = pdb::PDB::open(file).map_err(|e| e.to_string())?;
    if let Some(want) = expected {
        let info = pdb.pdb_information().map_err(|e| e.to_string())?;
        let age = pdb.debug_information().map(|d| d.age().unwrap_or(info.age)).unwrap_or(info.age);
        let id = format!("{}{:x}", info.guid.simple(), age);
        if !id.eq_ignore_ascii_case(want) {
            return Err(format!("{} is another build (id {id}, the module wants {want})", path.display()));
        }
    }
    let map = pdb.address_map().map_err(|e| e.to_string())?;
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

/// Where a PDB named `pdb` with identifier `id` may be in `folder`.
pub fn candidates(folder: &Path, pdb: &str, id: Option<&str>) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(id) = id {
        out.push(folder.join(pdb).join(id.to_ascii_uppercase()).join(pdb));
    }
    out.push(folder.join(pdb));
    out
}

/// PDBs from local folders, parsed once per module.
pub struct PdbSymbols {
    folders: Vec<PathBuf>,
    tables: Mutex<HashMap<String, Option<Arc<Table>>>>,
    /// Why PDBs that were found could not be used (for the dump's notes).
    problems: Mutex<Vec<String>>,
}

impl PdbSymbols {
    /// Absolute folders only; others are ignored.
    pub fn new(folders: impl IntoIterator<Item = PathBuf>) -> PdbSymbols {
        PdbSymbols {
            folders: folders.into_iter().filter(|f| f.is_absolute()).collect(),
            tables: Mutex::new(HashMap::new()),
            problems: Mutex::new(Vec::new()),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.folders.is_empty()
    }

    /// Problems met so far (another build's PDB, an unreadable file).
    pub fn problems(&self) -> Vec<String> {
        self.problems.lock().unwrap_or_else(|p| p.into_inner()).clone()
    }

    fn table(&self, module: &ModuleRef) -> Option<Arc<Table>> {
        let pdb = match module.debug_file {
            Some(d) => imt_dump::modules::file_name(d).to_string(),
            None => {
                let name = imt_dump::modules::file_name(module.name);
                let stem = name.rsplit_once('.').map_or(name, |(s, _)| s);
                format!("{stem}.pdb")
            }
        };
        let key = format!("{pdb}|{}", module.debug_id.unwrap_or(""));
        if let Some(t) = self.tables.lock().unwrap_or_else(|p| p.into_inner()).get(&key) {
            return t.clone();
        }
        let mut found = None;
        'search: for folder in &self.folders {
            for path in candidates(folder, &pdb, module.debug_id) {
                if !path.is_file() {
                    continue;
                }
                match read_table(&path, module.debug_id) {
                    Ok(t) => {
                        found = Some(Arc::new(t));
                        break 'search;
                    }
                    Err(e) => {
                        let mut p = self.problems.lock().unwrap_or_else(|p| p.into_inner());
                        if p.len() < 16 {
                            p.push(e);
                        }
                    }
                }
            }
        }
        let mut tables = self.tables.lock().unwrap_or_else(|p| p.into_inner());
        if tables.len() < MAX_TABLES {
            tables.insert(key, found.clone());
        }
        found
    }
}

impl SymbolProvider for PdbSymbols {
    fn lookup(&self, module: &ModuleRef, rva: u64) -> Option<Symbol> {
        if self.folders.is_empty() {
            return None;
        }
        self.table(module)?.lookup(rva)
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

    #[test]
    fn folders_are_searched_in_store_and_flat_layout() {
        let c = candidates(Path::new("/sym"), "app.pdb", Some("abc1"));
        assert_eq!(c, [PathBuf::from("/sym/app.pdb/ABC1/app.pdb"), PathBuf::from("/sym/app.pdb")]);
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("app.pdb"), b"not a pdb").unwrap();
        let s = PdbSymbols::new([dir.path().to_path_buf(), PathBuf::from("relative")]);
        assert!(!s.is_empty());
        let m = ModuleRef {
            name: "C:\\x\\app.exe",
            debug_file: None,
            debug_id: None,
        };
        assert!(s.lookup(&m, 0x1000).is_none());
        assert_eq!(s.problems().len(), 1, "the damaged PDB is reported once");
        assert!(s.lookup(&m, 0x1000).is_none());
        assert_eq!(s.problems().len(), 1, "and cached");
        assert!(PdbSymbols::new(Vec::new()).lookup(&m, 0).is_none());
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
        let table = read_table(&pdb, None).unwrap();
        assert!(!table.is_empty());
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
