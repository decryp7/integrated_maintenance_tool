//! The symbol port (ADR 0023, ADR 0036): names for addresses in a module, found **by the module's PDB identity**
//! (name, GUID and age from its CodeView record, [`PdbKey`]), never by a file name alone. The dumps plugin
//! implements it with a pure-Rust PDB reader over the user's symbol folders and IMT's symbol cache, which the host
//! fills from the user's symbol servers. [`symbolise_dump`] names the frames a dump lacks and records each
//! module's [`SymbolStatus`]; [`missing`] and [`mark_missing`] are the host's side of downloads.

use crate::limits::clip;
use crate::model::{Dump, Module, SymbolStatus};
pub use imt_symbols::id::{KeyError, PdbKey};

/// A function containing an address.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Symbol {
    pub function: String,
    /// Bytes from the function's start.
    pub offset: u64,
    pub file: Option<String>,
    pub line: Option<u32>,
}

/// Names for addresses, by PDB identity.
pub trait SymbolProvider: Send + Sync {
    /// Finds the PDB of `pdb` (once; later calls answer from memory): [`SymbolStatus::Loaded`] or why not
    /// (`Missing`, `Mismatch`, `Unreadable`).
    fn load(&self, pdb: &PdbKey) -> SymbolStatus;
    /// The function at `rva` (relative to the module's base) in the module whose PDB is `pdb`.
    fn lookup(&self, pdb: &PdbKey, rva: u64) -> Option<Symbol>;
}

/// No symbols: frames stay `module+offset`, and every PDB is missing.
pub struct NoSymbols;

impl SymbolProvider for NoSymbols {
    fn load(&self, _: &PdbKey) -> SymbolStatus {
        SymbolStatus::Missing
    }

    fn lookup(&self, _: &PdbKey, _: u64) -> Option<Symbol> {
        None
    }
}

/// The PDB identity `m` records.
pub fn pdb_key(m: &Module) -> Result<PdbKey, KeyError> {
    match (&m.debug_file, &m.debug_id) {
        (Some(file), Some(id)) => PdbKey::parse(file, id),
        _ => Err(KeyError::NoPdb),
    }
}

/// Names the dump's frames that have none, and records for every module a frame needed a name from what its
/// symbols are ([`SymbolStatus`]); modules no unnamed frame is in are left as they are.
pub fn symbolise_dump(d: &mut Dump, provider: &dyn SymbolProvider, max_text: usize) {
    let mut needed = vec![false; d.modules.len()];
    for f in d.threads.iter().flat_map(|t| &t.frames) {
        if let (None, Some(mi)) = (&f.function, f.module) {
            if let Some(slot) = needed.get_mut(mi as usize) {
                *slot = true;
            }
        }
    }
    let mut keys: Vec<Option<PdbKey>> = vec![None; d.modules.len()];
    for (i, m) in d.modules.iter_mut().enumerate() {
        if !needed[i] {
            continue;
        }
        m.symbols = match pdb_key(m) {
            Err(_) => SymbolStatus::NoIdentity,
            Ok(k) => {
                let status = provider.load(&k);
                if status.is_loaded() {
                    keys[i] = Some(k);
                }
                clip_status(status, max_text)
            }
        };
    }
    for t in &mut d.threads {
        for f in &mut t.frames {
            if f.function.is_some() {
                continue;
            }
            let Some(mi) = f.module else { continue };
            let (Some(Some(k)), Some(m)) = (keys.get(mi as usize), d.modules.get(mi as usize)) else {
                continue;
            };
            if let Some(s) = provider.lookup(k, f.address.wrapping_sub(m.base)) {
                f.function = Some(clip(&s.function, max_text));
                f.function_offset = Some(s.offset);
                f.file = s.file.map(|x| clip(&x, max_text));
                f.line = s.line;
            }
        }
    }
}

