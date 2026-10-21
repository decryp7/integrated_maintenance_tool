//! Parsers per source (ADR 0026): each source records the parser it was read with (id, version, content hash)
//! and the `parsers` table keeps a copy of every parser's TOML, so a shared project says what read it. The
//! library comes from the app ([`Profiles::set_parsers`]); the store keeps its own per-source choices ("Parse
//! with…") in `meta` under [`OVERRIDES_KEY`], and re-reads only the sources a change concerns.

use crate::{engine, read_lock, write_lock, DuckStore, Inner};
use duckdb::{params, Connection};
use imt_core::batch::SourceId;
use imt_core::parser::{ParserSet, ParserSetup};
use imt_core::ports::{ImportReport, ImportSession, Ingest, ParserChange, ParserDef, ParserRef, ParserUse, StoreError, StoreResult};
use imt_core::profile_change::ChangeClass;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

/// The `meta` key of the per-source parser choices (JSON: source path → parser id).
pub(crate) const OVERRIDES_KEY: &str = "parser_overrides";

/// Copies the TOML of every parser of `set` that a source records and the catalog does not have yet.
pub(crate) fn record(conn: &Connection, set: &ParserSet) -> duckdb::Result<()> {
    let used: Vec<String> = {
        let mut stmt = conn.prepare("SELECT DISTINCT parser_hash FROM sources WHERE parser_hash IS NOT NULL")?;
        let rows = stmt.query_map([], |r| r.get::<_, String>(0))?;
        rows.collect::<duckdb::Result<_>>()?
    };
    for (parser, r, _) in set.parsers() {
        if used.contains(&r.hash) {
            conn.execute(
                "INSERT OR IGNORE INTO parsers (hash, id, version, toml) VALUES (?, ?, ?, ?)",
                params![r.hash, r.id, r.version, parser.canonical_toml()],
            )?;
        }
    }
    Ok(())
}

/// The store's per-source parser choices (empty when none or unreadable).
pub(crate) fn overrides(conn: &Connection) -> BTreeMap<String, String> {
    conn.query_row("SELECT value FROM meta WHERE key = ?", params![OVERRIDES_KEY], |r| {
        r.get::<_, String>(0)
    })
    .ok()
    .and_then(|json| match serde_json::from_str(&json) {
        Ok(map) => Some(map),
        Err(e) => {
            log::warn!("the store's parser choices cannot be read ({e}); every file is matched again");
            None
        }
    })
    .unwrap_or_default()
}

fn write_overrides(conn: &Connection, overrides: &BTreeMap<String, String>) -> StoreResult<()> {
    let json = serde_json::to_string(overrides).map_err(|e| StoreError::Engine(e.to_string()))?;
    conn.execute("INSERT OR REPLACE INTO meta VALUES (?, ?)", params![OVERRIDES_KEY, json])
        .map_err(engine)?;
    Ok(())
}

