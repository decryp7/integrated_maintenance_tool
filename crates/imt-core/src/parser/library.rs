//! The parser library on disk (ADR 0026 §2): the built-in parsers plus the user's parser files
//! `<dir>/<id>.toml`, and the index `<dir>/library.json` with the disabled ids. A user file with a built-in's id
//! overrides it. A missing or corrupt index means "all enabled"; a user file that does not parse is listed as
//! broken and never used. Writes go through a temporary file and a rename.

use super::{builtin, Parser, ParserError, ParserSetup, SetupParser};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// The index file in the library folder.
pub const INDEX_FILE: &str = "library.json";
/// The index format this IMT writes.
pub const INDEX_VERSION: u32 = 1;
/// Most user parser files read from the folder.
pub const MAX_USER_PARSERS: usize = 1000;

#[derive(Debug, Default, Serialize, Deserialize)]
#[serde(default)]
struct Index {
    version: u32,
    disabled: Vec<String>,
}

/// Where a parser of the library comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Origin {
    /// Shipped with IMT.
    Builtin,
    /// A user file.
    User,
    /// A user file replacing the built-in parser with the same id.
    Override,
}

/// One parser of the library.
#[derive(Debug, Clone)]
pub struct LibraryParser {
    pub parser: Parser,
    /// The file's text as stored (comments kept): what export writes.
    pub toml: String,
    pub origin: Origin,
    pub enabled: bool,
}

/// A user file that does not parse.
#[derive(Debug, Clone)]
pub struct BrokenFile {
    pub file: String,
    pub error: ParserError,
}

/// Why a library change was refused.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LibraryError {
    #[error("{0}")]
    Invalid(ParserError),
    /// Another user parser already has this id.
    #[error("a parser with the id \"{0}\" is already in the library; confirm to replace it")]
    Conflict(String),
    #[error("no parser \"{0}\" in the library")]
    NotFound(String),
    #[error("\"{0}\" is a built-in parser: it can be disabled, not removed")]
    BuiltIn(String),
    #[error("{0}")]
    Io(String),
}

/// How an imported parser relates to the library.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Relation {
    /// No parser with its id.
    New,
    /// A newer version of a parser in the library ("Update").
    Newer,
    /// The same version (maybe a different text).
    Same,
    /// An older version.
    Older,
}

/// What importing a parser file would do.
#[derive(Debug, Clone)]
pub struct ImportCheck {
    pub parser: Parser,
    pub relation: Relation,
    /// The library's parser with the same id.
    pub existing: Option<LibraryParser>,
}

/// The library: built-in and user parsers, with their enabled flags.
#[derive(Debug, Clone)]
pub struct Library {
    dir: Option<PathBuf>,
    parsers: Vec<LibraryParser>,
    broken: Vec<BrokenFile>,
}

impl Library {
    /// The built-in parsers only (no folder: nothing can be saved).
    pub fn builtin_only() -> Library {
        Self::read(None)
    }

    /// Reads the library in `dir` (created on the first save). Never fails: what cannot be read is logged,
    /// listed as broken, or falls back to the defaults.
    pub fn load(dir: &Path) -> Library {
        Self::read(Some(dir.to_path_buf()))
    }

    fn read(dir: Option<PathBuf>) -> Library {
        let index = dir.as_deref().map(read_index).unwrap_or_default();
        let mut parsers: Vec<LibraryParser> = builtin::FILES
            .iter()
            .zip(builtin::parsers())
            .map(|(f, p)| LibraryParser {
                parser: p.clone(),
                toml: f.toml.to_string(),
                origin: Origin::Builtin,
                enabled: true,
            })
            .collect();
        let mut broken = Vec::new();
        for (file, text) in dir.as_deref().map(user_files).unwrap_or_default() {
            let text = match text {
                Ok(t) => t,
                Err(error) => {
                    broken.push(BrokenFile { file, error });
                    continue;
                }
            };
            match Parser::from_toml(&text) {
                Ok(p) if file_stem(&file) != p.id.as_str() => broken.push(BrokenFile {
                    error: ParserError::new(format!("the file is named {file} but holds the parser \"{}\"", p.id)),
                    file,
                }),
                Ok(p) => {
                    let user = LibraryParser {
                        toml: text,
                        origin: Origin::User,
                        enabled: true,
                        parser: p,
                    };
                    match parsers.iter_mut().find(|b| b.parser.id == user.parser.id) {
                        Some(b) => {
                            *b = LibraryParser {
                                origin: Origin::Override,
                                ..user
                            }
                        }
                        None => parsers.push(user),
                    }
                }
                Err(error) => broken.push(BrokenFile { file, error }),
            }
        }
        for p in &mut parsers {
            p.enabled = !index.disabled.iter().any(|d| d == p.parser.id.as_str());
        }
        Library { dir, parsers, broken }
    }

