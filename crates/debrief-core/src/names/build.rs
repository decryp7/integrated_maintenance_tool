//! Building the name book from the catalogue and keeping it for the app: [`Names`] is the shared handle the assistant
//! tools and the commands use (book, workspace vocabulary, context and Settings switches behind locks that recover
//! from poisoning).

use super::alias_job::{self, AliasJobConfig, AliasJobReport, AliasModel};
use super::book::NameBook;
use super::context::SearchContext;
use super::group::group;
use super::normalise::{file_name, strip_extension};
use super::resolve::NameSettings;
use super::segment::{split_raw, Vocabulary};
use crate::catalogue::{Candidate, CatalogueRequest};
use crate::ports::StoreResult;
use crate::search_all::SearchSource;
use std::sync::{Mutex, MutexGuard, RwLock};

/// The workspace's own words: pieces of names that are separated in the logs (`Graphic` and `Builder` in
/// `CompanyA.Graphic.Builder.exe`). A name written as one piece teaches nothing, so run-together spellings never
/// vouch for themselves.
pub fn vocabulary_from(candidates: &[Candidate]) -> Vocabulary {
    let mut v = Vocabulary::builtin();
    for c in candidates {
        let stem = strip_extension(file_name(&c.name));
        let pieces = split_raw(stem);
        if pieces.len() < 2 {
            continue;
        }
        for p in pieces
            .iter()
            .filter(|p| p.chars().count() >= 3 && p.chars().all(char::is_alphabetic))
        {
            v.add(p, c.count.clamp(1, 1000));
        }
    }
    v
}

/// What the app shares: the book, its vocabulary, the user's context and the Settings switches.
#[derive(Default)]
pub struct Names {
    book: RwLock<NameBook>,
    vocab: RwLock<Vocabulary>,
    ctx: Mutex<SearchContext>,
    settings: Mutex<NameSettings>,
}

impl Names {
    /// Rebuilds the table from the store's catalogue (when it is built) and returns the number of groups. The
    /// store is read once, off the lock; AI aliases and user facts are kept.
    pub fn refresh(&self, store: &(impl SearchSource + ?Sized)) -> StoreResult<usize> {
        let reply = store.catalogue(&CatalogueRequest {
            all: true,
            ..CatalogueRequest::default()
        })?;
        if !reply.built {
            return Ok(self.read(|b, _| b.groups().len()));
        }
        let vocab = vocabulary_from(&reply.candidates);
        let groups = group(&reply.candidates, &vocab);
        let n = groups.len();
        self.book.write().unwrap_or_else(|p| p.into_inner()).rebuild(groups);
        *self.vocab.write().unwrap_or_else(|p| p.into_inner()) = vocab;
        Ok(n)
    }

    /// Reads the book and vocabulary.
    pub fn read<R>(&self, f: impl FnOnce(&NameBook, &Vocabulary) -> R) -> R {
        let b = self.book.read().unwrap_or_else(|p| p.into_inner());
        let v = self.vocab.read().unwrap_or_else(|p| p.into_inner());
        f(&b, &v)
    }

    /// Changes the book (user feedback, AI entries).
    pub fn write<R>(&self, f: impl FnOnce(&mut NameBook) -> R) -> R {
        f(&mut self.book.write().unwrap_or_else(|p| p.into_inner()))
    }

    /// Replaces the book (loaded from its file); the table is rebuilt at the next [`Names::refresh`].
    pub fn set_book(&self, book: NameBook) {
        *self.book.write().unwrap_or_else(|p| p.into_inner()) = book;
    }

