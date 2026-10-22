//! Symbols for dumps (ADR 0036): Settings ▸ Symbols and the host's part of naming a dump's frames.
//!
//! A dump is opened by the dumps plugin, which names frames from the symbol folders (the user's, then IMT's symbol
//! cache) by PDB identity (name, GUID, age) and records each module's [`SymbolStatus`]. The modules whose PDB is
//! missing are then asked of the user's symbol servers here, into the cache, as a Jobs job (cancellable, with
//! progress) that runs after the dump is open: the dump shows at once with those modules "downloading", and is read
//! again with the new PDBs when they arrive. Downloads are off until the user turns them on; nothing is requested
//! while they are off.

use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use imt_dump::model::{Document, SymbolStatus};
use imt_dump::DumpDocs;
use imt_symbols::fetch::{FetchError, Fetcher};
use imt_symbols::id::PdbKey;
use imt_symbols::servers::{get_all, get_all_observed, Gate, Outcome, ServersConfig, MAX_CONCURRENT, MAX_SERVERS};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, RwLock};
use tauri::{AppHandle, Manager, Runtime};

pub const SECTION: &str = "symbols";

/// Most PDBs asked of the servers for one dump.
pub const MAX_DOWNLOADS: usize = 256;

/// The user's symbol settings, as in effect.
#[derive(Debug, Clone, Default)]
pub struct SymbolSettings {
    pub servers: ServersConfig,
    /// Where PDBs are cached; `None` = the default folder below IMT's data folder.
    pub cache_folder: Option<PathBuf>,
}

/// The symbol settings and the download gate (at most `concurrent` downloads at a time across every dump being
/// opened), managed by the app.
pub struct Symbols {
    settings: RwLock<SymbolSettings>,
    gate: RwLock<Arc<Gate>>,
    default_cache: Option<PathBuf>,
}

impl Symbols {
    pub fn new(default_cache: Option<PathBuf>) -> Symbols {
        let settings = SymbolSettings::default();
        Symbols {
            gate: RwLock::new(Arc::new(Gate::new(settings.servers.concurrent))),
            settings: RwLock::new(settings),
            default_cache,
        }
    }

    pub fn get(&self) -> SymbolSettings {
        crate::sync::read(&self.settings).clone()
    }

    fn set(&self, s: SymbolSettings) {
        *crate::sync::write(&self.gate) = Arc::new(Gate::new(s.servers.concurrent));
        *crate::sync::write(&self.settings) = s;
    }

    /// The folder PDBs are cached in (`None` without a data folder: nothing is cached or downloaded).
    pub fn cache_dir(&self) -> Option<PathBuf> {
        self.get().cache_folder.or_else(|| self.default_cache.clone())
    }

    /// Starts completing a freshly read dump: its missing modules are marked from what the user's settings allow.
    /// With downloads off (or no cache folder, or a bad address) that is final at once. With them on, the dump is
    /// handed back with its missing modules marked [`SymbolStatus::Downloading`] and the [`Download`] to run as a
    /// job: opening a dump never waits for the network.
    pub fn begin(&self, doc: Document, max_text: usize) -> Opened {
        let Some(cache) = self.cache_dir() else { return Opened::Final(doc) };
        let settings = self.get();
        let gate = crate::sync::read(&self.gate).clone();
        let servers = match settings.servers.servers() {
            Ok(s) => s,
            Err(e) => {
                let detail = e.to_string();
                let mut doc = doc;
                if let Document::Dump(d) = &mut doc {
                    imt_dump::symbols::mark_missing(d, |_| Some(SymbolStatus::DownloadFailed { detail: detail.clone() }), max_text);
                }
                return Opened::Final(doc);
            }
        };
        let Document::Dump(dump) = &doc else { return Opened::Final(doc) };
        let keys = imt_dump::symbols::missing(dump, MAX_DOWNLOADS);
        if keys.is_empty() {
            return Opened::Final(doc);
        }
        let Some(fetcher) = servers else {
            // Off: the cache is the only place to look, and it is quick.
            let outcomes = get_all(None, &keys, &cache, &gate);
            let mut doc = doc;
            Update {
                reimported: None,
                outcomes,
            }
            .mark(&mut doc, max_text);
            return Opened::Final(doc);
        };
        let mut doc = doc;
        if let Document::Dump(d) = &mut doc {
            imt_dump::symbols::mark_downloading(d, &keys);
        }
        Opened::Downloading(
            doc,
            Download {
                keys,
                cache,
                fetcher: Arc::new(fetcher),
                gate,
                config: settings.servers,
            },
        )
    }
}

