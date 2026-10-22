//! The two readers end to end, as the plugin runs them: read the file (by ranges), then name a dump's frames from
//! the symbol folders (by PDB identity). Pure (no plugin contract), so tests, samples and fuzz targets call them
//! directly.

use crate::source::Source;
use debrief_dump::model::Document;
use debrief_dump::ImportOptions;
use std::path::PathBuf;

/// Most folders the host may add ([`ImportOptions::symbol_folders`]).
pub const MAX_HOST_FOLDERS: usize = 4;

/// Reads a minidump and names its frames from the symbol folders (the user's, then the host's), recording each
/// module's symbol status (also when there are no folders: the host downloads what is missing).
pub fn import_dump(src: &dyn Source, options: &ImportOptions, folders: &[PathBuf]) -> debrief_dump::Result<Document> {
    let mut dump = crate::dump_reader::read_dump(src, &options.limits)?;
    let host = options.symbol_folders.iter().take(MAX_HOST_FOLDERS).map(PathBuf::from);
    let symbols = crate::pdb_symbols::PdbSymbols::new(folders.iter().cloned().chain(host));
    debrief_dump::symbols::symbolise_dump(&mut dump, &symbols, options.limits.max_text);
    Ok(Document::Dump(dump))
}

/// Reads a UMDH log. Its frames keep the names UMDH wrote: a log records no PDB identities (GUID, age), and
/// symbols are never matched by a module's name alone.
pub fn import_umdh(src: &dyn Source, options: &ImportOptions) -> debrief_dump::Result<Document> {
    let log = crate::umdh_reader::read_umdh(src, &options.limits, options.file_time_ms)?;
    Ok(Document::Umdh(log))
}