    /// Every parser: built-ins (or their overrides) in shipped order, then user parsers by id.
    pub fn parsers(&self) -> &[LibraryParser] {
        &self.parsers
    }

    /// User files that do not parse.
    pub fn broken(&self) -> &[BrokenFile] {
        &self.broken
    }

    pub fn get(&self, id: &str) -> Option<&LibraryParser> {
        self.parsers.iter().find(|p| p.parser.id.as_str() == id)
    }

    /// What the engine compiles: every parser with its enabled flag (no overrides; the window adds its own).
    pub fn setup(&self) -> ParserSetup {
        ParserSetup {
            parsers: self
                .parsers
                .iter()
                .map(|p| SetupParser {
                    toml: p.toml.clone(),
                    enabled: p.enabled,
                    user: p.origin != Origin::Builtin,
                })
                .collect(),
            overrides: Default::default(),
        }
    }

    fn dir(&self) -> Result<&Path, LibraryError> {
        self.dir
            .as_deref()
            .ok_or_else(|| LibraryError::Io("there is no parser folder (IMT has no config folder)".into()))
    }

    /// Saves `toml` as a user parser (a built-in's id makes an override). `previous` is the id it was edited
    /// from (a rename removes that file). An existing user parser with the same id is replaced only when it is
    /// `previous` or with `overwrite`. Returns the saved parser's id.
    pub fn save(&mut self, toml: &str, previous: Option<&str>, overwrite: bool) -> Result<String, LibraryError> {
        let parser = Parser::from_toml(toml).map_err(LibraryError::Invalid)?;
        let id = parser.id.to_string();
        let taken = self.get(&id).is_some_and(|p| p.origin != Origin::Builtin);
        if taken && previous != Some(id.as_str()) && !overwrite {
            return Err(LibraryError::Conflict(id));
        }
        let dir = self.dir()?.to_path_buf();
        write_atomic(&dir, &format!("{id}.toml"), toml)?;
        if let Some(old) = previous.filter(|old| *old != id) {
            let was_disabled = self.get(old).is_some_and(|p| !p.enabled);
            if self.get(old).is_some_and(|p| p.origin != Origin::Builtin) {
                remove_file(&dir.join(format!("{old}.toml")))?;
            }
            if was_disabled {
                self.write_enabled(&id, false)?;
            }
        }
        self.reload();
        Ok(id)
    }

    /// Removes a user parser or an override (the built-in comes back).
    pub fn remove(&mut self, id: &str) -> Result<(), LibraryError> {
        match self.get(id).map(|p| p.origin) {
            None => return Err(LibraryError::NotFound(id.into())),
            Some(Origin::Builtin) => return Err(LibraryError::BuiltIn(id.into())),
            Some(Origin::User) => self.write_enabled(id, true)?,
            Some(Origin::Override) => {}
        }
        let dir = self.dir()?.to_path_buf();
        remove_file(&dir.join(format!("{id}.toml")))?;
        self.reload();
        Ok(())
    }

    /// Enables or disables a parser.
    pub fn set_enabled(&mut self, id: &str, enabled: bool) -> Result<(), LibraryError> {
        if self.get(id).is_none() {
            return Err(LibraryError::NotFound(id.into()));
        }
        self.write_enabled(id, enabled)?;
        self.reload();
        Ok(())
    }

    /// Saves a copy of a parser under a new id (`<id>-copy`, `<id>-copy-2`, ...) and name; returns the new id.
    pub fn duplicate(&mut self, id: &str) -> Result<String, LibraryError> {
        let source = self.get(id).ok_or_else(|| LibraryError::NotFound(id.into()))?;
        let new_id = (1..)
            .map(|n| if n == 1 { format!("{id}-copy") } else { format!("{id}-copy-{n}") })
            .find(|c| self.get(c).is_none() && c.len() <= 64)
            .unwrap_or_else(|| format!("copy-{}", self.parsers.len()));
        let name = format!("{} (copy)", source.parser.name);
        let text = replace_top_key(&replace_top_key(&source.toml, "id", &new_id), "name", &name);
        self.save(&text, None, false)
    }