/// A dump read, and whether its symbols are still being downloaded.
pub enum Opened {
    /// Nothing more to do (or nothing that can be done: downloads are off).
    Final(Document),
    /// The dump as read, with the modules being downloaded marked; the download is yet to run.
    Downloading(Document, Download),
}

/// The PDBs of one dump's missing modules, to get from the user's symbol servers (a Jobs job).
pub struct Download {
    keys: Vec<PdbKey>,
    cache: PathBuf,
    fetcher: Arc<dyn Fetcher>,
    gate: Arc<Gate>,
    config: ServersConfig,
}

impl Download {
    /// A download of `dump`'s missing PDBs through `fetcher` (tests).
    #[cfg(test)]
    pub(crate) fn with_fetcher(dump: &imt_dump::model::Dump, cache: &std::path::Path, fetcher: Arc<dyn Fetcher>) -> Download {
        Download {
            keys: imt_dump::symbols::missing(dump, MAX_DOWNLOADS),
            cache: cache.to_path_buf(),
            fetcher,
            gate: Arc::new(Gate::new(1)),
            config: ServersConfig::default(),
        }
    }

    /// How many PDBs are asked.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    #[cfg(test)]
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// Gets the PDBs into the cache. No new one is started once `cancel` is set (a running one finishes within its
    /// own limits); `progress(done, total)` is told as each PDB is settled. When any arrived, `reimport` reads the
    /// dump again with them (a dump that cannot be read again keeps its first reading).
    pub fn run(
        &self,
        cancel: &AtomicBool,
        progress: &(dyn Fn(usize, usize) + Sync),
        reimport: impl FnOnce() -> imt_dump::Result<Document>,
    ) -> Update {
        let total = self.keys.len();
        let done = AtomicUsize::new(0);
        let outcomes = get_all_observed(Some(&*self.fetcher), &self.keys, &self.cache, &self.gate, cancel, &|_, _| {
            progress(done.fetch_add(1, Ordering::Relaxed) + 1, total);
        });
        let arrived = outcomes.iter().any(|(_, o)| matches!(o, Outcome::Downloaded(_)));
        let reimported = if arrived {
            imt_symbols::cache::prune(&self.cache, self.config.cache_max_bytes);
            reimport().ok()
        } else {
            None
        };
        Update { reimported, outcomes }
    }

    /// What to apply when the download could not even be started: the modules go back to missing.
    pub fn abandoned(&self) -> Update {
        Update {
            reimported: None,
            outcomes: Vec::new(),
        }
    }
}

/// What a download came to, ready to be put into the open document.
pub struct Update {
    reimported: Option<Document>,
    outcomes: Vec<(PdbKey, Outcome)>,
}

impl Update {
    /// Whether any PDB was downloaded.
    #[cfg(test)]
    pub fn arrived(&self) -> bool {
        self.outcomes.iter().any(|(_, o)| matches!(o, Outcome::Downloaded(_)))
    }

    /// Why the download failed, when nothing arrived and a server or the network could not be reached (a PDB that
    /// is simply not on the servers is not a failure).
    pub fn failure(&self) -> Option<String> {
        if self
            .outcomes
            .iter()
            .any(|(_, o)| matches!(o, Outcome::Downloaded(_) | Outcome::Cached(_)))
        {
            return None;
        }
        self.outcomes.iter().find_map(|(_, o)| match o {
            Outcome::Failed(e) if e.is_transient() => Some(e.to_string()),
            _ => None,
        })
    }

    fn mark(&self, doc: &mut Document, max_text: usize) {
        if let Document::Dump(d) = doc {
            mark_outcomes(d, &self.outcomes, max_text);
        }
    }

