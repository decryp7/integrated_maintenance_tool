//! One way to keep configuration and state files (ADR 0016): `settings.json`, `diagnostics.json`,
//! `plugins.json`, `recent.json`, `notifications.json` and `updates.json` are read and written here.
//!
//! - **Versioned**: a document is `{ "version": N, ... }` ([`Versioned`]); a file of an older version is read
//!   when its fields still fit (pre-v1: nothing is migrated), a newer, damaged or oversized one gives the
//!   defaults and is kept aside as `<name>.corrupt` (a damaged one) so nothing is lost silently.
//! - **Atomic**: written to a temporary file, flushed, then renamed over the old one.
//! - **One writer**: writes are serialised process-wide, so two features never interleave a file.
//! - **Cached** ([`Config`]): a document read once and changed in memory, persisted after each change
//!   outside its lock; [`Debounced`] coalesces frequent writes (notifications) on a thread of its own.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::{Arc, Mutex, RwLock};
use std::time::Duration;

/// Largest configuration file read (a bigger one is not a configuration file).
pub const MAX_FILE_BYTES: u64 = 4 * 1024 * 1024;

/// Serialises every write (one writer).
static WRITER: Mutex<()> = Mutex::new(());

/// A document with its format version.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Versioned<T> {
    #[serde(default)]
    pub version: u32,
    #[serde(flatten)]
    pub data: T,
}

/// Why a file was not read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReadError {
    /// It does not exist.
    Missing,
    /// It is not a file, too large, or not the expected JSON (kept aside as `.corrupt`).
    Damaged(String),
    /// It was written by a newer IMT (left as it is).
    Newer(u32),
}

/// A folder of configuration files (none: everything lives in memory).
#[derive(Debug, Clone, Default)]
pub struct ConfigStore {
    dir: Option<PathBuf>,
}

impl ConfigStore {
    pub fn new(dir: Option<PathBuf>) -> ConfigStore {
        ConfigStore { dir }
    }

    pub fn path(&self, name: &str) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(name))
    }

    /// The JSON value of file `name`.
    pub fn read_value(&self, name: &str) -> Result<serde_json::Value, ReadError> {
        let path = self.path(name).ok_or(ReadError::Missing)?;
        read_json(&path)
    }

    /// Document `name` of format `version` (or older).
    pub fn read<T: DeserializeOwned>(&self, name: &str, version: u32) -> Result<T, ReadError> {
        let value = self.read_value(name)?;
        let found = value.get("version").and_then(serde_json::Value::as_u64).unwrap_or(0) as u32;
        if found > version {
            return Err(ReadError::Newer(found));
        }
        serde_json::from_value::<Versioned<T>>(value)
            .map(|v| v.data)
            .map_err(|e| ReadError::Damaged(e.to_string()))
    }

    /// Document `name`, or the defaults when it is missing, newer or damaged (a damaged file is kept aside
    /// as `<name>.corrupt`; both are logged).
    pub fn load<T: DeserializeOwned + Default>(&self, name: &str, version: u32) -> T {
        match self.read(name, version) {
            Ok(t) => t,
            Err(ReadError::Missing) => T::default(),
            Err(ReadError::Newer(v)) => {
                log::warn!("{name} was written by a newer IMT (format {v}); defaults are used");
                T::default()
            }
            Err(ReadError::Damaged(why)) => {
                log::warn!("{name} could not be read ({why}); defaults are used and the file is kept as {name}.corrupt");
                if let Some(p) = self.path(name) {
                    let _ = std::fs::rename(&p, p.with_extension("json.corrupt"));
                }
                T::default()
            }
        }
    }

    /// A document that carries its own `version` field (no envelope): read as is; the caller checks the version.
    /// Missing or damaged: the defaults (a damaged file is kept aside, as in [`ConfigStore::load`]).
    pub fn load_doc<T: DeserializeOwned + Default>(&self, name: &str) -> T {
        match self
            .read_value(name)
            .and_then(|v| serde_json::from_value(v).map_err(|e| ReadError::Damaged(e.to_string())))
        {
            Ok(t) => t,
            Err(ReadError::Damaged(why)) => {
                log::warn!("{name} could not be read ({why}); defaults are used and the file is kept as {name}.corrupt");
                if let Some(p) = self.path(name) {
                    let _ = std::fs::rename(&p, p.with_extension("json.corrupt"));
                }
                T::default()
            }
            Err(_) => T::default(),
        }
    }

    /// Writes a document that carries its own `version` field, atomically.
    pub fn save_doc<T: Serialize>(&self, name: &str, doc: &T) -> std::io::Result<()> {
        let Some(path) = self.path(name) else { return Ok(()) };
        write_atomic(&path, &serde_json::to_vec_pretty(doc).map_err(std::io::Error::other)?)
    }

    /// Writes document `name` as format `version`, atomically. Without a folder nothing is written.
    pub fn save<T: Serialize>(&self, name: &str, version: u32, data: &T) -> std::io::Result<()> {
        let Some(path) = self.path(name) else { return Ok(()) };
        let doc = Versioned { version, data };
        let text = serde_json::to_vec_pretty(&doc).map_err(std::io::Error::other)?;
        write_atomic(&path, &text)
    }
}

