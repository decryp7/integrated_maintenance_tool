//! The DbgHelp [`Resolver`]: modules are loaded at their address from their image files (no process handle is
//! needed, so it also serves `.etl` files and dumps), PDBs come from the search path, the cache or the server.

use crate::id::PdbKey;
use crate::{dbghelp_lock, pe, Module, ModuleSymbols, Resolver, Symbol, SymbolConfig, SymbolError};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::HANDLE;
use windows::Win32::System::Diagnostics::Debug::*;

/// Longest symbol name read.
const MAX_NAME: usize = 1024;
/// Longest source file name read.
const MAX_FILE: usize = 1024;

/// DbgHelp sessions without a process are keyed by a unique value that is not a real handle.
static NEXT_SESSION: AtomicUsize = AtomicUsize::new(0x5a5a_0000);

pub(crate) fn fake_handle() -> HANDLE {
    HANDLE(NEXT_SESSION.fetch_add(4, Ordering::Relaxed) as *mut core::ffi::c_void)
}

/// DbgHelp's options for Debrief: undecorated names, line numbers, no dialogs, no deferred loads.
pub(crate) fn set_options() {
    // SAFETY: a process-wide option word; called under the DbgHelp lock.
    unsafe {
        SymSetOptions(SYMOPT_UNDNAME | SYMOPT_LOAD_LINES | SYMOPT_FAIL_CRITICAL_ERRORS | SYMOPT_NO_PROMPTS);
    }
}

fn last_error() -> String {
    std::io::Error::last_os_error().to_string()
}

/// A NUL-terminated wide string at `p` (at most `max` characters).
///
/// # Safety
/// `p` must be null or point at a NUL-terminated UTF-16 string.
unsafe fn wide_at(p: *const u16, max: usize) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut units = Vec::new();
    for i in 0..max {
        // SAFETY: the caller promises a NUL-terminated string; we stop at the NUL.
        let u = unsafe { *p.add(i) };
        if u == 0 {
            break;
        }
        units.push(u);
    }
    String::from_utf16_lossy(&units)
}

pub(crate) struct DbgHelpResolver {
    handle: HANDLE,
    config: SymbolConfig,
    #[cfg(feature = "server")]
    fetcher: Option<crate::fetch::HttpFetcher>,
    /// Loaded modules: base, size, file name, what their symbols came from.
    modules: Vec<(u64, u64, String, ModuleSymbols)>,
}

// SAFETY: the handle is an opaque key; every DbgHelp call is serialised by the process-wide lock.
unsafe impl Send for DbgHelpResolver {}

impl DbgHelpResolver {
    pub(crate) fn new(config: &SymbolConfig) -> Result<DbgHelpResolver, SymbolError> {
        #[cfg(feature = "server")]
        let fetcher = match &config.server {
            Some(s) if config.cache.is_some() => Some(crate::fetch::HttpFetcher::new(s.clone())?),
            _ => None,
        };
        let handle = fake_handle();
        let path = HSTRING::from(config.search_path(&[]));
        let _g = dbghelp_lock();
        set_options();
        // SAFETY: a fresh session key, a valid search path; cleaned up in Drop.
        unsafe { SymInitializeW(handle, PCWSTR(path.as_ptr()), false) }.map_err(|e| SymbolError::DbgHelp {
            call: "SymInitializeW",
            message: e.message(),
        })?;
        Ok(DbgHelpResolver {
            handle,
            config: config.clone(),
            #[cfg(feature = "server")]
            fetcher,
            modules: Vec::new(),
        })
    }

    /// The cache folder holding the module's PDB (downloading it first when a server is enabled).
    fn cached_pdb_dir(&self, m: &Module, notes: &mut Vec<String>) -> Option<PathBuf> {
        let cache = self.config.cache.as_ref()?;
        let id = pe::parse_file(&m.path).ok()?.pdb?;
        let key = PdbKey::from_pdb_id(&id).ok()?;
        let file = key.store_file(cache);
        if file.is_file() {
            return file.parent().map(PathBuf::from);
        }
        #[cfg(feature = "server")]
        if let Some(f) = &self.fetcher {
            use crate::fetch::Fetcher;
            match f.fetch(&key, cache) {
                Ok(p) => return p.parent().map(PathBuf::from),
                Err(e) => notes.push(e.to_string()),
            }
        }
        let _ = notes;
        None
    }
}