    /// Puts the result into the open document `id`: the dump read again with the new PDBs, or the modules' statuses
    /// in the document as it is. `false` when the document was closed (or reopened) meanwhile: nothing is changed.
    pub fn apply(self, docs: &RwLock<DumpDocs>, id: u32, max_text: usize) -> bool {
        match self.reimported {
            Some(mut doc) => {
                if let Document::Dump(d) = &mut doc {
                    mark_outcomes(d, &self.outcomes, max_text);
                }
                crate::sync::write(docs).replace(id, doc).unwrap_or(false)
            }
            None => crate::sync::write(docs).update(id, |doc| {
                if let Document::Dump(d) = doc {
                    mark_outcomes(d, &self.outcomes, max_text);
                }
            }),
        }
    }
}

/// Gives every module still missing or downloading the status its PDB's outcome says (none heard: missing).
fn mark_outcomes(d: &mut imt_dump::model::Dump, outcomes: &[(PdbKey, Outcome)], max_text: usize) {
    imt_dump::symbols::mark_missing(
        d,
        |k| {
            Some(
                outcomes
                    .iter()
                    .find(|(key, _)| key == k)
                    .map_or(SymbolStatus::Missing, |(_, o)| status_of(o)),
            )
        },
        max_text,
    );
}

/// What a download outcome says about the module's symbols, for a module that is still without them (one whose PDB
/// arrived and loaded is not touched: the dump was read again).
fn status_of(outcome: &Outcome) -> SymbolStatus {
    match outcome {
        Outcome::Cached(_) | Outcome::Downloaded(_) | Outcome::Cancelled => SymbolStatus::Missing,
        Outcome::Disabled => SymbolStatus::ServersDisabled,
        Outcome::Failed(FetchError::NotFound(_)) => SymbolStatus::NotOnServer,
        Outcome::Failed(FetchError::Mismatch(m)) => SymbolStatus::Mismatch {
            detail: format!("the symbol server's {m}"),
        },
        Outcome::Failed(e) => SymbolStatus::DownloadFailed { detail: e.to_string() },
    }
}

/// Settings ▸ Symbols.
pub struct SymbolsSection {
    pub symbols: Arc<Symbols>,
}

impl SymbolsSection {
    fn from_values(values: &ValidValues) -> anyhow::Result<SymbolSettings> {
        let d = ServersConfig::default();
        let servers = ServersConfig {
            enabled: values.bool("serversEnabled").unwrap_or(false),
            urls: values.strings("serverUrls").unwrap_or_default(),
            timeout: values.duration("downloadLimit").unwrap_or(d.timeout),
            max_bytes: values.bytes("maxPdbSize").unwrap_or(d.max_bytes),
            concurrent: values
                .int("concurrent")
                .map_or(d.concurrent, |n| n.clamp(1, MAX_CONCURRENT as i64) as usize),
            cache_max_bytes: values.bytes("cacheMaxSize").unwrap_or(d.cache_max_bytes),
        };
        // Every address is checked here, so a bad one is refused when it is saved (not when a dump is opened).
        ServersConfig {
            enabled: true,
            ..servers.clone()
        }
        .servers()
        .map_err(|e| anyhow::anyhow!("{e}"))?;
        let folder = values.text("cacheFolder").unwrap_or_default().trim();
        Ok(SymbolSettings {
            servers,
            cache_folder: (!folder.is_empty()).then(|| PathBuf::from(folder)),
        })
    }
}

