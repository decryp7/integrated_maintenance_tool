//! A third-party plugin from package to imported events, through IMT's real host and engine: the importer example
//! is packaged and signed, inspected (signature status, trust, permissions), installed under a plugins folder,
//! loaded as a contained process with its program verified by hash, and imports a file into a workspace. Also:
//! unsigned and unknown-key packages need developer mode, a package that changed after review is refused, a
//! program changed after install never starts, a plugin for a newer protocol is refused with what to update, and
//! remove/enable/grant flows.

use cargo_imt_plugin::{pack, PluginMeta};
use imt_core::{PluginRegistry, Profile, Workspace};
use imt_plugin_host::installed::PluginsDir;
use imt_plugin_host::Builtins;
use imt_plugin_package::archive::{build, Build};
use imt_plugin_package::manifest::host_target;
use imt_plugin_package::{Package, PackageManifest, SecretKey, SignatureStatus, Trust, TrustSettings};
use std::path::{Path, PathBuf};

const PROGRAM: &str = env!("CARGO_BIN_EXE_imt-plugin-e2e-kv");
const ID: &str = "net.decryptology.example.kv-lines";

fn examples() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins/importer-example")
}

fn package(dir: &Path, name: &str, key: Option<&SecretKey>) -> (PathBuf, String) {
    let meta = PluginMeta::read(&examples().join("Cargo.toml")).unwrap();
    let out = dir.join(name);
    let digest = pack(&meta, &[(host_target().to_string(), PROGRAM.into())], key, &out).unwrap();
    (out, digest)
}

fn trusting(key: &SecretKey) -> TrustSettings {
    let mut t = TrustSettings::default();
    t.add_key("IMT examples", &key.public().to_text()).unwrap();
    t
}

/// A workspace whose registry holds the installed plugins (as the app builds a window's).
fn workspace(dir: &PluginsDir) -> Workspace {
    let mut b = Builtins::none();
    for l in dir.load() {
        b.add_installed(l.process.unwrap()).unwrap();
    }
    let mut r = PluginRegistry::new();
    for p in b.plugins() {
        r.install_shared(p).unwrap();
    }
    r.resolve();
    Workspace::with_registry(Profile::default_profile(), r).unwrap()
}

#[test]
fn a_signed_package_installs_and_imports_a_file_through_the_real_host() {
    let t = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let (pkg, digest) = package(t.path(), "kv.imtplugin", Some(&key));
    let trust = trusting(&key);
    let dir = PluginsDir::new(t.path().join("plugins"));
    let seen = dir.inspect(&pkg, &trust).unwrap();
    assert_eq!(seen.signature, SignatureStatus::Trusted("IMT examples".into()));
    assert_eq!(
        seen.trust.clone().unwrap(),
        Trust::Trusted {
            publisher_key: "IMT examples".into()
        }
    );
    assert!(seen.installable().is_ok() && seen.installed.is_none());
    assert_eq!(seen.digest, digest);
    assert_eq!(seen.manifest.id, ID);
    let record = dir.install(&pkg, &trust, &digest, &[]).unwrap();
    assert_eq!(
        (record.id.as_str(), record.signature.as_str(), record.enabled),
        (ID, "trusted", true)
    );
    assert!(record.warning.is_none());

    // Loaded as IMT loads it: contained, verified, and importing a file through the engine.
    let mut ws = workspace(&dir);
    let file = t.path().join("app.log");
    std::fs::copy(examples().join("tests/samples/app.log"), &file).unwrap();
    ws.open(std::slice::from_ref(&file)).unwrap();
    let records = ws.records();
    assert_eq!(records.len(), 3, "the third-party importer read the file");
    assert_eq!(records[1].message, "disk almost full");
    assert!(
        ws.summary().sources[0].importer.contains("kv-lines"),
        "{}",
        ws.summary().sources[0].importer
    );
    let info = ws.summary().plugins.into_iter().find(|p| p.id == ID).unwrap();
    assert_eq!(info.failures, 0, "{info:?}");

    // Reinstalling the same version (now unsigned, in developer mode) is an update that keeps it enabled.
    dir.set_enabled(ID, false).unwrap();
    let (unsigned, udigest) = package(t.path(), "kv-unsigned.imtplugin", None);
    let dev = TrustSettings {
        developer_mode: true,
        ..TrustSettings::default()
    };
    let update = dir.inspect(&unsigned, &dev).unwrap();
    assert_eq!(update.installed.as_ref().map(|r| r.version.as_str()), Some("0.1.0"));
    assert!(update.added_permissions.is_empty() && update.removed_permissions.is_empty());
    let r = dir.install(&unsigned, &dev, &udigest, &[]).unwrap();
    assert!(!r.enabled, "an update keeps the user's choice");
    assert!(r.warning.unwrap().contains("not signed"));
    assert!(dir.load().is_empty(), "disabled plugins are not loaded");
    dir.set_enabled(ID, true).unwrap();
    assert_eq!(dir.load().len(), 1);
    drop(ws);
    dir.cleanup();
    let folders: Vec<_> = std::fs::read_dir(dir.root().join(ID))
        .unwrap()
        .flatten()
        .map(|e| e.file_name())
        .collect();
    assert_eq!(folders.len(), 3, "the current version, data and install.json: {folders:?}");

    // Removed: gone from the list and, after cleanup, from disk.
    assert!(dir.remove(ID).unwrap());
    assert!(dir.list().is_empty());
    dir.cleanup();
    assert!(!dir.root().join(ID).exists());
}

