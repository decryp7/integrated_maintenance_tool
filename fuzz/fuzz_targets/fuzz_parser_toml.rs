#![no_main]
//! Parser library files (`parsers/*.toml`, user library folders, imported parsers; ADR 0026): `Parser::from_toml`
//! (syntax, schema, limits, compile), an override fragment over a built-in, and the library's import check.
//! What is accepted round-trips through its canonical TOML with the same content hash.
use debrief_core::parser::builtin;
use debrief_core::parser::library::Library;
use debrief_core::parser::Parser;
use libfuzzer_sys::fuzz_target;
use std::sync::OnceLock;

fn library() -> &'static Library {
    static L: OnceLock<Library> = OnceLock::new();
    L.get_or_init(Library::builtin_only)
}

fuzz_target!(|data: &[u8]| {
    let Some((&pick, rest)) = data.split_first() else { return };
    let Ok(text) = std::str::from_utf8(rest) else { return };
    if let Ok(p) = Parser::from_toml(text) {
        let canonical = p.canonical_toml();
        let again = Parser::from_toml(&canonical).expect("the canonical TOML of an accepted parser is accepted");
        assert_eq!(again.reference(), p.reference());
    }
    let parsers = builtin::parsers();
    let _ = parsers[pick as usize % parsers.len()].with_override(text);
    let _ = library().check_import(text);
});