impl Configurable for SymbolsSection {
    fn settings_section(&self) -> SettingsSection {
        let d = ServersConfig::default();
        SettingsSection::new(SECTION, "Symbols", SectionOwner::App)
            .describe(
                "Where IMT finds the symbols (PDB files) that name the functions in a crash dump. A PDB is used only \
                 when it matches the module's own identity (name, GUID and age), never by its file name alone. \
                 Downloads are off until you turn them on; then a dump's missing PDBs are asked of the servers below \
                 (each request sends the PDB's name, GUID and age, which are in the dump) and kept in the cache.",
            )
            .field(
                SettingField::new("serversEnabled", "Download symbols from symbol servers", FieldKind::Bool, false)
                    .describe("Off by default: nothing is requested from the network while it is off."),
            )
            .field(
                SettingField::new(
                    "serverUrls",
                    "Symbol servers",
                    FieldKind::StringList { max_items: MAX_SERVERS },
                    d.urls.clone(),
                )
                .describe("HTTPS addresses, asked in order. The first that has the PDB gives it."),
            )
            .field(
                SettingField::new(
                    "downloadLimit",
                    "Longest download",
                    FieldKind::Duration {
                        min_ms: 1_000,
                        max_ms: 600_000,
                    },
                    d.timeout.as_millis() as u64,
                )
                .describe("The longest one PDB may take to download."),
            )
            .field(
                SettingField::new(
                    "maxPdbSize",
                    "Largest PDB",
                    FieldKind::Size {
                        min_bytes: 1 << 20,
                        max_bytes: 8 << 30,
                    },
                    d.max_bytes,
                )
                .describe("A PDB larger than this is not downloaded."),
            )
            .field(
                SettingField::new(
                    "concurrent",
                    "Downloads at the same time",
                    FieldKind::Int {
                        min: 1,
                        max: MAX_CONCURRENT as i64,
                        step: 1,
                        unit: None,
                    },
                    d.concurrent as i64,
                )
                .describe("Across every dump being opened."),
            )
            .field(
                SettingField::new(
                    "cacheMaxSize",
                    "Cache size",
                    FieldKind::Size {
                        min_bytes: 64 << 20,
                        max_bytes: 64 << 30,
                    },
                    d.cache_max_bytes,
                )
                .describe("The cache keeps the PDBs used most recently; the others are removed when it grows past this."),
            )
            .field(
                SettingField::new("cacheFolder", "Cache folder", FieldKind::Path { dir: true }, "")
                    .describe("Where downloaded PDBs are kept. Empty uses a folder in IMT's data folder."),
            )
    }

    fn apply_settings(&self, values: &ValidValues) -> anyhow::Result<()> {
        self.symbols.set(Self::from_values(values)?);
        Ok(())
    }
}

/// Registers the Symbols section and manages [`Symbols`]. Call from `setup` after `settings::init`.
pub fn init<R: Runtime>(app: &AppHandle<R>) {
    let default_cache = crate::app_dirs::data(app).ok().map(|d| d.join("symbols"));
    let symbols = Arc::new(Symbols::new(default_cache));
    app.manage(symbols.clone());
    if let Err(e) = crate::settings::register(app, Arc::new(SymbolsSection { symbols }), crate::settings::SectionGroup::Symbols) {
        log::error!("Could not register the symbols settings: {e:#}");
    }
}