#[test]
fn trust_needs_a_trusted_key_or_developer_mode() {
    let t = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let (signed, sdigest) = package(t.path(), "signed.imtplugin", Some(&key));
    let (unsigned, udigest) = package(t.path(), "unsigned.imtplugin", None);
    let dir = PluginsDir::new(t.path().join("plugins"));
    let strict = TrustSettings::default();
    let e = dir.install(&unsigned, &strict, &udigest, &[]).unwrap_err();
    assert_eq!(e.code(), "unsigned", "{e}");
    let e = dir.install(&signed, &strict, &sdigest, &[]).unwrap_err();
    assert_eq!(e.code(), "unknownKey", "{e}");
    assert!(e.to_string().contains(&key.public().id()));
    assert_eq!(
        dir.inspect(&signed, &strict).unwrap().signature,
        SignatureStatus::UnknownKey(key.public().id())
    );
    let dev = TrustSettings {
        developer_mode: true,
        ..TrustSettings::default()
    };
    let r = dir.install(&signed, &dev, &sdigest, &[]).unwrap();
    assert_eq!(r.signature, "unknownKey");
    assert!(r.warning.unwrap().contains(&key.public().id()));
    // What the user reviewed must be what is installed.
    let e = dir.install(&signed, &dev, &udigest, &[]).unwrap_err();
    assert_eq!(e.code(), "tampered");
}

#[test]
fn a_program_changed_after_install_never_starts() {
    let t = tempfile::tempdir().unwrap();
    let key = SecretKey::generate().unwrap();
    let (pkg, digest) = package(t.path(), "kv.imtplugin", Some(&key));
    let dir = PluginsDir::new(t.path().join("plugins"));
    let r = dir.install(&pkg, &trusting(&key), &digest, &[]).unwrap();
    let exe = dir.home(&r).unwrap().program_dir.join(&r.program);
    let mut bytes = std::fs::read(&exe).unwrap();
    bytes.extend_from_slice(b"appended");
    std::fs::write(&exe, bytes).unwrap();
    let mut ws = workspace(&dir);
    let file = t.path().join("app.log");
    std::fs::copy(examples().join("tests/samples/app.log"), &file).unwrap();
    let _ = ws.open(std::slice::from_ref(&file));
    assert!(
        ws.records().iter().all(|r| r.message != "disk almost full"),
        "the changed program did not import"
    );
    let info = ws.summary().plugins.into_iter().find(|p| p.id == ID).unwrap();
    assert!(info.last_error.unwrap_or_default().contains("changed after it was installed"));
}

#[test]
fn a_package_for_a_newer_protocol_or_another_computer_is_refused_with_what_to_update() {
    let t = tempfile::tempdir().unwrap();
    let prog = t.path().join("p");
    std::fs::write(&prog, b"program").unwrap();
    let mut m = PackageManifest::parse(&format!(
        "format = 1\n[plugin]\nid = \"com.example.future\"\nname = \"F\"\nversion = \"1.0.0\"\npublisher = \"P\"\nprotocol = \"{}.0\"\nbinary = \"f\"\ntargets = [\"{}\"]\n",
        imt_plugin_api::PROTOCOL.major + 1,
        host_target()
    ))
    .unwrap();
    let out = t.path().join("future.imtplugin");
    build(
        Build {
            manifest: m.clone(),
            programs: vec![(host_target().into(), &prog)],
            sign: None,
        },
        &out,
    )
    .unwrap();
    let dev = TrustSettings {
        developer_mode: true,
        ..TrustSettings::default()
    };
    let dir = PluginsDir::new(t.path().join("plugins"));
    let seen = dir.inspect(&out, &dev).unwrap();
    let e = seen.installable().unwrap_err();
    assert_eq!(e.code(), "incompatible");
    assert!(e.to_string().contains("needs a newer IMT"), "{e}");
    assert!(dir.install(&out, &dev, &seen.digest, &[]).is_err());
    // No program for this computer.
    m.protocol = imt_plugin_api::PROTOCOL;
    m.targets = vec!["riscv64gc-unknown-linux-gnu".into()];
    let other = t.path().join("other.imtplugin");
    build(
        Build {
            manifest: m,
            programs: vec![("riscv64gc-unknown-linux-gnu".into(), &prog)],
            sign: None,
        },
        &other,
    )
    .unwrap();
    assert_eq!(dir.inspect(&other, &dev).unwrap().installable().unwrap_err().code(), "wrongTarget");
    assert!(Package::open(&other).is_ok());
}
