//! A dump opened without symbols and named again when its PDBs arrive equals the dump read with those symbols
//! present: only the unnamed frames of the modules that gained symbols are looked at.

use debrief_dump::model::{Document, Dump, SymbolStatus};
use debrief_dump::symbols::{pdb_key, symbolise_dump, NoSymbols, PdbKey, Symbol, SymbolProvider};
use debrief_dump::{ImportOptions, Limits};
use debrief_plugin_dumps::import::{import_dump, resymbolise_dump};
use debrief_plugin_dumps::{dump_reader, synth};
use std::collections::HashSet;
use std::path::PathBuf;

/// PDBs "on disk": the ones named in `have` load and name every address `<function>_<rva>`.
struct Have(HashSet<String>);

impl SymbolProvider for Have {
    fn load(&self, pdb: &PdbKey) -> SymbolStatus {
        if self.0.contains(pdb.name().as_str()) {
            SymbolStatus::Loaded
        } else {
            SymbolStatus::Missing
        }
    }

    fn lookup(&self, pdb: &PdbKey, rva: u64) -> Option<Symbol> {
        self.0.contains(pdb.name().as_str()).then(|| Symbol {
            function: format!("{}_{:x}", pdb.name().as_str(), rva & !0xff),
            offset: rva & 0xff,
            file: Some("synthetic.cpp".into()),
            line: Some(1 + (rva & 7) as u32),
        })
    }
}

fn read() -> Dump {
    dump_reader::read_dump(&synth::sample_crash(), &Limits::default()).expect("the synthetic crash dump reads")
}

fn pdbs(d: &Dump) -> Vec<String> {
    let mut v: Vec<String> = d
        .modules
        .iter()
        .filter_map(|m| pdb_key(m).ok())
        .map(|k| k.name().as_str().to_string())
        .collect();
    v.sort();
    v.dedup();
    v
}

fn unnamed(d: &Dump) -> usize {
    d.threads
        .iter()
        .flat_map(|t| &t.frames)
        .filter(|f| f.function.is_none() && f.module.is_some())
        .count()
}

#[test]
fn naming_again_after_symbols_arrive_equals_reading_with_them() {
    let max = Limits::default().max_text;
    let names = pdbs(&read());
    assert!(names.len() >= 2, "the fixture has modules with identities: {names:?}");
    // The PDBs of all modules but the first arrive in two steps.
    let first: HashSet<String> = names.iter().skip(1).take(1).cloned().collect();
    let all: HashSet<String> = names.iter().skip(1).cloned().collect();

    let mut full = read();
    symbolise_dump(&mut full, &Have(all.clone()), max);

    let mut incremental = read();
    symbolise_dump(&mut incremental, &NoSymbols, max);
    let before = unnamed(&incremental);
    assert!(before > 0, "frames wait for symbols");
    symbolise_dump(&mut incremental, &Have(first), max);
    assert!(unnamed(&incremental) < before, "the first PDB named frames");
    symbolise_dump(&mut incremental, &Have(all), max);

    assert!(unnamed(&full) < before);
    assert_eq!(incremental, full);
}

#[test]
fn the_plugin_names_an_open_document_like_a_new_import() {
    let bytes = synth::sample_crash();
    let options = ImportOptions::default();
    let none: Vec<PathBuf> = Vec::new();
    let opened = import_dump(&bytes, &options, &none).expect("import");
    let again = resymbolise_dump(opened.clone(), &options, &none);
    assert_eq!(again, opened);
    assert_eq!(import_dump(&bytes, &options, &none).expect("import"), again);
    // A document that is not a dump is returned as it is.
    let Document::Dump(d) = opened else { panic!("a dump") };
    assert!(matches!(resymbolise_dump(Document::Dump(d), &options, &none), Document::Dump(_)));
}