    /// Checks a parser file for import: valid, and how it relates to the library.
    pub fn check_import(&self, toml: &str) -> Result<ImportCheck, ParserError> {
        let parser = Parser::from_toml(toml)?;
        let existing = self.get(parser.id.as_str()).cloned();
        let relation = match &existing {
            None => Relation::New,
            Some(e) => match parser.version.cmp(&e.parser.version) {
                std::cmp::Ordering::Greater => Relation::Newer,
                std::cmp::Ordering::Equal => Relation::Same,
                std::cmp::Ordering::Less => Relation::Older,
            },
        };
        Ok(ImportCheck {
            parser,
            relation,
            existing,
        })
    }

    fn reload(&mut self) {
        *self = Self::read(self.dir.clone());
    }

    fn write_enabled(&self, id: &str, enabled: bool) -> Result<(), LibraryError> {
        let dir = self.dir()?;
        let mut index = read_index(dir);
        index.version = INDEX_VERSION;
        index.disabled.retain(|d| d != id);
        if !enabled {
            index.disabled.push(id.to_string());
            index.disabled.sort();
        }
        let json = serde_json::to_string_pretty(&index).map_err(|e| LibraryError::Io(e.to_string()))?;
        write_atomic(dir, INDEX_FILE, &json)
    }
}

/// The index, or the defaults when it is missing, corrupt or of a newer version (logged).
fn read_index(dir: &Path) -> Index {
    let path = dir.join(INDEX_FILE);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Index::default();
    };
    match serde_json::from_str::<Index>(&text) {
        Ok(i) if i.version <= INDEX_VERSION => i,
        Ok(i) => {
            log::warn!(
                "the parser library index is of version {}; this IMT reads {INDEX_VERSION}: every parser is enabled",
                i.version
            );
            Index::default()
        }
        Err(e) => {
            log::warn!("the parser library index cannot be read ({e}): every parser is enabled");
            Index::default()
        }
    }
}

/// `(file name, text or why it cannot be read)` of the user's parser files, sorted by name.
fn user_files(dir: &Path) -> Vec<(String, Result<String, ParserError>)> {
    let Ok(entries) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut names: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.is_file() && p.extension().is_some_and(|e| e == "toml"))
        .collect();
    names.sort();
    names
        .into_iter()
        .take(MAX_USER_PARSERS)
        .filter_map(|p| {
            let name = p.file_name()?.to_string_lossy().into_owned();
            let too_large = std::fs::metadata(&p).is_ok_and(|m| m.len() > super::MAX_PARSER_BYTES as u64);
            let text = if too_large {
                Err(ParserError::new(format!(
                    "the parser file is larger than {} KiB",
                    super::MAX_PARSER_BYTES / 1024
                )))
            } else {
                std::fs::read_to_string(&p).map_err(|e| ParserError::new(format!("cannot read the file: {e}")))
            };
            Some((name, text))
        })
        .collect()
}

fn file_stem(file: &str) -> &str {
    file.strip_suffix(".toml").unwrap_or(file)
}

fn write_atomic(dir: &Path, name: &str, text: &str) -> Result<(), LibraryError> {
    let io = |e: std::io::Error| LibraryError::Io(format!("cannot write {name} in the parser folder: {e}"));
    std::fs::create_dir_all(dir).map_err(io)?;
    let tmp = dir.join(format!(".{name}.tmp"));
    std::fs::write(&tmp, text).map_err(io)?;
    std::fs::rename(&tmp, dir.join(name)).map_err(io)
}

fn remove_file(path: &Path) -> Result<(), LibraryError> {
    match std::fs::remove_file(path) {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(LibraryError::Io(format!("cannot remove {}: {e}", path.display()))),
    }
}