/// The managed [`Symbols`] (a fresh one with downloads off when the app has none, e.g. in tests).
pub fn of<R: Runtime>(app: &AppHandle<R>) -> Arc<Symbols> {
    app.try_state::<Arc<Symbols>>()
        .map_or_else(|| Arc::new(Symbols::new(None)), |s| s.inner().clone())
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_dump::model::Dump;
    use imt_dump::testing::sample_dump;
    use std::path::Path;
    use std::sync::Mutex;

    /// Answers each PDB by name from a table; counts the requests.
    struct Table {
        answers: Mutex<Vec<(String, Result<(), FetchError>)>>,
        asked: AtomicUsize,
    }

    impl Fetcher for Table {
        fn fetch(&self, key: &PdbKey, cache: &Path) -> Result<PathBuf, FetchError> {
            self.asked.fetch_add(1, Ordering::SeqCst);
            let answers = self.answers.lock().unwrap();
            match answers.iter().find(|(n, _)| key.name().to_string() == *n) {
                Some((_, Ok(()))) => Ok(cache.join("fetched.pdb")),
                Some((_, Err(e))) => Err(e.clone()),
                None => Err(FetchError::NotFound(key.name().to_string())),
            }
        }
    }

    fn table(answers: Vec<(&str, Result<(), FetchError>)>) -> Table {
        Table {
            answers: Mutex::new(answers.into_iter().map(|(n, r)| (n.to_string(), r)).collect()),
            asked: AtomicUsize::new(0),
        }
    }

    /// The sample dump with none of its modules' symbols found.
    fn missing_dump() -> Dump {
        let mut d = sample_dump();
        for m in &mut d.modules {
            m.symbols = SymbolStatus::Missing;
        }
        d
    }

    fn dump_of(d: Document) -> Dump {
        match d {
            Document::Dump(d) => d,
            Document::Umdh(_) => panic!("a dump was expected"),
        }
    }

    fn missing_names(d: &Dump) -> Vec<String> {
        imt_dump::symbols::missing(d, 100).iter().map(|k| k.name().to_string()).collect()
    }

    fn status(d: &Dump, pdb: &str) -> SymbolStatus {
        d.modules
            .iter()
            .find(|m| m.debug_file.as_deref() == Some(pdb))
            .map(|m| m.symbols.clone())
            .unwrap_or_else(|| panic!("no module with {pdb}"))
    }

    fn download(fetcher: Arc<dyn Fetcher>, dump: &Dump, cache: &Path) -> Download {
        Download::with_fetcher(dump, cache, fetcher)
    }

    /// The dump with its missing modules marked, in a document set under id 1.
    fn opened(dump: &Dump, download: &Download) -> (RwLock<DumpDocs>, u32) {
        let mut marked = dump.clone();
        imt_dump::symbols::mark_downloading(&mut marked, &download.keys);
        let mut docs = DumpDocs::new(Vec::new(), imt_dump::Limits::default());
        let id = docs.insert("a.dmp", Some("/x/a.dmp"), Document::Dump(marked)).unwrap().id;
        (RwLock::new(docs), id)
    }

    fn stored(docs: &RwLock<DumpDocs>, id: u32) -> Dump {
        let rows = crate::sync::read(docs)
            .modules(&imt_dump::api::ModulesRequest {
                id,
                offset: 0,
                limit: 100,
                filter: None,
            })
            .unwrap();
        let mut d = missing_dump();
        for m in &mut d.modules {
            let Some(row) = rows.rows.iter().find(|r| r.debug_file.is_some() && r.debug_file == m.debug_file) else {
                continue;
            };
            m.symbols = match row.symbols.clone() {
                imt_dump::api::SymbolStatusDto::Downloading => SymbolStatus::Downloading,
                imt_dump::api::SymbolStatusDto::Loaded => SymbolStatus::Loaded,
                imt_dump::api::SymbolStatusDto::Missing => SymbolStatus::Missing,
                imt_dump::api::SymbolStatusDto::NotOnServer => SymbolStatus::NotOnServer,
                imt_dump::api::SymbolStatusDto::DownloadFailed { detail } => SymbolStatus::DownloadFailed { detail },
                imt_dump::api::SymbolStatusDto::Mismatch { detail } => SymbolStatus::Mismatch { detail },
                other => panic!("unexpected {other:?}"),
            };
        }
        d
    }

    fn settings_on(cache: &Path) -> Symbols {
        let symbols = Symbols::new(Some(cache.to_path_buf()));
        symbols.set(SymbolSettings {
            servers: ServersConfig {
                enabled: true,
                ..ServersConfig::default()
            },
            cache_folder: None,
        });
        symbols
    }

    #[test]
    fn with_downloads_off_every_missing_module_says_so_at_once() {
        let first = missing_names(&missing_dump());
        assert!(!first.is_empty());
        let cache = tempfile::tempdir().unwrap();
        let symbols = Symbols::new(Some(cache.path().to_path_buf()));
        let Opened::Final(doc) = symbols.begin(Document::Dump(missing_dump()), 120) else {
            panic!("with downloads off nothing is left to do in the background");
        };
        let d = dump_of(doc);
        for pdb in &first {
            assert_eq!(status(&d, pdb), SymbolStatus::ServersDisabled, "{pdb}");
        }
        assert!(missing_names(&d).is_empty());
    }

    #[test]
    fn with_downloads_on_the_dump_opens_at_once_with_its_missing_modules_downloading() {
        let cache = tempfile::tempdir().unwrap();
        let symbols = settings_on(cache.path());
        let Opened::Downloading(doc, download) = symbols.begin(Document::Dump(missing_dump()), 120) else {
            panic!("a download was expected");
        };
        let d = dump_of(doc);
        let names = missing_names(&missing_dump());
        assert_eq!(download.len(), names.len());
        assert!(!download.is_empty());
        for pdb in &names {
            assert_eq!(status(&d, pdb), SymbolStatus::Downloading, "{pdb}");
        }
        assert!(missing_names(&d).is_empty(), "none is both missing and downloading");
        // Nothing missing, or a UMDH log: nothing to download.
        assert!(matches!(symbols.begin(Document::Dump(sample_dump()), 120), Opened::Final(_)));
        let umdh = Document::Umdh(imt_dump::testing::sample_umdh(imt_dump::model::UmdhKind::Diff, 0, &[(1, 2, 3)]));
        assert!(matches!(symbols.begin(umdh, 120), Opened::Final(_)));
    }

    #[test]
    fn a_late_arrival_updates_the_open_dump_and_failures_say_why() {
        let names = missing_names(&missing_dump());
        assert!(names.len() >= 3, "{names:?}");
        let fetcher = Arc::new(table(vec![
            (&names[0], Ok(())),
            (&names[1], Err(FetchError::Network("timed out".into()))),
            (&names[2], Err(FetchError::Mismatch("answer is another build".into()))),
        ]));
        let cache = tempfile::tempdir().unwrap();
        let dl = download(fetcher.clone(), &missing_dump(), cache.path());
        let (docs, id) = opened(&missing_dump(), &dl);
        assert_eq!(status(&stored(&docs, id), &names[0]), SymbolStatus::Downloading);
        let settled = Mutex::new(Vec::new());
        let first = names[0].clone();
        let update = dl.run(
            &AtomicBool::new(false),
            &|done, total| settled.lock().unwrap().push((done, total)),
            || {
                // The second reading finds the downloaded PDB.
                let mut d = missing_dump();
                for m in d.modules.iter_mut().filter(|m| m.debug_file.as_deref() == Some(&first)) {
                    m.symbols = SymbolStatus::Loaded;
                }
                Ok(Document::Dump(d))
            },
        );
        assert!(update.arrived());
        assert_eq!(update.failure(), None, "one PDB arrived");
        let total = names.len();
        assert_eq!(*settled.lock().unwrap(), (1..=total).map(|n| (n, total)).collect::<Vec<_>>());
        assert!(update.apply(&docs, id, 120));
        let d = stored(&docs, id);
        assert_eq!(status(&d, &names[0]), SymbolStatus::Loaded);
        assert!(matches!(status(&d, &names[1]), SymbolStatus::DownloadFailed { detail } if detail.contains("timed out")));
        assert!(matches!(status(&d, &names[2]), SymbolStatus::Mismatch { detail } if detail.contains("another build")));
        for other in &names[3..] {
            assert_eq!(status(&d, other), SymbolStatus::NotOnServer, "{other}");
        }
        assert_eq!(fetcher.asked.load(Ordering::SeqCst), names.len());
    }

    #[test]
    fn a_download_that_failed_everywhere_is_a_failure_but_not_on_the_servers_is_not() {
        let names = missing_names(&missing_dump());
        let cache = tempfile::tempdir().unwrap();
        let broken = Arc::new(table(
            names
                .iter()
                .map(|n| (n.as_str(), Err(FetchError::Network("offline".into()))))
                .collect(),
        ));
        let dl = download(broken, &missing_dump(), cache.path());
        let (docs, id) = opened(&missing_dump(), &dl);
        let update = dl.run(&AtomicBool::new(false), &|_, _| {}, || panic!("nothing arrived"));
        assert!(!update.arrived());
        assert!(update.failure().unwrap().contains("offline"));
        assert!(update.apply(&docs, id, 120));
        assert!(matches!(status(&stored(&docs, id), &names[0]), SymbolStatus::DownloadFailed { .. }));
        // Not on any server is an answer, not a failure.
        let none = Arc::new(table(vec![]));
        let dl = download(none, &missing_dump(), cache.path());
        let update = dl.run(&AtomicBool::new(false), &|_, _| {}, || panic!("nothing arrived"));
        assert_eq!(update.failure(), None);
    }

    #[test]
    fn cancelling_stops_asking_and_puts_the_modules_back_to_missing() {
        let names = missing_names(&missing_dump());
        let fetcher = Arc::new(table(vec![]));
        let cache = tempfile::tempdir().unwrap();
        let dl = download(fetcher.clone(), &missing_dump(), cache.path());
        let (docs, id) = opened(&missing_dump(), &dl);
        let cancel = AtomicBool::new(false);
        // Cancel after the first PDB is settled.
        let update = dl.run(&cancel, &|_, _| cancel.store(true, Ordering::SeqCst), || panic!("nothing arrived"));
        assert_eq!(fetcher.asked.load(Ordering::SeqCst), 1);
        assert!(update.apply(&docs, id, 120));
        let d = stored(&docs, id);
        assert_eq!(status(&d, &names[0]), SymbolStatus::NotOnServer, "the one that was asked");
        for other in &names[1..] {
            assert_eq!(
                status(&d, other),
                SymbolStatus::Missing,
                "{other}: not asked, so nothing is claimed"
            );
        }
    }

    #[test]
    fn a_dump_that_cannot_be_read_again_keeps_its_first_reading_and_a_closed_one_is_left_alone() {
        let names = missing_names(&missing_dump());
        let cache = tempfile::tempdir().unwrap();
        let dl = download(Arc::new(table(vec![(&names[0], Ok(()))])), &missing_dump(), cache.path());
        let (docs, id) = opened(&missing_dump(), &dl);
        let update = dl.run(&AtomicBool::new(false), &|_, _| {}, || {
            Err(imt_dump::DumpError::new(imt_dump::ErrorCode::Invalid, "gone"))
        });
        assert!(update.arrived());
        assert!(update.apply(&docs, id, 120));
        let d = stored(&docs, id);
        // Downloaded, but not loaded: it stays missing rather than claiming symbols it does not have.
        assert_eq!(status(&d, &names[0]), SymbolStatus::Missing);
        assert_eq!(status(&d, &names[1]), SymbolStatus::NotOnServer);
        // The document was closed while the PDBs came: the late result changes nothing and does not fail.
        let (docs, id) = opened(&missing_dump(), &dl);
        assert!(crate::sync::write(&docs).close(id));
        assert!(!dl
            .run(&AtomicBool::new(false), &|_, _| {}, || Ok(Document::Dump(missing_dump())))
            .apply(&docs, id, 120));
        // A download that could not even start puts the modules back.
        let (docs, id) = opened(&missing_dump(), &dl);
        assert!(dl.abandoned().apply(&docs, id, 120));
        assert_eq!(status(&stored(&docs, id), &names[0]), SymbolStatus::Missing);
    }

    fn values(extra: serde_json::Value) -> Result<SymbolSettings, anyhow::Error> {
        let section = SymbolsSection {
            symbols: Arc::new(Symbols::new(None)),
        }
        .settings_section();
        let mut v = section.defaults().to_json();
        v.as_object_mut().unwrap().extend(extra.as_object().unwrap().clone());
        let valid = imt_core::settings::validate(&section, &v).map_err(|e| anyhow::anyhow!("{e:?}"))?;
        SymbolsSection::from_values(&valid)
    }

    #[test]
    fn downloads_are_off_by_default_and_a_bad_address_is_refused_when_saved() {
        let d = values(serde_json::json!({})).unwrap();
        assert!(!d.servers.enabled);
        assert!(d.servers.urls[0].starts_with("https://"));
        assert!(d.cache_folder.is_none());
        let on = values(serde_json::json!({ "serversEnabled": true, "concurrent": 4, "cacheFolder": "/tmp/syms" })).unwrap();
        assert!(on.servers.enabled);
        assert_eq!((on.servers.concurrent, on.cache_folder), (4, Some(PathBuf::from("/tmp/syms"))));
        assert!(values(serde_json::json!({ "serverUrls": ["http://insecure.example/s"] })).is_err());
        assert!(values(serde_json::json!({ "concurrent": 99 })).is_err());
    }

    #[test]
    fn a_bad_address_that_got_in_marks_the_modules_instead_of_downloading() {
        let symbols = Symbols::new(Some(tempfile::tempdir().unwrap().keep()));
        symbols.set(SymbolSettings {
            servers: ServersConfig {
                enabled: true,
                urls: vec!["http://insecure.example/s".into()],
                ..ServersConfig::default()
            },
            cache_folder: None,
        });
        let Opened::Final(doc) = symbols.begin(Document::Dump(missing_dump()), 120) else {
            panic!("a bad address is final");
        };
        let names = missing_names(&missing_dump());
        assert!(matches!(status(&dump_of(doc), &names[0]), SymbolStatus::DownloadFailed { .. }));
        // Without a cache folder nothing is asked at all.
        let none = Symbols::new(None);
        let Opened::Final(doc) = none.begin(Document::Dump(missing_dump()), 120) else {
            panic!("no cache")
        };
        assert_eq!(doc, Document::Dump(missing_dump()));
    }
}
