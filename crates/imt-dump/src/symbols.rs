//! The symbol port (ADR 0023): names for addresses in a module. The dumps plugin implements it with a pure-Rust
//! PDB reader over the user's symbol folders; `imt-symbols` (DbgHelp, symbol servers) can implement the same trait
//! later without touching the readers. [`symbolise_dump`] and [`symbolise_umdh`] fill in what a file lacks.

use crate::limits::clip;
use crate::model::{Dump, UmdhLog};

/// A module as symbol lookups identify it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ModuleRef<'a> {
    /// The code file's name (`ntdll.dll`; UMDH logs often name it without extension).
    pub name: &'a str,
    /// The PDB's name, when the module records it.
    pub debug_file: Option<&'a str>,
    /// The PDB's GUID and age as symbol stores spell them, when known. Without it a lookup cannot verify that
    /// the PDB matches the module.
    pub debug_id: Option<&'a str>,
}

/// A function containing an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub function: String,
    /// Bytes from the function's start.
    pub offset: u64,
    pub file: Option<String>,
    pub line: Option<u32>,
}

/// Names for addresses.
pub trait SymbolProvider: Send + Sync {
    /// The function at `rva` (relative to the module's base) in `module`.
    fn lookup(&self, module: &ModuleRef, rva: u64) -> Option<Symbol>;
}

/// No symbols: frames stay `module+offset`.
pub struct NoSymbols;

impl SymbolProvider for NoSymbols {
    fn lookup(&self, _: &ModuleRef, _: u64) -> Option<Symbol> {
        None
    }
}

/// Names the dump's frames that have none; marks the modules symbols were found for.
pub fn symbolise_dump(d: &mut Dump, provider: &dyn SymbolProvider, max_text: usize) {
    let mut found = vec![false; d.modules.len()];
    for t in &mut d.threads {
        for f in &mut t.frames {
            if f.function.is_some() {
                continue;
            }
            let Some(mi) = f.module else { continue };
            let Some(m) = d.modules.get(mi as usize) else { continue };
            let module = ModuleRef {
                name: m.name(),
                debug_file: m.debug_file.as_deref(),
                debug_id: m.debug_id.as_deref(),
            };
            if let Some(s) = provider.lookup(&module, f.address.wrapping_sub(m.base)) {
                f.function = Some(clip(&s.function, max_text));
                f.function_offset = Some(s.offset);
                f.file = s.file.map(|x| clip(&x, max_text));
                f.line = s.line;
                if let Some(slot) = found.get_mut(mi as usize) {
                    *slot = true;
                }
            }
        }
    }
    for (m, f) in d.modules.iter_mut().zip(found) {
        m.symbols |= f;
    }
}

/// Names the log's frames that have a module and an offset from its base but no function.
pub fn symbolise_umdh(u: &mut UmdhLog, provider: &dyn SymbolProvider, max_text: usize) {
    for s in &mut u.stacks {
        for f in &mut s.frames {
            let (Some(module), None, Some(rva)) = (&f.module, &f.function, f.offset) else {
                continue;
            };
            let m = ModuleRef {
                name: module,
                debug_file: None,
                debug_id: None,
            };
            if let Some(sym) = provider.lookup(&m, rva) {
                f.function = Some(clip(&sym.function, max_text));
                f.offset = Some(sym.offset);
                f.file = sym.file.map(|x| clip(&x, max_text));
                f.line = sym.line;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::UmdhKind;
    use crate::testing::{sample_dump, sample_umdh, SHOP_BASE};

    struct Table;

    impl SymbolProvider for Table {
        fn lookup(&self, module: &ModuleRef, rva: u64) -> Option<Symbol> {
            (module.name.starts_with("Shop.Orders") && rva >= 0x1000).then(|| Symbol {
                function: "Shop::Found".into(),
                offset: rva - 0x1000,
                file: Some("found.cpp".into()),
                line: Some(7),
            })
        }
    }

    #[test]
    fn frames_without_names_are_symbolised() {
        let mut d = sample_dump();
        d.modules[3].symbols = false;
        d.threads[0].frames[2].function = None;
        d.threads[0].frames[2].address = SHOP_BASE + 0x1234;
        symbolise_dump(&mut d, &NoSymbols, 100);
        assert!(d.threads[0].frames[2].function.is_none());
        symbolise_dump(&mut d, &Table, 100);
        let f = &d.threads[0].frames[2];
        assert_eq!(
            (f.function.as_deref(), f.function_offset, f.line),
            (Some("Shop::Found"), Some(0x234), Some(7))
        );
        assert!(d.modules[3].symbols);
        // Named frames are left alone.
        assert_eq!(d.threads[1].frames[3].function.as_deref(), Some("Ledger::Post"));

        let mut u = sample_umdh(UmdhKind::Snapshot, 0, &[(1, 10, 1)]);
        u.stacks[0].frames[3].function = None;
        u.stacks[0].frames[3].offset = Some(0x1010);
        symbolise_umdh(&mut u, &Table, 100);
        assert_eq!(u.stacks[0].frames[3].label(), "Shop.Orders!Shop::Found+0x10");
    }
}