/// `text` with the top-level `key = ...` line (before the first table) set to `value`.
fn replace_top_key(text: &str, key: &str, value: &str) -> String {
    let quoted = toml::Value::String(value.to_string()).to_string();
    let mut out = String::with_capacity(text.len() + value.len());
    let mut done = false;
    for line in text.split_inclusive('\n') {
        let t = line.trim_start();
        if t.starts_with('[') {
            done = true;
        }
        let is_key = t.strip_prefix(key).is_some_and(|rest| rest.trim_start().starts_with('='));
        if !done && is_key {
            out.push_str(&format!("{key} = {quoted}\n"));
            done = true;
        } else {
            out.push_str(line);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Dir(PathBuf);

    impl Dir {
        fn new(tag: &str) -> Dir {
            let d = std::env::temp_dir().join(format!("imt-parser-lib-{tag}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&d);
            Dir(d)
        }
    }

    impl Drop for Dir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn parser(id: &str, version: &str) -> String {
        format!("# shared by a colleague\nschema = 1\nid = \"{id}\"\nname = \"{id} logs\"\nversion = \"{version}\"\n[format]\nkind = \"text\"\nregexes = ['^(?P<timestamp>\\S+ \\S+) (?P<message>.*)$']\n")
    }

    #[test]
    fn a_new_library_holds_the_builtins_enabled() {
        let d = Dir::new("new");
        let lib = Library::load(&d.0);
        assert_eq!(lib.parsers().len(), builtin::FILES.len());
        assert!(lib.parsers().iter().all(|p| p.enabled && p.origin == Origin::Builtin));
        assert!(lib.broken().is_empty());
        let setup = lib.setup();
        assert_eq!(setup.parsers.len(), builtin::FILES.len());
        assert!(setup.parsers.iter().all(|p| p.enabled && !p.user));
        assert!(Library::builtin_only().get("generic").is_some());
    }

    #[test]
    fn user_parsers_are_saved_listed_disabled_and_removed() {
        let d = Dir::new("crud");
        let mut lib = Library::load(&d.0);
        assert_eq!(lib.save(&parser("orders", "1.0.0"), None, false).unwrap(), "orders");
        let p = lib.get("orders").unwrap();
        assert_eq!(p.origin, Origin::User);
        assert!(p.toml.starts_with("# shared by a colleague"), "the text is kept as written");
        // Saving the same id again needs the edit's previous id or a confirmation.
        assert_eq!(
            lib.save(&parser("orders", "1.1.0"), None, false).unwrap_err(),
            LibraryError::Conflict("orders".into())
        );
        lib.save(&parser("orders", "1.1.0"), Some("orders"), false).unwrap();
        lib.save(&parser("orders", "1.2.0"), None, true).unwrap();
        assert_eq!(lib.get("orders").unwrap().parser.version.to_string(), "1.2.0");
        lib.set_enabled("orders", false).unwrap();
        assert!(!Library::load(&d.0).get("orders").unwrap().enabled, "the flag is persisted");
        assert!(!lib.setup().parsers.iter().find(|p| p.toml.contains("\"orders\"")).unwrap().enabled);
        // Renaming keeps the disabled flag and removes the old file.
        lib.save(&parser("orders-v2", "1.2.0"), Some("orders"), false).unwrap();
        assert!(lib.get("orders").is_none());
        assert!(!lib.get("orders-v2").unwrap().enabled);
        lib.remove("orders-v2").unwrap();
        assert!(lib.get("orders-v2").is_none());
        assert_eq!(lib.remove("orders-v2").unwrap_err(), LibraryError::NotFound("orders-v2".into()));
        assert_eq!(lib.remove("generic").unwrap_err(), LibraryError::BuiltIn("generic".into()));
        assert_eq!(lib.set_enabled("nope", true).unwrap_err(), LibraryError::NotFound("nope".into()));
        assert!(matches!(lib.save("schema = 1\n", None, false), Err(LibraryError::Invalid(_))));
        assert!(lib.duplicate("nope").is_err());
    }

    #[test]
    fn a_user_file_overrides_its_builtin_until_removed() {
        let d = Dir::new("override");
        let mut lib = Library::load(&d.0);
        lib.set_enabled("nlog", false).unwrap();
        let edited = builtin::file("nlog")
            .unwrap()
            .toml
            .replace("version = \"1.0.0\"", "version = \"1.0.1\"");
        lib.save(&edited, Some("nlog"), false).unwrap();
        let p = lib.get("nlog").unwrap();
        assert_eq!(
            (p.origin, p.enabled, p.parser.version.to_string().as_str()),
            (Origin::Override, false, "1.0.1")
        );
        assert_eq!(lib.parsers().len(), builtin::FILES.len(), "an override takes the built-in's place");
        lib.remove("nlog").unwrap();
        let p = lib.get("nlog").unwrap();
        assert_eq!((p.origin, p.parser.version.to_string().as_str()), (Origin::Builtin, "1.0.0"));
        assert!(!p.enabled, "the enabled flag belongs to the id");
    }

    #[test]
    fn duplicates_get_a_free_id_and_keep_comments() {
        let d = Dir::new("dup");
        let mut lib = Library::load(&d.0);
        assert_eq!(lib.duplicate("serilog-clef").unwrap(), "serilog-clef-copy");
        assert_eq!(lib.duplicate("serilog-clef").unwrap(), "serilog-clef-copy-2");
        let copy = lib.get("serilog-clef-copy").unwrap();
        assert_eq!(copy.parser.name, "Serilog CLEF (copy)");
        assert!(copy.toml.starts_with("# Serilog compact JSON"));
        assert_eq!(copy.origin, Origin::User);
        assert_eq!(
            replace_top_key("id = 'a'\n[format]\nid = 'b'\n", "id", "x"),
            "id = \"x\"\n[format]\nid = 'b'\n"
        );
    }

    #[test]
    fn imports_are_compared_with_the_library() {
        let d = Dir::new("import");
        let mut lib = Library::load(&d.0);
        assert_eq!(lib.check_import(&parser("orders", "1.0.0")).unwrap().relation, Relation::New);
        lib.save(&parser("orders", "1.0.0"), None, false).unwrap();
        let newer = lib.check_import(&parser("orders", "1.1.0")).unwrap();
        assert_eq!(newer.relation, Relation::Newer);
        assert_eq!(newer.existing.unwrap().parser.version.to_string(), "1.0.0");
        assert_eq!(lib.check_import(&parser("orders", "1.0.0")).unwrap().relation, Relation::Same);
        assert_eq!(lib.check_import(&parser("orders", "0.9.0")).unwrap().relation, Relation::Older);
        let builtin = builtin::file("nlog")
            .unwrap()
            .toml
            .replace("version = \"1.0.0\"", "version = \"2.0.0\"");
        assert_eq!(lib.check_import(&builtin).unwrap().relation, Relation::Newer);
        let future = lib.check_import("schema = 7\nid = \"x\"\n").unwrap_err();
        assert!(future.message.contains("update IMT"));
    }

    #[test]
    fn corrupt_files_fall_back_or_are_listed_as_broken() {
        let d = Dir::new("corrupt");
        std::fs::create_dir_all(&d.0).unwrap();
        std::fs::write(d.0.join(INDEX_FILE), "{ not json").unwrap();
        std::fs::write(d.0.join("bad.toml"), "schema = 1\nid = \"bad\"\nname = \n").unwrap();
        std::fs::write(d.0.join("named.toml"), parser("other", "1.0.0")).unwrap();
        std::fs::write(d.0.join("big.toml"), "#".repeat(super::super::MAX_PARSER_BYTES + 1)).unwrap();
        std::fs::write(d.0.join("notes.txt"), "ignored").unwrap();
        let lib = Library::load(&d.0);
        assert!(lib.parsers().iter().all(|p| p.enabled), "a corrupt index enables everything");
        let broken: Vec<(&str, Option<usize>)> = lib.broken().iter().map(|b| (b.file.as_str(), b.error.line)).collect();
        assert_eq!(broken.len(), 3, "{broken:?}");
        assert!(broken.contains(&("bad.toml", Some(3))));
        assert!(lib
            .broken()
            .iter()
            .any(|b| b.file == "named.toml" && b.error.message.contains("holds the parser")));
        assert!(lib.broken().iter().any(|b| b.file == "big.toml" && b.error.message.contains("KiB")));
        // A newer index is ignored too.
        std::fs::write(d.0.join(INDEX_FILE), "{\"version\": 9, \"disabled\": [\"generic\"]}").unwrap();
        assert!(Library::load(&d.0).get("generic").unwrap().enabled);
        std::fs::write(d.0.join(INDEX_FILE), "{\"version\": 1, \"disabled\": [\"generic\"]}").unwrap();
        assert!(!Library::load(&d.0).get("generic").unwrap().enabled);
    }

    #[test]
    fn a_library_without_a_folder_cannot_save() {
        let mut lib = Library::builtin_only();
        assert!(matches!(lib.save(&parser("x", "1.0.0"), None, false), Err(LibraryError::Io(_))));
        assert!(matches!(lib.set_enabled("generic", false), Err(LibraryError::Io(_))));
    }

    #[test]
    fn renaming_a_disabled_parser_keeps_it_disabled_and_removes_the_old_file() {
        let d = Dir::new("rename-disabled");
        let mut lib = Library::load(&d.0);
        lib.save(&parser("orders", "1.0.0"), None, false).unwrap();
        lib.set_enabled("orders", false).unwrap();
        lib.save(&parser("orders-v2", "1.0.0"), Some("orders"), false).unwrap();
        assert!(!d.0.join("orders.toml").exists(), "the old file is gone");
        assert!(lib.get("orders").is_none());
        assert!(!lib.get("orders-v2").unwrap().enabled, "the rename keeps the disabled state");
    }

    #[test]
    fn removing_a_file_reports_what_cannot_be_removed_and_forgives_a_missing_one() {
        let d = Dir::new("remove-file");
        std::fs::create_dir_all(&d.0).unwrap();
        assert!(remove_file(&d.0.join("missing.toml")).is_ok());
        let err = remove_file(&d.0).unwrap_err();
        assert!(matches!(&err, LibraryError::Io(m) if m.starts_with("cannot remove ")), "{err:?}");
    }
}