fn clip_status(s: SymbolStatus, max: usize) -> SymbolStatus {
    match s {
        SymbolStatus::Mismatch { detail } => SymbolStatus::Mismatch {
            detail: clip(&detail, max),
        },
        SymbolStatus::Unreadable { detail } => SymbolStatus::Unreadable {
            detail: clip(&detail, max),
        },
        SymbolStatus::DownloadFailed { detail } => SymbolStatus::DownloadFailed {
            detail: clip(&detail, max),
        },
        other => other,
    }
}

/// The identities of the modules whose PDB is [`SymbolStatus::Missing`] (what a host may download), at most `max`.
pub fn missing(d: &Dump, max: usize) -> Vec<PdbKey> {
    let mut out: Vec<PdbKey> = d
        .modules
        .iter()
        .filter(|m| m.symbols == SymbolStatus::Missing)
        .filter_map(|m| pdb_key(m).ok())
        .collect();
    out.sort();
    out.dedup();
    out.truncate(max);
    out
}

/// Marks the modules whose PDB is `Missing` and is one of `keys` as [`SymbolStatus::Downloading`].
pub fn mark_downloading(d: &mut Dump, keys: &[PdbKey]) {
    for m in d.modules.iter_mut().filter(|m| m.symbols == SymbolStatus::Missing) {
        if pdb_key(m).is_ok_and(|k| keys.contains(&k)) {
            m.symbols = SymbolStatus::Downloading;
        }
    }
}