impl Inner {
    /// Which parser each ready source was read with.
    pub(crate) fn parser_uses(&self) -> StoreResult<Vec<ParserUse>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare(
                "SELECT parser_id, parser_version, parser_hash, list(id ORDER BY id) FROM sources
                 WHERE ready AND parser_hash IS NOT NULL GROUP BY ALL ORDER BY parser_id, parser_hash",
            )
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                let ids: duckdb::types::Value = r.get(3)?;
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?, r.get::<_, String>(2)?, ids))
            })
            .map_err(engine)?;
        let mut out = Vec::new();
        for row in rows {
            let (id, version, hash, ids) = row.map_err(engine)?;
            let sources = match ids {
                duckdb::types::Value::List(v) => v
                    .into_iter()
                    .filter_map(|x| match x {
                        duckdb::types::Value::UInt(n) => Some(SourceId(n)),
                        _ => None,
                    })
                    .collect(),
                _ => Vec::new(),
            };
            out.push(ParserUse {
                parser: ParserRef { id, version, hash },
                sources,
            });
        }
        Ok(out)
    }

    /// `(source, path, parser)` of every ready source.
    fn recorded(&self) -> StoreResult<Vec<(SourceId, String, ParserRef)>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT id, path, parser_id, parser_version, parser_hash FROM sources WHERE ready AND parser_hash IS NOT NULL")
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    SourceId(r.get(0)?),
                    r.get::<_, String>(1)?,
                    ParserRef {
                        id: r.get(2)?,
                        version: r.get(3)?,
                        hash: r.get(4)?,
                    },
                ))
            })
            .map_err(engine)?;
        rows.collect::<duckdb::Result<Vec<_>>>().map_err(engine)
    }

    /// The sources read with a parser that is not the one `set` reads them with.
    pub(crate) fn stale_sources(&self, set: &ParserSet) -> StoreResult<Vec<(SourceId, String)>> {
        let recorded = self.recorded()?;
        let stale: Vec<SourceId> = set.stale(recorded.iter().map(|(id, path, r)| (*id, path.as_str(), r)));
        Ok(recorded
            .into_iter()
            .filter(|(id, ..)| stale.contains(id))
            .map(|(id, path, _)| (id, path))
            .collect())
    }

    /// The parser set now.
    pub(crate) fn parser_set(&self) -> Arc<ParserSet> {
        read_lock(&self.state).parsers.clone()
    }

    /// Compiles `setup` (with the store's overrides) and makes it current.
    fn swap_parsers(&self, mut setup: ParserSetup, overrides: BTreeMap<String, String>) -> StoreResult<Arc<ParserSet>> {
        setup.overrides = overrides;
        let profile = self.profile().profile.clone();
        let set = Arc::new(self.registry().compile_parsers(&profile, &setup).map_err(crate::invalid)?);
        let mut s = write_lock(&self.state);
        s.parsers = set.clone();
        s.profile = set.base().clone();
        drop(s);
        *crate::lock(&self.derived) = None;
        Ok(set)
    }

    /// The embedded TOML of a parser (a shared project's record of what read it).
    #[cfg(test)]
    pub(crate) fn parser_toml(&self, hash: &str) -> StoreResult<Option<String>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row("SELECT toml FROM parsers WHERE hash = ?", params![hash], |r| r.get(0))
            .ok())
    }
}

/// The file a source path belongs to (an archive member's archive).
fn file_of(path: &str) -> PathBuf {
    PathBuf::from(path.split(imt_core::archive::ENTRY_SEPARATOR).next().unwrap_or(path))
}

impl DuckStore {
    /// Reads `files` again: a refresh, with the files no opened root covers added as roots first (a store
    /// reopened without its roots, e.g. a restored scratch store, still re-reads them).
    fn read_again(&self, files: &[PathBuf]) -> StoreResult<ImportSession> {
        {
            let mut s = write_lock(&self.inner.state);
            for f in files {
                if !s.roots.iter().any(|r| f.starts_with(r)) {
                    s.roots.push(f.clone());
                }
            }
        }
        Ingest::refresh(self, files)
    }
}

impl DuckStore {
    /// Replaces the library (keeping the store's per-source choices); returns the sources to re-read.
    pub(crate) fn replace_parsers(&self, setup: &ParserSetup) -> StoreResult<Vec<SourceId>> {
        let overrides = overrides(&*self.inner.conn()?);
        let set = self.inner.swap_parsers(setup.clone(), overrides)?;
        Ok(self.inner.stale_sources(&set)?.into_iter().map(|(id, _)| id).collect())
    }

    /// Re-reads the sources whose parser changed and the files no parser read before.
    pub(crate) fn reread_parsers(&self) -> StoreResult<ImportSession> {
        let set = self.inner.parser_set();
        let mut paths: Vec<PathBuf> = self.inner.stale_sources(&set)?.iter().map(|(_, p)| file_of(p)).collect();
        paths.extend(
            self.inner
                .unrecognized()
                .iter()
                .filter(|u| u.reason.starts_with("no enabled parser") || u.reason == "binary file")
                .map(|u| file_of(&u.path)),
        );
        paths.sort();
        paths.dedup();
        if paths.is_empty() {
            return ImportSession::spawn(self.inner.progress.clone(), |_| Ok(ImportReport::default()));
        }
        self.read_again(&paths)
    }

    /// "Parse with…": reads one source with `parser` (`None`: the automatic choice again).
    pub(crate) fn assign(&self, source: SourceId, parser: Option<&str>) -> StoreResult<ImportSession> {
        let conn = self.inner.conn()?;
        let path: String = conn
            .query_row("SELECT path FROM sources WHERE id = ?", params![source.0], |r| r.get(0))
            .map_err(|_| StoreError::NotFound(format!("source {source}")))?;
        let set = self.inner.parser_set();
        if let Some(id) = parser {
            if set.parser(id).is_none() {
                return Err(StoreError::NotFound(format!("parser \"{id}\" (it is not in the library)")));
            }
        }
        let mut chosen = overrides(&conn);
        match parser {
            Some(id) => chosen.insert(path.clone(), id.to_string()),
            None => chosen.remove(&path),
        };
        write_overrides(&conn, &chosen)?;
        drop(conn);
        self.inner.swap_parsers(set.setup().clone(), chosen)?;
        self.read_again(&[file_of(&path)])
    }