/// Reads a JSON file of at most [`MAX_FILE_BYTES`].
pub fn read_json(path: &Path) -> Result<serde_json::Value, ReadError> {
    let meta = match std::fs::metadata(path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Err(ReadError::Missing),
        Err(e) => return Err(ReadError::Damaged(e.to_string())),
    };
    if !meta.is_file() {
        return Err(ReadError::Damaged("not a file".into()));
    }
    if meta.len() > MAX_FILE_BYTES {
        return Err(ReadError::Damaged(format!("larger than {} KB", MAX_FILE_BYTES / 1024)));
    }
    let text = std::fs::read_to_string(path).map_err(|e| ReadError::Damaged(e.to_string()))?;
    serde_json::from_str(&text).map_err(|e| ReadError::Damaged(e.to_string()))
}

/// Writes `bytes` to `path` atomically (a temporary file next to it, flushed, then renamed), one writer at
/// a time; creates the folder.
pub fn write_atomic(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let _one = crate::sync::lock(&WRITER);
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir)?;
    }
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let tmp = path.with_file_name(format!(".{name}.{}.tmp", std::process::id()));
    let result = (|| {
        let mut f = std::fs::File::create(&tmp)?;
        f.write_all(bytes)?;
        f.sync_all()?;
        drop(f);
        std::fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    result
}

/// A cached document: read once, changed in memory, persisted after each change (outside its lock).
pub struct Config<T> {
    store: ConfigStore,
    name: &'static str,
    version: u32,
    value: RwLock<T>,
}

impl<T: Clone + Serialize + DeserializeOwned + Default> Config<T> {
    /// Reads document `name` from `store` (defaults when missing or unreadable).
    pub fn open(store: ConfigStore, name: &'static str, version: u32) -> Config<T> {
        let value = RwLock::new(store.load(name, version));
        Config {
            store,
            name,
            version,
            value,
        }
    }

    /// The same document with `fix` applied to what was read (bounds, dropped entries); not written.
    pub fn fixed(self, fix: impl FnOnce(T) -> T) -> Config<T> {
        let value = fix(self.value.into_inner().unwrap_or_else(|p| p.into_inner()));
        Config {
            value: RwLock::new(value),
            ..self
        }
    }

    pub fn get(&self) -> T {
        crate::sync::read(&self.value).clone()
    }

    /// Changes the document; when `f` says it changed something, writes it (after the cache lock is released)
    /// and says so.
    pub fn update_if(&self, f: impl FnOnce(&mut T) -> bool) -> std::io::Result<bool> {
        let next = {
            let mut v = crate::sync::write(&self.value);
            if !f(&mut v) {
                return Ok(false);
            }
            v.clone()
        };
        self.store.save(self.name, self.version, &next)?;
        Ok(true)
    }
}

/// What the writer thread hears.
enum Msg<T> {
    Write(T),
    /// Write what is pending now, then answer.
    Flush(mpsc::Sender<()>),
}

/// Writes the latest of many snapshots, at most once per `delay`, on a thread of its own (callers never
/// wait for the disk, and never write under their own locks).
pub struct Debounced<T: Send + 'static> {
    tx: Mutex<Option<mpsc::Sender<Msg<T>>>>,
}

