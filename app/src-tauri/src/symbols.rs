//! Symbols for dumps (ADR 0036): Settings ▸ Symbols and the host's part of naming a dump's frames.
//!
//! A dump is opened by the dumps plugin, which names frames from the symbol folders (the user's, then IMT's symbol
//! cache) by PDB identity (name, GUID, age) and records each module's [`SymbolStatus`]. The modules whose PDB is
//! missing are then asked of the user's symbol servers here, into the cache, and the dump is read again with the
//! new PDBs. Downloads are off until the user turns them on; nothing is requested while they are off.

use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use imt_dump::model::{Document, SymbolStatus};
use imt_symbols::fetch::{FetchError, Fetcher};
use imt_symbols::id::PdbKey;
use imt_symbols::servers::{get_all, Gate, Outcome, ServersConfig, MAX_CONCURRENT, MAX_SERVERS};
use std::path::{Path, PathBuf};
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

    /// The dump with the PDBs of its missing modules fetched from the symbol servers (when the user turned them on),
    /// and every module's status updated. `reimport` reads the dump again once PDBs were added to the cache.
    pub fn complete(&self, doc: Document, max_text: usize, reimport: impl FnOnce() -> imt_dump::Result<Document>) -> Document {
        let Some(cache) = self.cache_dir() else { return doc };
        let settings = self.get();
        let gate = crate::sync::read(&self.gate).clone();
        match settings.servers.servers() {
            Ok(servers) => fill_missing(
                doc,
                &cache,
                servers.as_ref().map(|s| s as &dyn Fetcher),
                &gate,
                max_text,
                reimport,
                &settings.servers,
            ),
            Err(e) => {
                let detail = e.to_string();
                let mut doc = doc;
                if let Document::Dump(d) = &mut doc {
                    imt_dump::symbols::mark_missing(d, |_| Some(SymbolStatus::DownloadFailed { detail: detail.clone() }), max_text);
                }
                doc
            }
        }
    }
}

/// What a download outcome says about the module's symbols (`None`: nothing new to say).
fn status_of(outcome: &Outcome) -> Option<SymbolStatus> {
    match outcome {
        Outcome::Cached(_) | Outcome::Downloaded(_) => None,
        Outcome::Disabled => Some(SymbolStatus::ServersDisabled),
        Outcome::Failed(FetchError::NotFound(_)) => Some(SymbolStatus::NotOnServer),
        Outcome::Failed(FetchError::Mismatch(m)) => Some(SymbolStatus::Mismatch {
            detail: format!("the symbol server's {m}"),
        }),
        Outcome::Failed(e) => Some(SymbolStatus::DownloadFailed { detail: e.to_string() }),
    }
}

