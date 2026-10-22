//! The parser library (ADR 0026): one TOML file per log format, built-in or the user's, in
//! `<config>/parsers`. This module is the shell's adapter: it owns the app-wide [`imt_core::parser::library`]
//! (shared by every window), hands its parsers to each window's store, and exposes the Settings ▸ Parsers
//! commands ([`commands`]). Parsing, matching and validation are the engine's.

pub(crate) mod commands;
pub(crate) mod dto;
#[cfg(test)]
mod tests;

use imt_core::parser::library::Library;
use imt_core::parser::ParserSetup;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

/// The folder of the user's parser files under IMT's config folder.
pub(crate) fn parser_dir(config_dir: Option<&Path>) -> Option<PathBuf> {
    config_dir.map(|d| d.join("parsers"))
}

/// The app's parser library (one for all windows).
pub(crate) struct ParserLibrary {
    lib: Mutex<Library>,
}

impl ParserLibrary {
    /// The library in `dir` (built-in parsers only without a folder). Never fails: broken files are listed.
    pub(crate) fn load(dir: Option<&Path>) -> ParserLibrary {
        ParserLibrary {
            lib: Mutex::new(match dir {
                Some(d) => Library::load(d),
                None => Library::builtin_only(),
            }),
        }
    }

    /// What every window's store reads with.
    pub(crate) fn setup(&self) -> ParserSetup {
        crate::sync::lock(&self.lib).setup()
    }

    /// Runs `f` on the library (reads and changes are serialized).
    pub(crate) fn with<T>(&self, f: impl FnOnce(&mut Library) -> T) -> T {
        f(&mut crate::sync::lock(&self.lib))
    }
}
