//! The symbolisation step of every capture (ADR 0022 §4): images and distinct code addresses in, `NativeSymbol`
//! events and notes for the user out.

use crate::paths::{device_map, map_device_path};
use imt_plugin_profiling::nettrace::ext::{ImageLoad, NativeSymbol};
use imt_symbols::{resolve_all, Module, ModuleSymbols, Resolver, SymbolConfig};
use std::path::PathBuf;

/// Module names listed in a note before "and N more".
const NOTE_NAMES: usize = 5;

/// Images as modules DbgHelp can open (kernel device paths mapped to drive paths; unmappable ones dropped).
pub fn modules(images: &[ImageLoad], devices: &[(String, String)], system_root: Option<&str>) -> Vec<Module> {
    images
        .iter()
        .filter_map(|i| {
            Some(Module {
                path: PathBuf::from(map_device_path(&i.path, devices, system_root)?),
                base: i.base,
                size: i.size,
            })
        })
        .collect()
}

/// Names `addresses` with `resolver` (at most `max`); returns the symbol events and notes.
pub fn with_resolver(resolver: &mut dyn Resolver, modules: &[Module], addresses: &[u64], max: usize) -> (Vec<NativeSymbol>, Vec<String>) {
    let r = resolve_all(resolver, modules, addresses, max);
    let mut symbols: Vec<NativeSymbol> = r
        .symbols
        .into_iter()
        .map(|(address, s)| NativeSymbol {
            address,
            start: s.start,
            line: s.line.unwrap_or(0),
            function: s.function,
            file: s.file.unwrap_or_default(),
        })
        .collect();
    symbols.sort_by_key(|s| s.address);
    let mut notes = Vec::new();
    let without: Vec<&str> = r
        .modules
        .iter()
        .filter(|(_, s)| !matches!(s, ModuleSymbols::Pdb))
        .map(|(n, _)| n.as_str())
        .collect();
    let pdbs = r.modules.len() - without.len();
    if !r.modules.is_empty() {
        let mut note = format!("Symbols: {pdbs} of {} modules had a PDB", r.modules.len());
        if !without.is_empty() {
            let shown: Vec<&str> = without.iter().copied().take(NOTE_NAMES).collect();
            let more = without.len().saturating_sub(NOTE_NAMES);
            note.push_str(&format!(
                "; without one (exports or addresses only): {}{}",
                shown.join(", "),
                if more > 0 { format!(" and {more} more") } else { String::new() }
            ));
            note.push_str(
                ". Add their PDB folders to the symbol path, or enable downloads from the Microsoft symbol server for Windows modules",
            );
        }
        notes.push(note);
    }
    (symbols, notes)
}

/// Names `addresses` of `images` with this platform's resolver for `config`.
pub fn symbolize(config: &SymbolConfig, images: &[ImageLoad], addresses: &[u64], max: usize) -> (Vec<NativeSymbol>, Vec<String>) {
    if addresses.is_empty() || images.is_empty() {
        return (Vec::new(), Vec::new());
    }
    let system_root = std::env::var("SystemRoot").ok();
    let modules = modules(images, &device_map(), system_root.as_deref());
    match imt_symbols::resolver(config) {
        Ok(mut r) => with_resolver(r.as_mut(), &modules, addresses, max),
        Err(e) => (Vec::new(), vec![format!("Native code is shown as module+offset: {e}")]),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_symbols::Symbol;

    struct Fake;
    impl Resolver for Fake {
        fn add_module(&mut self, m: &Module) -> ModuleSymbols {
            match m.name().as_str() {
                "busy.exe" => ModuleSymbols::Pdb,
                "ntdll.dll" => ModuleSymbols::Exports,
                _ => ModuleSymbols::None { why: "none".into() },
            }
        }
        fn resolve(&mut self, a: u64) -> Option<Symbol> {
            (a < 0x2000).then(|| Symbol {
                function: "busy::spin".into(),
                start: 0x1000,
                module: "busy.exe".into(),
                file: Some("busy.rs".into()),
                line: Some(3),
            })
        }
    }

    fn image(base: u64, path: &str) -> ImageLoad {
        ImageLoad {
            base,
            size: 0x1000,
            path: path.into(),
        }
    }

    #[test]
    fn names_addresses_and_says_which_modules_lacked_pdbs() {
        let devices = vec![(r"\Device\HarddiskVolume3".to_string(), "C:".to_string())];
        let images = [
            image(0x1000, r"\Device\HarddiskVolume3\app\busy.exe"),
            image(0x2000, r"\SystemRoot\System32\ntdll.dll"),
            image(0x3000, r"C:\x\other.dll"),
            image(0x4000, r"\Device\Mup\unmapped.dll"),
        ];
        let mods = modules(&images, &devices, Some(r"C:\Windows"));
        assert_eq!(mods.len(), 3);
        assert_eq!(mods[0].path, PathBuf::from(r"C:\app\busy.exe"));
        let (syms, notes) = with_resolver(&mut Fake, &mods, &[0x1010, 0x2010, 0x3010], 10);
        assert_eq!(syms.len(), 1);
        assert_eq!((syms[0].address, syms[0].line, syms[0].file.as_str()), (0x1010, 3, "busy.rs"));
        assert!(notes[0].starts_with("Symbols: 1 of 3 modules had a PDB"), "{notes:?}");
        assert!(notes[0].contains("ntdll.dll, other.dll"), "{notes:?}");
        // Nothing to name: no work, no notes.
        assert_eq!(symbolize(&SymbolConfig::default(), &[], &[1], 10), (Vec::new(), Vec::new()));
        let (_, none) = with_resolver(&mut Fake, &[], &[], 10);
        assert!(none.is_empty());
    }
}
