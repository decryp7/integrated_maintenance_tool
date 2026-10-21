//! The two readers end to end, as the plugin runs them: read the file (by ranges), then name frames from the symbol
//! folders. Pure (no plugin contract), so tests, samples and fuzz targets call them directly.

use crate::source::Source;
use imt_dump::model::Document;
use imt_dump::ImportOptions;
use std::path::PathBuf;

/// Reads a minidump and names its frames from the symbol folders.
pub fn import_dump(src: &dyn Source, options: &ImportOptions, folders: &[PathBuf]) -> imt_dump::Result<Document> {
    let mut dump = crate::dump_reader::read_dump(src, &options.limits)?;
    let symbols = crate::pdb_symbols::PdbSymbols::new(folders.iter().cloned());
    if !symbols.is_empty() {
        imt_dump::symbols::symbolise_dump(&mut dump, &symbols, options.limits.max_text);
        for p in symbols
            .problems()
            .into_iter()
            .take(options.limits.max_notes.saturating_sub(dump.notes.len()))
        {
            dump.notes.push(imt_dump::limits::clip(&p, options.limits.max_text));
        }
    }
    Ok(Document::Dump(dump))
}

/// Reads a UMDH log and names frames without symbols from the symbol folders.
pub fn import_umdh(src: &dyn Source, options: &ImportOptions, folders: &[PathBuf]) -> imt_dump::Result<Document> {
    let mut log = crate::umdh_reader::read_umdh(src, &options.limits, options.file_time_ms)?;
    let symbols = crate::pdb_symbols::PdbSymbols::new(folders.iter().cloned());
    if !symbols.is_empty() {
        imt_dump::symbols::symbolise_umdh(&mut log, &symbols, options.limits.max_text);
    }
    Ok(Document::Umdh(log))
}