/// Gets the missing PDBs of a dump into `cache` through `fetcher` (`None`: downloads are off), reads the dump again
/// when any arrived, and records what became of the others.
fn fill_missing(
    doc: Document,
    cache: &Path,
    fetcher: Option<&dyn Fetcher>,
    gate: &Gate,
    max_text: usize,
    reimport: impl FnOnce() -> imt_dump::Result<Document>,
    config: &ServersConfig,
) -> Document {
    let Document::Dump(dump) = &doc else { return doc };
    let keys = imt_dump::symbols::missing(dump, MAX_DOWNLOADS);
    if keys.is_empty() {
        return doc;
    }
    let outcomes: Vec<(PdbKey, Outcome)> = get_all(fetcher, &keys, cache, gate);
    let mut doc = doc;
    if outcomes.iter().any(|(_, o)| matches!(o, Outcome::Downloaded(_))) {
        imt_symbols::cache::prune(cache, config.cache_max_bytes);
        // A dump that cannot be read again keeps the first reading, with its PDBs still marked missing.
        if let Ok(again) = reimport() {
            doc = again;
        }
    }
    if let Document::Dump(d) = &mut doc {
        imt_dump::symbols::mark_missing(
            d,
            |k| outcomes.iter().find(|(key, _)| key == k).and_then(|(_, o)| status_of(o)),
            max_text,
        );
    }
    doc
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
    use std::sync::atomic::{AtomicUsize, Ordering};
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

    fn run(fetcher: Option<&dyn Fetcher>, reimport: impl FnOnce() -> imt_dump::Result<Document>) -> Dump {
        let cache = tempfile::tempdir().unwrap();
        dump_of(fill_missing(
            Document::Dump(missing_dump()),
            cache.path(),
            fetcher,
            &Gate::new(2),
            120,
            reimport,
            &ServersConfig::default(),
        ))
    }

    #[test]
    fn with_downloads_off_every_missing_module_says_so_and_nothing_is_read_again() {
        let first = missing_names(&missing_dump());
        assert!(!first.is_empty());
        let d = run(None, || panic!("nothing was downloaded"));
        for pdb in &first {
            assert_eq!(status(&d, pdb), SymbolStatus::ServersDisabled, "{pdb}");
        }
        assert!(missing_names(&d).is_empty());
    }

    #[test]
    fn a_failed_download_says_why_and_a_downloaded_one_reads_the_dump_again() {
        let names = missing_names(&missing_dump());
        assert!(names.len() >= 3, "{names:?}");
        let fetcher = table(vec![
            (&names[0], Ok(())),
            (&names[1], Err(FetchError::Network("timed out".into()))),
            (&names[2], Err(FetchError::Mismatch("answer is another build".into()))),
        ]);
        let again = AtomicUsize::new(0);
        let first = names[0].clone();
        let d = run(Some(&fetcher), || {
            again.fetch_add(1, Ordering::SeqCst);
            // The second reading finds the downloaded PDB.
            let mut d = missing_dump();
            for m in d.modules.iter_mut().filter(|m| m.debug_file.as_deref() == Some(&first)) {
                m.symbols = SymbolStatus::Loaded;
            }
            Ok(Document::Dump(d))
        });
        assert_eq!(again.load(Ordering::SeqCst), 1);
        assert_eq!(status(&d, &names[0]), SymbolStatus::Loaded);
        assert!(matches!(status(&d, &names[1]), SymbolStatus::DownloadFailed { detail } if detail.contains("timed out")));
        assert!(matches!(status(&d, &names[2]), SymbolStatus::Mismatch { detail } if detail.contains("another build")));
        for other in &names[3..] {
            assert_eq!(status(&d, other), SymbolStatus::NotOnServer, "{other}");
        }
        assert_eq!(fetcher.asked.load(Ordering::SeqCst), names.len());
    }

    #[test]
    fn a_dump_that_cannot_be_read_again_keeps_its_first_reading() {
        let names = missing_names(&missing_dump());
        let fetcher = table(vec![(&names[0], Ok(()))]);
        let d = run(Some(&fetcher), || {
            Err(imt_dump::DumpError::new(imt_dump::ErrorCode::Invalid, "gone"))
        });
        // Downloaded, but not loaded: it stays missing rather than claiming symbols it does not have.
        assert_eq!(status(&d, &names[0]), SymbolStatus::Missing);
        assert_eq!(status(&d, &names[1]), SymbolStatus::NotOnServer);
    }

    #[test]
    fn umdh_logs_and_dumps_with_nothing_missing_are_left_alone() {
        let cache = tempfile::tempdir().unwrap();
        let umdh = Document::Umdh(imt_dump::testing::sample_umdh(imt_dump::model::UmdhKind::Diff, 0, &[(1, 2, 3)]));
        let out = fill_missing(
            umdh.clone(),
            cache.path(),
            None,
            &Gate::new(1),
            100,
            || panic!("no"),
            &ServersConfig::default(),
        );
        assert_eq!(out, umdh);
        let all = sample_dump();
        let out = fill_missing(
            Document::Dump(all.clone()),
            cache.path(),
            None,
            &Gate::new(1),
            100,
            || panic!("no"),
            &ServersConfig::default(),
        );
        assert_eq!(out, Document::Dump(all));
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
        let d = dump_of(symbols.complete(Document::Dump(missing_dump()), 120, || panic!("no download happened")));
        let names = missing_names(&missing_dump());
        assert!(matches!(status(&d, &names[0]), SymbolStatus::DownloadFailed { .. }));
        // Without a cache folder nothing is asked at all.
        let none = Symbols::new(None);
        assert_eq!(
            none.complete(Document::Dump(missing_dump()), 120, || panic!("no")),
            Document::Dump(missing_dump())
        );
    }
}