    pub fn context(&self) -> MutexGuard<'_, SearchContext> {
        self.ctx.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn settings(&self) -> NameSettings {
        *self.settings.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub fn set_settings(&self, s: NameSettings) {
        *self.settings.lock().unwrap_or_else(|p| p.into_inner()) = s;
    }

    /// Runs the alias job (see [`alias_job::run`]); the book is locked only while storing answers, never while the
    /// model thinks. Does nothing when the Settings switch is off.
    pub fn run_alias_job(&self, model: Option<&dyn AliasModel>, cfg: &AliasJobConfig, cancelled: &dyn Fn() -> bool) -> AliasJobReport {
        let cfg = AliasJobConfig {
            enabled: cfg.enabled && self.settings().ai_aliases,
            ..cfg.clone()
        };
        let mut snapshot = self.read(|b, _| b.clone());
        let before: std::collections::BTreeSet<String> = snapshot.groups().iter().map(|g| g.id()).collect();
        let rep = alias_job::run(&mut snapshot, model, &cfg, cancelled);
        self.write(|b| {
            for id in before {
                if let Some(e) = snapshot.ai(&id) {
                    if !b.ai_current(&id, &e.model) {
                        b.set_ai(&id, e.clone());
                    }
                }
            }
        });
        rep
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalogue::{CatalogueReply, Scope};
    use crate::entities::EntityKind;

    fn cand(name: &str, n: u64) -> Candidate {
        Candidate {
            scope: Scope::Entity(EntityKind::Executable),
            name: name.into(),
            count: n,
            first_ts: None,
            last_ts: None,
            seen_in: vec![],
        }
    }

    #[test]
    fn vocabulary_learns_only_separated_words() {
        let v = vocabulary_from(&[cand("Acme.Quasar.Hub.exe", 9), cand("zorblaxfoo.exe", 50), cand("a-b.exe", 1)]);
        assert!(v.knows("quasar") && v.knows("acme"));
        assert!(!v.knows("zorblaxfoo"));
        assert!(!v.knows("a"));
    }

    struct Fixed(Vec<Candidate>, bool);

    impl SearchSource for Fixed {
        fn summary(&self) -> StoreResult<crate::ports::SummaryReply> {
            unreachable!()
        }
        fn page(&self, _: &crate::ports::PageQuery) -> StoreResult<crate::ports::PageReply> {
            unreachable!()
        }
        fn facet(&self, _: &crate::api::FacetRequest) -> StoreResult<crate::api::Facet> {
            unreachable!()
        }
        fn exception_groups(&self) -> StoreResult<Vec<crate::exceptions::ExceptionGroup>> {
            unreachable!()
        }
        fn catalogue(&self, req: &CatalogueRequest) -> StoreResult<CatalogueReply> {
            assert!(req.all);
            Ok(CatalogueReply {
                built: self.1,
                pending: 0,
                candidates: self.0.clone(),
            })
        }
    }

    #[test]
    fn refresh_builds_groups_and_keeps_learning() {
        let names = Names::default();
        let store = Fixed(vec![cand("Acme.Graphic.Builder.exe", 5), cand("graphicbuilder.exe", 9)], true);
        assert_eq!(names.refresh(&store).unwrap(), 1);
        names.write(|b| b.accept("gfx", "executable|graphic builder", false));
        assert_eq!(names.refresh(&store).unwrap(), 1);
        assert_eq!(names.read(|b, _| b.facts().len()), 1);
        // A catalogue that is not built keeps the old table.
        let unbuilt = Fixed(vec![], false);
        assert_eq!(names.refresh(&unbuilt).unwrap(), 1);
        names.context().remember("graphic");
        assert_eq!(names.context().recent.len(), 1);
        names.set_settings(NameSettings {
            ai_aliases: false,
            context: true,
        });
        assert!(!names.settings().ai_aliases);
        names.set_book(NameBook::default());
        assert_eq!(names.read(|b, _| b.groups().len()), 0);
    }

    struct M;
    impl AliasModel for M {
        fn version(&self) -> String {
            "m1".into()
        }
        fn complete(&self, _: &str) -> Result<String, String> {
            Ok(r#"{"items":[{"id":1,"name":"Graphic Builder","aliases":["image editor"]}]}"#.into())
        }
    }

    #[test]
    fn alias_job_respects_the_settings_switch() {
        let names = Names::default();
        names.refresh(&Fixed(vec![cand("graphicbuilder.exe", 50)], true)).unwrap();
        let cfg = AliasJobConfig {
            pause: std::time::Duration::ZERO,
            ..AliasJobConfig::default()
        };
        names.set_settings(NameSettings {
            ai_aliases: false,
            context: true,
        });
        assert_eq!(names.run_alias_job(Some(&M), &cfg, &|| false).skipped, "off");
        names.set_settings(NameSettings::default());
        assert_eq!(names.run_alias_job(Some(&M), &cfg, &|| false).stored, 1);
        assert_eq!(
            names.read(|b, _| b.ai("executable|graphic builder").map(|e| e.aliases.clone())),
            Some(vec!["image editor".to_string()])
        );
    }
}
