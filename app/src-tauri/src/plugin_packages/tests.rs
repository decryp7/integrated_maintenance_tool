//! The shell edge of third-party plugins: argument checks, the trust section, errors with their reasons, the state
//! an installed plugin shows, and the commands against an install folder (installing itself is tested end to end
//! in `imt-plugin-e2e`, which can build a plugin program).

use super::commands::checks::{check_digest, check_id, permissions};
use super::commands::{installed_dto, list};
use super::dto::*;
use super::trust_section::parse_keys;
use super::{Installed, TrustSection};
use crate::ErrorCode;
use imt_core::settings::Configurable;
use imt_plugin_api::manifest::{Permission, PermissionRequest};
use imt_plugin_host::installed::{InstallRecord, PluginsDir, RECORD_FILE, RECORD_FORMAT};
use imt_plugin_package::{PackageError, SecretKey};
use std::sync::Arc;

fn record(id: &str) -> InstallRecord {
    InstallRecord {
        format: RECORD_FORMAT,
        id: id.into(),
        version: "1.0.0".into(),
        name: "Kv".into(),
        publisher: "Example".into(),
        description: "Reads kv.".into(),
        signature: "trusted".into(),
        signer: Some("Example".into()),
        warning: None,
        digest: "a".repeat(64),
        folder: "v-1.0.0-aaaaaaaa".into(),
        program: "kv.exe".into(),
        sha256: "0".repeat(64),
        requested: vec![PermissionRequest {
            permission: Permission::Network,
            reason: "Looks up hosts.".into(),
        }],
        granted: vec![Permission::Network],
        enabled: true,
        installed_at_ms: 1,
    }
}

#[test]
fn arguments_are_checked() {
    assert!(check_id("com.example.kv").is_ok());
    assert_eq!(check_id("../x").unwrap_err().code, ErrorCode::Invalid);
    assert!(check_digest(&"0f".repeat(32)).is_ok());
    assert!(check_digest("abc").is_err() && check_digest(&"zz".repeat(32)).is_err());
    let p = permissions(&[PermissionDto::Network, PermissionDto::Network, PermissionDto::RunPrograms]).unwrap();
    assert_eq!(p, [Permission::Network, Permission::RunPrograms]);
    assert!(permissions(&[PermissionDto::Network; 9]).is_err());
}

#[test]
fn trusted_keys_are_parsed_and_bad_lines_named() {
    let key = SecretKey::generate().unwrap().public();
    let t = parse_keys(&[format!("Example Ltd = {}", key.to_text()), "  ".into()], true).unwrap();
    assert!(t.developer_mode);
    assert_eq!(t.keys.len(), 1);
    assert_eq!(t.keys[0].name, "Example Ltd");
    let e = parse_keys(&["no equals sign".into()], false).unwrap_err();
    assert!(e.contains("trusted key 1"), "{e}");
    let e = parse_keys(&["x".into(), "Bad = not-a-key".into()], false).unwrap_err();
    assert!(e.contains("trusted key 1"), "{e}");
    let e = parse_keys(&[format!("A = {}", key.to_text()), format!("B = {}", key.to_text())], false).unwrap_err();
    assert!(e.contains("trusted key 2") && e.contains("already trusted"), "{e}");
    // The section writes what it parsed into the shared settings.
    let trust = Arc::default();
    let section = TrustSection { trust: Arc::clone(&trust) };
    let s = section.settings_section();
    s.check().unwrap();
    let values = imt_core::settings::validate(
        &s,
        &serde_json::json!({ "developerMode": true, "trustedKeys": [format!("Me = {}", key.to_text())] }),
    )
    .unwrap();
    section.apply_settings(&values).unwrap();
    assert!(crate::sync::read(&trust).developer_mode);
    assert_eq!(crate::sync::read(&trust).keys.len(), 1);
    let bad = imt_core::settings::validate(&s, &serde_json::json!({ "trustedKeys": ["Me = nope"] })).unwrap();
    assert!(section.apply_settings(&bad).is_err());
}