    /// What changing a parser of the library to `parser` would do: re-read the sources read with it.
    pub(crate) fn classify_parser_change(&self, parser: &ParserDef) -> StoreResult<ParserChange> {
        let new = imt_core::parser::Parser::from_toml(&parser.toml).map_err(|e| StoreError::Invalid(e.to_string()))?;
        let reference = new.reference();
        let sources: Vec<SourceId> = self
            .inner
            .recorded()?
            .into_iter()
            .filter(|(_, _, r)| r.id == reference.id && r.hash != reference.hash)
            .map(|(id, ..)| id)
            .collect();
        Ok(ParserChange {
            class: if sources.is_empty() {
                ChangeClass::Presentation
            } else {
                ChangeClass::Parsing
            },
            sections: if sources.is_empty() { Vec::new() } else { vec!["format".into()] },
            sources,
        })
    }

    /// The parsers of the project's sources (for `project.toml`).
    pub(crate) fn parser_refs(&self) -> StoreResult<Vec<ParserRef>> {
        Ok(self.inner.parser_uses()?.into_iter().map(|u| u.parser).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::parser::SetupParser;
    use imt_core::ports::{Ingest, Profiles};
    use imt_core::Profile;

    struct Root(PathBuf);

    impl Drop for Root {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn root(tag: &str) -> Root {
        let root = std::env::temp_dir().join(format!("imt-parsers-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("logs")).unwrap();
        Root(root)
    }

    const CLEF: &str = "{\"@t\":\"2026-10-14T10:00:00Z\",\"@mt\":\"Started {App}\",\"App\":\"orders\"}\n";
    const CSV: &str = "2025/11/08,00:00:00.013+01:00,INFO,1000,OrderService,4120,Started\n2025/11/08,00:00:01.013+01:00,WARN,2101,OrderService,4120,Slow\n";
    const TEXT: &str = "2026-10-14 10:00:00 INFO a\n";

    #[test]
    fn a_mixed_folder_is_read_with_a_parser_per_file_and_projects_embed_them() {
        let r = root("mixed");
        std::fs::write(r.0.join("logs/orders.clef"), CLEF).unwrap();
        std::fs::write(r.0.join("logs/orders.csv"), CSV).unwrap();
        std::fs::write(r.0.join("logs/app.log"), TEXT).unwrap();
        let store = DuckStore::create(&r.0.join("store"), Profile::default_profile(), imt_core::PluginRegistry::new()).unwrap();
        store.open(&[r.0.join("logs")]).unwrap().wait().unwrap();
        let uses = store.parsers().unwrap();
        let ids: Vec<&str> = uses.iter().map(|u| u.parser.id.as_str()).collect();
        assert_eq!(ids, ["generic", "log4net-csv", "serilog-clef"]);
        assert!(uses.iter().all(|u| u.sources.len() == 1));
        for u in &uses {
            let toml = store.inner.parser_toml(&u.parser.hash).unwrap().unwrap();
            assert!(toml.contains(&format!("id = \"{}\"", u.parser.id)));
        }
        let mut manifest = imt_core::package::ProjectManifest::default();
        store.save_project(&mut manifest).unwrap();
        assert_eq!(manifest.parsers.len(), 3);
    }

    #[test]
    fn disabling_a_parser_marks_its_sources_stale_and_reread_uses_the_next_best() {
        let r = root("stale");
        std::fs::write(r.0.join("logs/orders.clef"), CLEF).unwrap();
        std::fs::write(r.0.join("logs/app.log"), TEXT).unwrap();
        std::fs::write(r.0.join("logs/notes.txt"), "just words\n").unwrap();
        let store = DuckStore::create(&r.0.join("store"), Profile::default_profile(), imt_core::PluginRegistry::new()).unwrap();
        store.open(&[r.0.join("logs")]).unwrap().wait().unwrap();
        assert_eq!(store.inner.unrecognized().len(), 1);
        let mut setup = ParserSetup::builtins();
        for p in &mut setup.parsers {
            p.enabled = !p.toml.contains("id = \"serilog-clef\"");
        }
        // A user parser that reads the notes.
        setup.parsers.push(SetupParser {
            toml: "schema = 1\nid = \"notes\"\nname = \"Notes\"\nversion = \"1.0.0\"\n[match]\nfiles = ['*.txt']\n[format]\nkind = \"text\"\nregexes = ['^(?P<message>.+)$']\n".into(),
            enabled: true,
            user: true,
        });
        let stale = store.set_parsers(&setup).unwrap();
        assert_eq!(stale.len(), 1, "only the CLEF source");
        store.reread_parsers().unwrap().wait().unwrap();
        let mut ids: Vec<String> = store.parsers().unwrap().into_iter().map(|u| u.parser.id).collect();
        ids.sort();
        assert_eq!(ids, ["generic", "notes"]);
        assert!(store.inner.unrecognized().is_empty(), "the notes are read now");
        assert!(store.set_parsers(&setup).unwrap().is_empty());
    }

    #[test]
    fn parse_with_overrides_one_source_and_survives_a_reopen() {
        let r = root("assign");
        std::fs::write(r.0.join("logs/orders.clef"), CLEF).unwrap();
        let dir = r.0.join("store");
        let store = DuckStore::create(&dir, Profile::default_profile(), imt_core::PluginRegistry::new()).unwrap();
        store.open(&[r.0.join("logs")]).unwrap().wait().unwrap();
        let source = store.parsers().unwrap()[0].sources[0];
        store.assign_parser(source, Some("generic")).unwrap().wait().unwrap();
        let uses = store.parsers().unwrap();
        assert_eq!(uses[0].parser.id, "generic");
        assert!(matches!(
            store.assign_parser(uses[0].sources[0], Some("nope")),
            Err(StoreError::NotFound(_))
        ));
        assert!(matches!(store.assign_parser(SourceId(999), None), Err(StoreError::NotFound(_))));
        // The choice is the store's: a reopen keeps it.
        let id = uses[0].sources[0];
        drop(store);
        let store = DuckStore::open(&dir, Profile::default_profile(), imt_core::PluginRegistry::new()).unwrap();
        let path = std::fs::canonicalize(r.0.join("logs/orders.clef"))
            .unwrap()
            .to_string_lossy()
            .into_owned();
        assert_eq!(store.inner.parser_set().override_of(&path), Some("generic"));
        // Automatic again.
        store.assign_parser(id, None).unwrap().wait().unwrap();
        assert_eq!(store.parsers().unwrap()[0].parser.id, "serilog-clef");
        assert!(overrides(&store.inner.conn().unwrap()).is_empty());
    }

    #[test]
    fn a_parser_change_concerns_the_sources_it_read() {
        let r = root("classify");
        std::fs::write(r.0.join("logs/orders.clef"), CLEF).unwrap();
        std::fs::write(r.0.join("logs/app.log"), TEXT).unwrap();
        let store = DuckStore::create(&r.0.join("store"), Profile::default_profile(), imt_core::PluginRegistry::new()).unwrap();
        store.open(&[r.0.join("logs")]).unwrap().wait().unwrap();
        let clef = imt_core::parser::builtin::find("serilog-clef").unwrap();
        let def = |p: &imt_core::parser::Parser| ParserDef {
            id: p.id.to_string(),
            version: p.version.to_string(),
            toml: p.canonical_toml(),
        };
        let same = store.classify_parser(&def(clef)).unwrap();
        assert_eq!((same.class, same.sources.len()), (ChangeClass::Presentation, 0));
        let changed = clef.with_override("[format]\ntimestamp_formats = [\"%d.%m.%Y\"]").unwrap();
        let c = store.classify_parser(&def(&changed)).unwrap();
        assert_eq!((c.class, c.sources.len()), (ChangeClass::Parsing, 1));
        assert!(store
            .classify_parser(&ParserDef {
                id: "x".into(),
                version: "1".into(),
                toml: "[bad".into()
            })
            .is_err());
        // A corrupt overrides entry is ignored.
        let conn = store.inner.conn().unwrap();
        conn.execute("INSERT OR REPLACE INTO meta VALUES (?, ?)", params![OVERRIDES_KEY, "{not json"])
            .unwrap();
        assert!(overrides(&conn).is_empty());
    }
}
