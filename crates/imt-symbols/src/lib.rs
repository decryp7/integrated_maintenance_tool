//! Native symbols for IMT (ADR 0022), shared by the native profiler and dump analysis.
//!
//! - [`pe`]: a PE image's machine, size and PDB identity (pure; fuzzed).
//! - [`SymbolConfig`]: where PDBs are looked for (the user's search path, the symstore-layout cache) and whether
//!   a symbol server may be asked ([`ServerConfig`], off unless the user enables it).
//! - [`fetch`]: downloading a PDB from a symbol server into the cache (feature `server`).
//! - [`Resolver`]: modules in, addresses → [`Symbol`]s out; [`resolver`] gives the DbgHelp one on Windows and
//!   [`SymbolError::Unavailable`] elsewhere.
//! - [`walk`] (Windows): DbgHelp `StackWalkEx` over a [`walk::Memory`] (a live process, a snapshot, a dump).
//!
//! DbgHelp is single-threaded: every call this crate makes runs under one process-wide lock, which other DbgHelp
//! users in the same process take through [`with_dbghelp`].

pub mod config;
pub mod pe;

#[cfg(feature = "server")]
pub mod fetch;

#[cfg(windows)]
mod dbghelp;
#[cfg(windows)]
pub mod walk;

pub use config::{parse_search_path, ServerConfig, SymbolConfig};

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Mutex, MutexGuard};

/// A module mapped into a process: `path` on disk at `base..base + size`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Module {
    pub path: PathBuf,
    pub base: u64,
    pub size: u64,
}

impl Module {
    /// The file name (`busy.exe`).
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| self.path.to_string_lossy().into_owned())
    }

    pub fn contains(&self, address: u64) -> bool {
        address >= self.base && address - self.base < self.size
    }
}

/// What an address is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    /// The function (undecorated).
    pub function: String,
    /// The function's first address.
    pub start: u64,
    /// The module's file name.
    pub module: String,
    /// The function's source file and first line, when the PDB has line information.
    pub file: Option<String>,
    pub line: Option<u32>,
}

/// What a module's symbols came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModuleSymbols {
    /// A matching PDB (full names, source lines).
    Pdb,
    /// Only the module's exports (no PDB found).
    Exports,
    /// Nothing; `why` says what was missing.
    None { why: String },
}

/// Why symbols cannot be resolved here.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum SymbolError {
    #[error("native symbols need Windows (DbgHelp)")]
    Unavailable,
    #[error("invalid symbol path: {0}")]
    InvalidPath(String),
    #[error("{call} failed: {message}")]
    DbgHelp { call: &'static str, message: String },
}

/// Turns code addresses into symbols. Implementations never panic on any address.
pub trait Resolver: Send {
    /// Makes `module`'s symbols available (finding its PDB on the search path, in the cache or, when enabled, on
    /// the symbol server).
    fn add_module(&mut self, module: &Module) -> ModuleSymbols;
    /// The symbol of `address`, if a loaded module has one.
    fn resolve(&mut self, address: u64) -> Option<Symbol>;
}

/// The resolver of this platform: DbgHelp on Windows.
pub fn resolver(config: &SymbolConfig) -> Result<Box<dyn Resolver>, SymbolError> {
    #[cfg(windows)]
    {
        Ok(Box::new(dbghelp::DbgHelpResolver::new(config)?))
    }
    #[cfg(not(windows))]
    {
        let _ = config;
        Err(SymbolError::Unavailable)
    }
}

/// The result of [`resolve_all`].
#[derive(Debug, Default)]
pub struct Resolved {
    pub symbols: HashMap<u64, Symbol>,
    /// Per module (file name): what its symbols came from.
    pub modules: Vec<(String, ModuleSymbols)>,
    /// Addresses no loaded module named.
    pub unresolved: usize,
}

/// Adds the modules that contain any of `addresses` and resolves at most `max` addresses.
pub fn resolve_all(r: &mut dyn Resolver, modules: &[Module], addresses: &[u64], max: usize) -> Resolved {
    let mut out = Resolved::default();
    let used: Vec<&Module> = modules.iter().filter(|m| addresses.iter().any(|&a| m.contains(a))).collect();
    for m in used {
        out.modules.push((m.name(), r.add_module(m)));
    }
    for &a in addresses.iter().take(max) {
        match r.resolve(a) {
            Some(s) => {
                out.symbols.insert(a, s);
            }
            None => out.unresolved += 1,
        }
    }
    out.unresolved += addresses.len().saturating_sub(max);
    out
}

static DBGHELP: Mutex<()> = Mutex::new(());

/// The process-wide DbgHelp lock (poisoning is recovered: the lock guards no data).
fn dbghelp_lock() -> MutexGuard<'static, ()> {
    DBGHELP.lock().unwrap_or_else(|p| p.into_inner())
}

/// Runs `f` while holding the process-wide DbgHelp lock: call DbgHelp functions only inside it.
pub fn with_dbghelp<R>(f: impl FnOnce() -> R) -> R {
    let _g = dbghelp_lock();
    f()
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Fake;
    impl Resolver for Fake {
        fn add_module(&mut self, m: &Module) -> ModuleSymbols {
            if m.name() == "a.dll" {
                ModuleSymbols::Pdb
            } else {
                ModuleSymbols::None { why: "no pdb".into() }
            }
        }
        fn resolve(&mut self, address: u64) -> Option<Symbol> {
            (address < 0x2000).then(|| Symbol {
                function: format!("f{address:x}"),
                start: address,
                module: "a.dll".into(),
                file: None,
                line: None,
            })
        }
    }

    #[test]
    fn resolve_all_adds_used_modules_and_bounds_the_work() {
        let modules = [
            Module {
                path: PathBuf::from(r"C:\x\a.dll"),
                base: 0x1000,
                size: 0x1000,
            },
            Module {
                path: PathBuf::from(r"C:\x\b.dll"),
                base: 0x2000,
                size: 0x1000,
            },
            Module {
                path: PathBuf::from(r"C:\x\unused.dll"),
                base: 0x9000,
                size: 0x10,
            },
        ];
        let r = resolve_all(&mut Fake, &modules, &[0x1010, 0x2010, 0x1020], 2);
        assert_eq!(r.modules.len(), 2, "{:?}", r.modules);
        assert_eq!(r.symbols.len(), 1);
        assert_eq!(r.unresolved, 2, "one unnamed, one beyond the limit");
        assert!(modules[0].contains(0x1fff) && !modules[0].contains(0x2000));
        assert_eq!(with_dbghelp(|| 7), 7);
        if !cfg!(windows) {
            assert!(matches!(resolver(&SymbolConfig::default()), Err(SymbolError::Unavailable)));
        }
    }
}