#[test]
fn package_errors_keep_their_reason() {
    let cases = [
        (PackageError::Unsigned, ErrorCode::Untrusted, "unsigned"),
        (PackageError::UnknownKey { key_id: "k".into() }, ErrorCode::Untrusted, "unknownKey"),
        (PackageError::Tampered("x".into()), ErrorCode::SignatureInvalid, "tampered"),
        (PackageError::Incompatible("x".into()), ErrorCode::Incompatible, "incompatible"),
        (
            PackageError::WrongTarget {
                target: "a".into(),
                supported: "b".into(),
            },
            ErrorCode::Unsupported,
            "wrongTarget",
        ),
        (PackageError::UnlistedFile("x".into()), ErrorCode::InvalidManifest, "unlistedFile"),
        (PackageError::TooLarge { what: "x".into(), limit: 1 }, ErrorCode::TooLarge, "tooLarge"),
        (PackageError::Io("x".into()), ErrorCode::Io, "io"),
        (PackageError::InvalidKey("x".into()), ErrorCode::Invalid, "invalidKey"),
    ];
    for (e, code, reason) in cases {
        let c = cmd_error(e);
        assert_eq!((c.code, c.detail.as_deref()), (code, Some(reason)));
    }
    assert!(cmd_error(PackageError::Unsigned).hint.unwrap().contains("developer mode"));
    assert_eq!(RefusalDto::from(&PackageError::Unsigned).reason, "unsigned");
}

#[test]
fn an_installed_plugin_shows_its_state_in_this_run() {
    let r = record("com.example.kv");
    let dto = installed_dto(&r, None, false);
    assert_eq!(dto.state, InstalledState::PendingRestart, "installed since the start");
    assert_eq!(dto.granted, [PermissionDto::Network]);
    assert_eq!(dto.requested[0].allows, "connect to the network");
    assert_eq!(dto.signature.kind, SignatureKind::Trusted);
    let loaded = (r.clone(), Err::<Arc<imt_plugin_host::PluginProcess>, _>("its program is missing".to_string()));
    let dto = installed_dto(&r, Some(&loaded), false);
    assert_eq!((dto.state, dto.problem.as_deref()), (InstalledState::Unavailable, Some("its program is missing")));
    let mut updated = r.clone();
    updated.digest = "b".repeat(64);
    assert_eq!(installed_dto(&updated, Some(&loaded), false).state, InstalledState::PendingRestart);
    let mut off = r.clone();
    off.enabled = false;
    assert_eq!(installed_dto(&off, Some(&loaded), false).state, InstalledState::Disabled);
    assert_eq!(installed_dto(&r, None, true).state, InstalledState::Unavailable, "safe mode");
    assert_eq!(SignatureDto::of_record("unknownKey", None).kind, SignatureKind::UnknownKey);
    assert_eq!(SignatureDto::of_record("other", None).kind, SignatureKind::Unsigned);
}

#[test]
fn the_list_reads_the_install_folder_and_reports_damaged_records() {
    let t = tempfile::tempdir().unwrap();
    let dir = PluginsDir::new(t.path());
    let good = record("com.example.kv");
    std::fs::create_dir_all(t.path().join(&good.id)).unwrap();
    std::fs::write(t.path().join(&good.id).join(RECORD_FILE), serde_json::to_vec(&good).unwrap()).unwrap();
    std::fs::create_dir_all(t.path().join("com.example.bad")).unwrap();
    std::fs::write(t.path().join("com.example.bad").join(RECORD_FILE), b"{").unwrap();
    let installed = Installed {
        dir: Some(dir.clone()),
        ..Installed::none()
    };
    let l = list(&installed);
    assert_eq!(l.plugins.len(), 1);
    assert_eq!(l.plugins[0].id, "com.example.kv");
    assert_eq!(l.damaged.len(), 1);
    assert!(!l.developer_mode && !l.safe_mode);
    // Grants and enablement go through the folder (the commands add their argument checks).
    let r = dir.set_granted("com.example.kv", &[]).unwrap();
    assert!(r.granted.is_empty());
    let r = dir.set_granted("com.example.kv", &[Permission::Network, Permission::RunPrograms]).unwrap();
    assert_eq!(r.granted, [Permission::Network], "only what it asks for");
    assert!(!dir.set_enabled("com.example.kv", false).unwrap().enabled);
    assert_eq!(list(&installed).plugins[0].state, InstalledState::Disabled);
    assert!(list(&Installed::none()).plugins.is_empty());
}

#[test]
fn nothing_is_loaded_in_safe_mode_and_a_missing_folder_is_empty() {
    let t = tempfile::tempdir().unwrap();
    let mut b = imt_plugin_host::Builtins::none();
    let i = Installed::load(Some(t.path().join("plugins")), true, &mut b);
    assert!(i.loaded.is_empty() && i.safe_mode && b.processes().is_empty());
    let i = Installed::load(Some(t.path().join("none")), false, &mut b);
    assert!(i.loaded.is_empty());
    let i = Installed::load(None, false, &mut b);
    assert!(i.dir.is_none());
}
