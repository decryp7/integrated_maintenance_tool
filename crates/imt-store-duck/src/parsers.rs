//! Parsers per source (prepared for ADR 0026: parsers become individual TOML files matched per file, several
//! in one workspace). Each source records the parser it was read with (id, version, content hash) and the
//! `parsers` table embeds each parser's TOML, so a shared project says what read it. Until the parser library
//! exists the workspace's profile is the one parser of every source; a profile change re-reads only the
//! sources whose parser hash differs, and assigning another parser to a source is refused.

use crate::{engine, DuckStore, Inner};
use duckdb::{params, Connection};
use imt_core::batch::SourceId;
use imt_core::ports::{ParserChange, ParserDef, ParserRef, ParserUse, StoreError, StoreResult};
use imt_core::profile_change::{classify, ChangeClass};
use imt_core::Profile;

/// Records `profile` as the parser of the sources that have none yet (those just imported).
pub(crate) fn record(conn: &Connection, profile: &Profile) -> duckdb::Result<()> {
    let p = ParserRef::of_profile(profile);
    conn.execute(
        "INSERT OR IGNORE INTO parsers (hash, id, version, toml) VALUES (?, ?, ?, ?)",
        params![p.hash, p.id, p.version, imt_core::ports::profile_toml(profile)],
    )?;
    conn.execute(
        "UPDATE sources SET parser_id = ?, parser_version = ?, parser_hash = ? WHERE parser_hash IS NULL",
        params![p.id, p.version, p.hash],
    )?;
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

    /// The sources whose parser is not `parser` (they need reading again under it).
    pub(crate) fn sources_not_read_with(&self, parser: &ParserRef) -> StoreResult<Vec<(SourceId, String)>> {
        let conn = self.conn()?;
        let mut stmt = conn
            .prepare("SELECT id, path FROM sources WHERE ready AND (parser_hash IS NULL OR parser_hash <> ?)")
            .map_err(engine)?;
        let rows = stmt
            .query_map(params![parser.hash], |r| Ok((SourceId(r.get(0)?), r.get::<_, String>(1)?)))
            .map_err(engine)?;
        rows.collect::<duckdb::Result<Vec<_>>>().map_err(engine)
    }

    /// The embedded TOML of a parser (a shared project's record of what read it; ADR 0026 reads it back).
    #[cfg(test)]
    pub(crate) fn parser_toml(&self, hash: &str) -> StoreResult<Option<String>> {
        let conn = self.conn()?;
        Ok(conn
            .query_row("SELECT toml FROM parsers WHERE hash = ?", params![hash], |r| r.get(0))
            .ok())
    }
}

impl DuckStore {
    /// What changing the interim parser (the profile) to `parser` would do.
    pub(crate) fn classify_parser_change(&self, parser: &ParserDef) -> StoreResult<ParserChange> {
        let new = Profile::from_toml(&parser.toml).map_err(|e| StoreError::Invalid(e.message))?;
        let change = classify(&self.inner.profile().profile, &new);
        let sources = if change.class == ChangeClass::Presentation {
            Vec::new()
        } else {
            self.inner
                .sources_not_read_with(&ParserRef::of_profile(&new))?
                .into_iter()
                .map(|(id, _)| id)
                .collect()
        };
        Ok(ParserChange {
            class: change.class,
            sections: change.sections,
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
    use imt_core::ports::{Ingest, Profiles};

    #[test]
    fn sources_record_their_parser_and_projects_embed_it() {
        let root = std::env::temp_dir().join(format!("imt-parsers-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("logs")).unwrap();
        std::fs::write(root.join("logs/a.log"), "2026-10-14 10:00:00 INFO a\n").unwrap();
        std::fs::write(root.join("logs/b.log"), "2026-10-14 10:00:00 INFO b\n").unwrap();
        let profile = Profile::default_profile();
        let store = DuckStore::create(&root.join("store"), profile.clone(), imt_core::PluginRegistry::new()).unwrap();
        store.open(&[root.join("logs")]).unwrap().wait().unwrap();
        let uses = store.parsers().unwrap();
        assert_eq!(uses.len(), 1, "one parser (the profile) reads every source");
        assert_eq!(uses[0].parser, ParserRef::of_profile(&profile));
        assert_eq!(uses[0].sources.len(), 2);
        assert!(store.inner.parser_toml(&uses[0].parser.hash).unwrap().unwrap().contains("[parse]"));
        // A presentation change touches no source; a parsing change touches the sources of that parser.
        let mut renamed = profile.clone();
        renamed.name = "Renamed".into();
        let def = |p: &Profile| ParserDef {
            id: ParserRef::PROFILE.into(),
            version: p.name.clone(),
            toml: imt_core::ports::profile_toml(p),
        };
        let c = store.classify_parser(&def(&renamed)).unwrap();
        assert_eq!((c.class, c.sources.len()), (ChangeClass::Presentation, 0));
        let mut reparse = profile.clone();
        reparse.parse.timestamp_formats.push("%d/%m/%Y %H:%M:%S".into());
        let c = store.classify_parser(&def(&reparse)).unwrap();
        assert_eq!((c.class, c.sources.len()), (ChangeClass::Parsing, 2));
        assert!(matches!(
            store.assign_parser(SourceId(0), &def(&reparse)),
            Err(StoreError::Unsupported { .. })
        ));
        // Re-reading under the new profile records it.
        store.set_profile(reparse.clone()).unwrap().wait().unwrap();
        let uses = store.parsers().unwrap();
        assert_eq!(uses.len(), 1);
        assert_eq!(uses[0].parser, ParserRef::of_profile(&reparse));
        let mut manifest = imt_core::package::ProjectManifest::default();
        store.save_project(&mut manifest).unwrap();
        assert_eq!(manifest.parsers, vec![ParserRef::of_profile(&reparse)]);
        drop(store);
        let _ = std::fs::remove_dir_all(&root);
    }
}