/// Sets the status of every module whose PDB is still missing (or being downloaded) to what `why` says about its
/// identity (the outcome of a download, or that downloads are off); modules `why` says nothing about keep their status.
pub fn mark_missing(d: &mut Dump, why: impl Fn(&PdbKey) -> Option<SymbolStatus>, max_text: usize) {
    for m in d
        .modules
        .iter_mut()
        .filter(|m| matches!(m.symbols, SymbolStatus::Missing | SymbolStatus::Downloading))
    {
        if let Some(s) = pdb_key(m).ok().and_then(|k| why(&k)) {
            m.symbols = clip_status(s, max_text);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testing::{sample_dump, SHOP_BASE};

    struct Table;

    impl SymbolProvider for Table {
        fn load(&self, pdb: &PdbKey) -> SymbolStatus {
            match pdb.name().as_str() {
                "Shop.Orders.pdb" => SymbolStatus::Loaded,
                "ntdll.pdb" => SymbolStatus::Mismatch {
                    detail: "another build ".repeat(20),
                },
                _ => SymbolStatus::Missing,
            }
        }

        fn lookup(&self, pdb: &PdbKey, rva: u64) -> Option<Symbol> {
            (pdb.name().as_str() == "Shop.Orders.pdb" && rva >= 0x1000).then(|| Symbol {
                function: "Shop::Found".into(),
                offset: rva - 0x1000,
                file: Some("found.cpp".into()),
                line: Some(7),
            })
        }
    }

    fn unnamed(d: &mut Dump, thread: usize, frame: usize, module: usize, rva: u64) {
        let base = d.modules[module].base;
        let f = &mut d.threads[thread].frames[frame];
        f.function = None;
        f.module = Some(module as u32);
        f.address = base + rva;
    }

    #[test]
    fn frames_without_names_are_symbolised_by_identity() {
        let mut d = sample_dump();
        unnamed(&mut d, 0, 2, 3, 0x1234);
        assert_eq!(d.modules[3].base, SHOP_BASE);
        symbolise_dump(&mut d, &NoSymbols, 100);
        assert!(d.threads[0].frames[2].function.is_none());
        assert_eq!(d.modules[3].symbols, SymbolStatus::Missing);
        symbolise_dump(&mut d, &Table, 100);
        let f = &d.threads[0].frames[2];
        assert_eq!(
            (f.function.as_deref(), f.function_offset, f.line),
            (Some("Shop::Found"), Some(0x234), Some(7))
        );
        assert_eq!(d.modules[3].symbols, SymbolStatus::Loaded);
        // Named frames are left alone, and so are modules no unnamed frame is in.
        assert_eq!(d.threads[1].frames[3].function.as_deref(), Some("Ledger::Post"));
        assert_eq!(d.modules[0].symbols, SymbolStatus::Loaded);
    }

    #[test]
    fn modules_without_an_identity_or_with_another_build_are_not_named() {
        let mut d = sample_dump();
        let ntdll = d.modules.iter().position(|m| m.name() == "ntdll.dll").unwrap();
        unnamed(&mut d, 0, 0, ntdll, 0x10);
        unnamed(&mut d, 0, 2, 3, 0x1234);
        // The module records its PDB's name but no GUID and age: never looked up by name.
        d.modules[3].debug_id = None;
        symbolise_dump(&mut d, &Table, 30);
        assert_eq!(d.modules[3].symbols, SymbolStatus::NoIdentity);
        assert!(d.threads[0].frames[2].function.is_none());
        let SymbolStatus::Mismatch { detail } = &d.modules[ntdll].symbols else {
            panic!("{:?}", d.modules[ntdll].symbols)
        };
        assert!(detail.chars().count() <= 30, "clipped: {detail}");
        assert!(d.threads[0].frames[0].function.is_none());
        assert_eq!(pdb_key(&d.modules[3]), Err(KeyError::NoPdb));
    }

    #[test]
    fn missing_pdbs_are_listed_once_and_marked_with_the_download_outcome() {
        let mut d = sample_dump();
        for m in &mut d.modules {
            m.symbols = SymbolStatus::Missing;
        }
        d.modules[0].debug_id = Some("not an id".into());
        let keys = missing(&d, 100);
        assert_eq!(keys.len(), d.modules.len() - 1);
        assert_eq!(missing(&d, 1).len(), 1);
        let shop = pdb_key(&d.modules[3]).unwrap();
        mark_missing(
            &mut d,
            |k| (k == &shop).then(|| SymbolStatus::DownloadFailed { detail: "x".repeat(50) }),
            10,
        );
        assert_eq!(
            d.modules[3].symbols,
            SymbolStatus::DownloadFailed {
                detail: "x".repeat(9) + "…"
            }
        );
        assert_eq!(d.modules[1].symbols, SymbolStatus::Missing);
        mark_missing(&mut d, |_| Some(SymbolStatus::ServersDisabled), 10);
        assert_eq!(d.modules[1].symbols, SymbolStatus::ServersDisabled);
        assert_eq!(d.modules[0].symbols, SymbolStatus::Missing, "no identity to download by");
        assert_eq!(d.modules[3].symbols.detail(), Some("xxxxxxxxx…"));
    }

    #[test]
    fn modules_being_downloaded_are_marked_and_then_get_their_outcome() {
        let mut d = sample_dump();
        for m in &mut d.modules {
            m.symbols = SymbolStatus::Missing;
        }
        let keys = missing(&d, 10);
        let (first, rest) = (keys[0].clone(), &keys[1..]);
        mark_downloading(&mut d, std::slice::from_ref(&first));
        let downloading = |d: &Dump| d.modules.iter().filter(|m| m.symbols == SymbolStatus::Downloading).count();
        assert_eq!(downloading(&d), 1);
        assert!(!rest.is_empty());
        // A module that is not being downloaded keeps its status; one being downloaded gets the outcome.
        mark_missing(&mut d, |k| (k == &first).then_some(SymbolStatus::NotOnServer), 100);
        assert_eq!(downloading(&d), 0);
        assert!(d.modules.iter().any(|m| m.symbols == SymbolStatus::NotOnServer));
        // Nothing to say (cancelled): it goes back to missing.
        mark_downloading(&mut d, &keys);
        assert!(downloading(&d) > 0);
        mark_missing(&mut d, |_| Some(SymbolStatus::Missing), 100);
        assert_eq!(downloading(&d), 0);
    }
}