impl Resolver for DbgHelpResolver {
    fn add_module(&mut self, m: &Module) -> ModuleSymbols {
        if let Some((.., known)) = self.modules.iter().find(|(b, ..)| *b == m.base) {
            return known.clone();
        }
        let mut notes = Vec::new();
        let extra: Vec<PathBuf> = self.cached_pdb_dir(m, &mut notes).into_iter().collect();
        let path = HSTRING::from(m.path.as_os_str());
        let search = HSTRING::from(self.config.search_path(&extra));
        let size = u32::try_from(m.size).unwrap_or(u32::MAX);
        let _g = dbghelp_lock();
        // SAFETY: our session; the strings outlive the calls.
        let base = unsafe {
            let _ = SymSetSearchPathW(self.handle, PCWSTR(search.as_ptr()));
            SymLoadModuleExW(self.handle, None, PCWSTR(path.as_ptr()), PCWSTR::null(), m.base, size, None, None)
        };
        if base == 0 {
            let why = format!("{} could not be loaded: {}", m.name(), last_error());
            return ModuleSymbols::None { why };
        }
        let mut info = IMAGEHLP_MODULEW64 {
            SizeOfStruct: std::mem::size_of::<IMAGEHLP_MODULEW64>() as u32,
            ..Default::default()
        };
        // SAFETY: a sized out struct.
        let ok = unsafe { SymGetModuleInfoW64(self.handle, m.base, &mut info) }.is_ok();
        let status = match info.SymType {
            _ if !ok => ModuleSymbols::None {
                why: format!("no symbol information for {}", m.name()),
            },
            t if t == SymPdb => ModuleSymbols::Pdb,
            t if t == SymExport => ModuleSymbols::Exports,
            _ => ModuleSymbols::None {
                why: if notes.is_empty() {
                    format!("no PDB found for {} on the symbol path", m.name())
                } else {
                    format!("no PDB found for {} ({})", m.name(), notes.join("; "))
                },
            },
        };
        self.modules.push((m.base, m.size, m.name(), status.clone()));
        status
    }

    fn resolve(&mut self, address: u64) -> Option<Symbol> {
        let module = self
            .modules
            .iter()
            .find(|(b, s, ..)| address >= *b && address - *b < *s)
            .map(|(_, _, n, _)| n.clone())?;
        // SYMBOL_INFOW followed by room for the name, 8-byte aligned.
        let size = std::mem::size_of::<SYMBOL_INFOW>() + MAX_NAME * 2;
        let mut buf = vec![0u64; size.div_ceil(8)];
        let info = buf.as_mut_ptr() as *mut SYMBOL_INFOW;
        let _g = dbghelp_lock();
        // SAFETY: the buffer is large enough for the struct plus MAX_NAME characters and zeroed.
        unsafe {
            (*info).SizeOfStruct = std::mem::size_of::<SYMBOL_INFOW>() as u32;
            (*info).MaxNameLen = MAX_NAME as u32;
        }
        let mut displacement = 0u64;
        // SAFETY: our session and the buffer above.
        unsafe { SymFromAddrW(self.handle, address, Some(&mut displacement), info) }.ok()?;
        // SAFETY: SymFromAddrW filled the struct and NameLen characters of the name (capped by MaxNameLen).
        let (function, start) = unsafe {
            let len = ((*info).NameLen as usize).min(MAX_NAME);
            let name = std::slice::from_raw_parts(std::ptr::addr_of!((*info).Name) as *const u16, len);
            let start = if (*info).Address != 0 {
                (*info).Address
            } else {
                address.saturating_sub(displacement)
            };
            (String::from_utf16_lossy(name), start)
        };
        if function.is_empty() {
            return None;
        }
        let mut line = IMAGEHLP_LINEW64 {
            SizeOfStruct: std::mem::size_of::<IMAGEHLP_LINEW64>() as u32,
            ..Default::default()
        };
        let mut d = 0u32;
        // SAFETY: a sized out struct; FileName points into DbgHelp's memory until the next call (read at once).
        let (file, number) = match unsafe { SymGetLineFromAddrW64(self.handle, start, &mut d, &mut line) } {
            Ok(()) => (
                Some(unsafe { wide_at(line.FileName.0, MAX_FILE) }).filter(|f| !f.is_empty()),
                Some(line.LineNumber),
            ),
            Err(_) => (None, None),
        };
        Some(Symbol {
            function,
            start,
            module,
            file,
            line: number.filter(|&n| n > 0),
        })
    }
}

impl Drop for DbgHelpResolver {
    fn drop(&mut self) {
        let _g = dbghelp_lock();
        // SAFETY: our session.
        unsafe {
            let _ = SymCleanup(self.handle);
        }
    }
}
