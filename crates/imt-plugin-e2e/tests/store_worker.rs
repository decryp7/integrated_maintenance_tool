//! An installed third-party importer takes part in imports through the store worker (ADR 0027 with ADR 0014 6c):
//! the importer example is packaged, signed and installed; IMT's store worker program is told about it, starts it
//! contained from its install record and reads a file with it into the DuckDB store. A plugin the app did not load
//! as installed (its program changed since, or not named) does not read anything.

use cargo_imt_plugin::{pack, PluginMeta};
use imt_core::ports::{Ingest, PageQuery, Query};
use imt_core::Profile;
use imt_plugin_host::installed::{InstallRecord, PluginsDir};
use imt_plugin_package::manifest::host_target;
use imt_plugin_package::{SecretKey, TrustSettings};
use imt_store_worker::protocol::ThirdPartyPlugin;
use imt_store_worker::{RemoteStore, StoreWorker, WorkerConfig};
use std::path::{Path, PathBuf};

const PROGRAM: &str = env!("CARGO_BIN_EXE_imt-plugin-e2e-kv");
const WORKER: &str = env!("CARGO_BIN_EXE_imt-plugin-e2e-store-worker");
const ID: &str = "net.decryptology.example.kv-lines";

fn example() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/importer-example")
}

/// The importer example installed under `root/plugins`, signed by a trusted key.
fn installed(root: &Path) -> (PluginsDir, InstallRecord) {
    let key = SecretKey::generate().unwrap();
    let meta = PluginMeta::read(&example().join("Cargo.toml")).unwrap();
    let pkg = root.join("kv.imtplugin");
    let digest = pack(&meta, &[(host_target().to_string(), PROGRAM.into())], Some(&key), &pkg).unwrap();
    let mut trust = TrustSettings::default();
    trust.add_key("IMT examples", &key.public().to_text()).unwrap();
    let dir = PluginsDir::new(root.join("plugins"));
    let record = dir.install(&pkg, &trust, &digest, &[]).unwrap();
    assert_eq!(record.id, ID);
    (dir, record)
}

fn spec(dir: &PluginsDir, r: &InstallRecord) -> ThirdPartyPlugin {
    ThirdPartyPlugin {
        plugins_dir: dir.root().to_path_buf(),
        id: r.id.clone(),
        sha256: r.sha256.clone(),
        granted: r.granted.clone(),
    }
}

/// Imports the example's log into a new store of a worker that knows `third_party`.
fn import(root: &Path, name: &str, third_party: Vec<ThirdPartyPlugin>) -> (StoreWorker, RemoteStore) {
    let mut cfg = WorkerConfig::new(WORKER);
    cfg.third_party = third_party;
    let w = StoreWorker::new(cfg).unwrap();
    let store = w.open_store(&root.join(name), Profile::default_profile(), true, None).unwrap();
    let file = root.join(format!("{name}.log"));
    std::fs::copy(example().join("tests/samples/app.log"), &file).unwrap();
    store.open(std::slice::from_ref(&file)).unwrap().wait().unwrap();
    (w, store)
}

fn messages(store: &RemoteStore) -> Vec<String> {
    let page = store.page(&PageQuery::default()).unwrap();
    page.rows.into_iter().map(|r| r.message).collect()
}

#[test]
fn an_installed_importer_reads_files_in_the_store_worker() {
    let t = tempfile::tempdir().unwrap();
    let (dir, record) = installed(t.path());
    let (w, store) = import(t.path(), "with", vec![spec(&dir, &record)]);
    let summary = store.summary().unwrap();
    assert_eq!(summary.total, 3, "the third-party importer read the file in the worker");
    assert!(summary.sources[0].importer.contains("kv-lines"), "{}", summary.sources[0].importer);
    assert!(messages(&store).iter().any(|m| m == "disk almost full"));

    // The app's list reaches the running worker (unchanged here: the example asks for no permission, so its
    // process is kept); the next import still reads with it.
    w.set_third_party(vec![spec(&dir, &record)]).unwrap();
    let file = t.path().join("again.log");
    std::fs::copy(example().join("tests/samples/app.log"), &file).unwrap();
    store.open(std::slice::from_ref(&file)).unwrap().wait().unwrap();
    assert_eq!(store.summary().unwrap().total, 6);
    w.shutdown();
}

#[test]
fn a_plugin_the_app_did_not_load_reads_nothing_in_the_worker() {
    let t = tempfile::tempdir().unwrap();
    let (dir, record) = installed(t.path());
    // Not named by the app: the file is not read by it.
    let (w, store) = import(t.path(), "unnamed", Vec::new());
    assert!(store.summary().unwrap().sources.iter().all(|s| !s.importer.contains("kv-lines")));
    w.shutdown();
    // Updated after the app started (another program hash): refused, the file is not read by it.
    let stale = ThirdPartyPlugin {
        sha256: "0".repeat(64),
        ..spec(&dir, &record)
    };
    let (w, store) = import(t.path(), "stale", vec![stale]);
    assert!(store.summary().unwrap().sources.iter().all(|s| !s.importer.contains("kv-lines")));
    w.shutdown();
}