impl<T: Send + 'static> Debounced<T> {
    /// `write` runs on the writer thread with the newest snapshot of each burst.
    pub fn new(delay: Duration, write: impl Fn(T) + Send + 'static) -> Arc<Debounced<T>> {
        let (tx, rx) = mpsc::channel::<Msg<T>>();
        let spawned = std::thread::Builder::new().name("imt-config-writer".into()).spawn(move || {
            let mut pending: Option<T> = None;
            loop {
                let msg = if pending.is_some() {
                    match rx.recv_timeout(delay) {
                        Ok(m) => m,
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if let Some(t) = pending.take() {
                                write(t);
                            }
                            continue;
                        }
                        Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    }
                } else {
                    match rx.recv() {
                        Ok(m) => m,
                        Err(_) => break,
                    }
                };
                match msg {
                    Msg::Write(t) => pending = Some(t),
                    Msg::Flush(ack) => {
                        if let Some(t) = pending.take() {
                            write(t);
                        }
                        let _ = ack.send(());
                    }
                }
            }
            if let Some(t) = pending.take() {
                write(t);
            }
        });
        if let Err(e) = spawned {
            log::warn!("the configuration writer could not start: {e}");
        }
        Arc::new(Debounced { tx: Mutex::new(Some(tx)) })
    }

    /// Queues a snapshot to write.
    pub fn schedule(&self, snapshot: T) {
        if let Some(tx) = crate::sync::lock(&self.tx).as_ref() {
            let _ = tx.send(Msg::Write(snapshot));
        }
    }

    /// Writes what is pending now and waits for it (at most `limit`): before exit, and in tests.
    pub fn flush(&self, limit: Duration) {
        let (ack, done) = mpsc::channel();
        let sent = self
            .tx
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .is_some_and(|tx| tx.send(Msg::Flush(ack)).is_ok());
        if sent {
            let _ = done.recv_timeout(limit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::TempDir;

    #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
    struct Doc {
        #[serde(default)]
        items: Vec<String>,
    }

    #[test]
    fn documents_are_versioned_atomic_and_damage_is_kept_aside() {
        let t = TempDir::new("config-store");
        let store = ConfigStore::new(Some(t.0.clone()));
        assert_eq!(store.read::<Doc>("a.json", 1), Err(ReadError::Missing));
        let doc = Doc { items: vec!["x".into()] };
        store.save("a.json", 1, &doc).unwrap();
        assert_eq!(store.read::<Doc>("a.json", 1).unwrap(), doc);
        assert_eq!(store.read_value("a.json").unwrap()["version"], 1);
        // An older file (no version) still reads when its fields fit; a newer one does not.
        std::fs::write(t.0.join("old.json"), r#"{"items":["o"]}"#).unwrap();
        assert_eq!(store.load::<Doc>("old.json", 2).items, ["o"]);
        std::fs::write(t.0.join("new.json"), r#"{"version":9,"items":["n"]}"#).unwrap();
        assert_eq!(store.read::<Doc>("new.json", 2), Err(ReadError::Newer(9)));
        assert_eq!(store.load::<Doc>("new.json", 2), Doc::default());
        assert!(t.0.join("new.json").exists(), "a newer file is left alone");
        // Damaged: defaults, and the file is kept aside.
        std::fs::write(t.0.join("bad.json"), "{ nope").unwrap();
        assert_eq!(store.load::<Doc>("bad.json", 1), Doc::default());
        assert!(t.0.join("bad.json.corrupt").exists() && !t.0.join("bad.json").exists());
        std::fs::write(t.0.join("big.json"), vec![b' '; (MAX_FILE_BYTES + 1) as usize]).unwrap();
        assert!(matches!(store.read_value("big.json"), Err(ReadError::Damaged(_))));
        std::fs::create_dir_all(t.0.join("dir.json")).unwrap();
        assert!(matches!(store.read_value("dir.json"), Err(ReadError::Damaged(_))));
        // No temporary files are left, and without a folder nothing is written.
        assert!(std::fs::read_dir(&t.0)
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".tmp")));
        ConfigStore::new(None).save("a.json", 1, &doc).unwrap();
        assert_eq!(ConfigStore::new(None).load::<Doc>("a.json", 1), Doc::default());
        assert!(write_atomic(&t.0.join("dir.json"), b"x").is_err(), "a folder is not replaced");
    }

    #[test]
    fn cached_documents_persist_each_change_and_debounced_writes_coalesce() {
        let t = TempDir::new("config-cached");
        let store = ConfigStore::new(Some(t.0.clone()));
        let config: Config<Doc> = Config::open(store.clone(), "c.json", 1);
        assert_eq!(config.get(), Doc::default());
        assert!(config
            .update_if(|d| {
                d.items.push("a".into());
                true
            })
            .unwrap());
        assert!(!config.update_if(|_| false).unwrap());
        assert_eq!(Config::<Doc>::open(store, "c.json", 1).get().items, ["a"]);

        let written = Arc::new(Mutex::new(Vec::new()));
        let w = written.clone();
        let debounced = Debounced::new(Duration::from_millis(50), move |n: u32| w.lock().unwrap().push(n));
        for n in 1..=5 {
            debounced.schedule(n);
        }
        debounced.flush(Duration::from_secs(10));
        assert_eq!(*written.lock().unwrap(), [5], "one write with the newest snapshot");
        debounced.schedule(6);
        let start = std::time::Instant::now();
        while written.lock().unwrap().len() < 2 && start.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(*written.lock().unwrap(), [5, 6], "written after the delay without a flush");
    }
}
