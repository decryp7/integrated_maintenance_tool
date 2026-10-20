//! The active profile: loading the saved one, validation with error lines,
//! saving (with version history) and import/export. The profile on disk is
//! shared by all windows; each window's workspace applies its own copy (a
//! project can carry a different one).

use crate::activity::{domain_event, privacy, OpId};
use crate::jobs::{JobKind, JobSpec};
use crate::paths::WindowGrants;
use crate::windows::{read, CurrentWindow};
use crate::{err, AppState, CmdResult, State};
use imt_core::{LogStore, Profile, DEFAULT_PROFILE_TOML};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Runtime};

pub(crate) const PROFILE_FILE: &str = "active.imtprofile.toml";
/// Saved profile versions kept for "restore".
const PROFILE_HISTORY: usize = 20;

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProfileDoc {
    pub name: String,
    pub path: Option<String>,
    pub toml: String,
}

impl ProfileDoc {
    /// The built-in default profile.
    pub fn builtin() -> (Profile, ProfileDoc) {
        let p = Profile::default_profile();
        let doc = ProfileDoc {
            name: p.name.clone(),
            path: None,
            toml: DEFAULT_PROFILE_TOML.to_string(),
        };
        (p, doc)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProfileValidation {
    ok: bool,
    error: Option<String>,
    line: Option<usize>,
}

#[tauri::command]
pub(crate) fn get_profile(state: CurrentWindow) -> CmdResult<ProfileDoc> {
    Ok(state.profile_doc())
}

/// Parses and compiles a profile against the installed plugins.
/// Async (off the main thread): while an import or live tail holds the workspace, a sync command would wait for
/// the lock on the main thread and freeze every window.
#[tauri::command]
pub(crate) async fn validate_profile(state: CurrentWindow, toml: String) -> CmdResult<ProfileValidation> {
    read(&state, move |store| Ok(validate(store, &toml))).await
}

pub(crate) fn validate(store: &dyn LogStore, toml: &str) -> ProfileValidation {
    match Profile::from_toml(toml) {
        Err(e) => ProfileValidation {
            ok: false,
            error: Some(e.message),
            line: e.line,
        },
        Ok(p) => match store.validate_profile(p) {
            Ok(_) => ProfileValidation {
                ok: true,
                error: None,
                line: None,
            },
            Err(e) => ProfileValidation {
                ok: false,
                line: imt_core::profile::RuleContext::of(&e).and_then(|r| r.line_in(toml)),
                error: Some(err(e)),
            },
        },
    }
}

/// Saves the profile as the active one (for new windows and the next start) and applies it to the calling
/// window: the file is checked and written now, the files are re-imported with it as a background import job
/// (progress in the status bar, Cancel in the Jobs pane). Returns that job at once.
#[tauri::command]
pub(crate) fn save_profile<R: Runtime>(
    app: AppHandle<R>,
    app_state: State<'_, AppState>,
    state: CurrentWindow,
    op: OpId,
    toml: String,
) -> CmdResult<u64> {
    let profile = Profile::from_toml(&toml).map_err(|e| crate::CmdError::invalid(e.message).field("toml").line(e.line))?;
    // Never replace a working profile on disk with one that does not compile.
    state.store_read().validate_profile(profile.clone()).map_err(|e| {
        let line = imt_core::profile::RuleContext::of(&e).and_then(|r| r.line_in(&toml));
        crate::CmdError::invalid(err(e)).field("toml").line(line)
    })?;
    let name = profile.name.clone();
    let path = app_state.profile_dir.as_ref().map(|d| d.join(PROFILE_FILE));
    if let Some(p) = &path {
        std::fs::create_dir_all(p.parent().ok_or("invalid profile folder")?).map_err(err)?;
        archive_profile(p);
        std::fs::write(p, &toml).map_err(err)?;
    }
    domain_event("profile.saved", op.as_deref(), &[("Profile", &name)]);
    let spec = JobSpec::new(JobKind::Import, format!("Apply the profile {name}"));
    state.set_profile_doc(ProfileDoc {
        name,
        path: path.map(|p| p.to_string_lossy().into_owned()),
        toml,
    });
    crate::workspace_commands::start_import(&app, &state, spec, move |ws| ws.set_profile(profile))
}

/// Copies the current profile into `history/` before it is replaced.
fn archive_profile(active: &Path) {
    let Some(dir) = active.parent().map(|d| d.join("history")) else {
        return;
    };
    if !active.exists() || std::fs::create_dir_all(&dir).is_err() {
        return;
    }
    let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S%3f");
    let _ = std::fs::copy(active, dir.join(format!("{stamp}.imtprofile.toml")));
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    for old in files.iter().rev().skip(PROFILE_HISTORY) {
        let _ = std::fs::remove_file(old);
    }
}

/// Epoch ms of a history stamp (`YYYYMMDD-HHMMSSmmm`, UTC); `None` when it is not one.
fn stamp_ms(stamp: &str) -> Option<i64> {
    let (date_time, millis) = stamp.split_at_checked(stamp.len().checked_sub(3)?)?;
    let t = chrono::NaiveDateTime::parse_from_str(date_time, "%Y%m%d-%H%M%S").ok()?;
    Some(t.and_utc().timestamp_millis() + millis.parse::<i64>().ok()?)
}

/// Previous profile versions, newest first: `[{ file, name, savedAt, savedAtMs }]`.
#[tauri::command]
pub(crate) fn profile_history(app_state: State<'_, AppState>) -> Vec<Value> {
    let Some(dir) = app_state.profile_dir.as_ref().map(|d| d.join("history")) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = std::fs::read_dir(&dir)
        .map(|r| r.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    files.sort();
    files
        .iter()
        .rev()
        .map(|f| {
            let toml = std::fs::read_to_string(f).unwrap_or_default();
            let name = Profile::from_toml(&toml).map(|p| p.name).unwrap_or_else(|_| "(unreadable)".into());
            let stem = f.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
            let saved_at = stem.split('.').next().unwrap_or("");
            serde_json::json!({ "file": stem, "name": name, "savedAt": saved_at, "savedAtMs": stamp_ms(saved_at) })
        })
        .collect()
}

/// One previous version's text (not applied until saved again).
#[tauri::command]
pub(crate) fn profile_version(app_state: State<'_, AppState>, file: String) -> CmdResult<ProfileDoc> {
    let dir = app_state.profile_dir.as_ref().ok_or("no profile folder")?.join("history");
    let version = crate::paths::child(&dir, &file).map_err(|_| "invalid version".to_string())?;
    let toml = std::fs::read_to_string(version).map_err(err)?;
    let name = Profile::from_toml(&toml).map_err(err)?.name;
    Ok(ProfileDoc { name, path: None, toml })
}

/// The built-in profile templates (applied only through validate + save).
#[tauri::command]
pub(crate) fn profile_templates() -> &'static [imt_core::templates::ProfileTemplate] {
    imt_core::templates::TEMPLATES
}

/// Reads a profile file chosen by the user (not applied until saved).
#[tauri::command]
pub(crate) fn import_profile(grants: WindowGrants, path: String) -> CmdResult<ProfileDoc> {
    let file = grants.read_file(&path, &["toml"])?;
    let toml = std::fs::read_to_string(file.path()).map_err(err)?;
    let profile = Profile::from_toml(&toml).map_err(err)?;
    Ok(ProfileDoc {
        name: profile.name,
        path: Some(path),
        toml,
    })
}

/// Writes the profile text to a file the user chose; an existing file only with `overwrite`.
#[tauri::command]
pub(crate) fn export_profile(grants: WindowGrants, path: String, toml: String, overwrite: Option<bool>) -> CmdResult<()> {
    let target = grants.write(&path, &["toml"])?;
    imt_core::overwrite::check_overwrite(Path::new(&path), overwrite.unwrap_or(false))?;
    std::fs::write(target.path(), &toml).map_err(err)?;
    let (safe, bytes) = (privacy::safe_path(Path::new(&path)), toml.len().to_string());
    domain_event("export.written", None, &[("Export", "profile"), ("Path", &safe), ("Bytes", &bytes)]);
    Ok(())
}

/// The saved profile if it still compiles against the installed plugins,
/// else the built-in default.
pub(crate) fn load_profile(dir: Option<&PathBuf>, store: &dyn LogStore) -> (Profile, ProfileDoc) {
    if let Some(path) = dir.map(|d| d.join(PROFILE_FILE)) {
        if let Ok(toml) = std::fs::read_to_string(&path) {
            match Profile::from_toml(&toml) {
                Ok(p) if store.validate_profile(p.clone()).is_ok() => {
                    let doc = ProfileDoc {
                        name: p.name.clone(),
                        path: Some(path.to_string_lossy().into_owned()),
                        toml,
                    };
                    return (p, doc);
                }
                _ => log::warn!("ignoring invalid profile {}", path.display()),
            }
        }
    }
    ProfileDoc::builtin()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, test_app, TempDir};
    use imt_core::Workspace;
    use tauri::Manager;

    #[test]
    fn profiles_are_validated_saved_and_reloaded() {
        let dir = TempDir::new("profiles");
        let app = test_app(Some(dir.0.clone()));
        let st = || main_window(&app);
        let shared = || app.state::<AppState>();

        let doc = get_profile(st()).unwrap();
        assert_eq!((doc.path, doc.name.as_str()), (None, "Default (Serilog)"));

        let win = st();
        let store = win.store_read();
        let validate = |t: &str| validate(store.as_ref(), t);
        let ok = validate("name = 'x'");
        assert!(ok.ok && ok.error.is_none());
        let syntax = validate("name = 'x'\n[parse\n");
        assert_eq!((syntax.ok, syntax.line), (false, Some(2)));
        let semantic = validate("name = 'x'\n\n[[extract]]\nname = \"stats\"\nregex = '('\n");
        assert!(!semantic.ok);
        assert_eq!(semantic.line, Some(4));
        let unlocated = validate("[parse]\nassume_timezone = 'Mars'");
        assert!(block(validate_profile(st(), "name = 'y'".into())).unwrap().ok);
        drop(store);
        assert_eq!((unlocated.ok, unlocated.line), (false, None));

        let bad = save_profile(app.handle().clone(), shared(), st(), OpId::default(), "[parse".into()).unwrap_err();
        assert_eq!(
            (bad.code, bad.field.as_deref(), bad.line),
            (crate::ErrorCode::Invalid, Some("toml"), Some(1))
        );
        let rule = save_profile(
            app.handle().clone(),
            shared(),
            st(),
            OpId::default(),
            "name = 'x'

[[extract]]
name = \"stats\"
regex = '('
"
            .into(),
        )
        .unwrap_err();
        assert_eq!(rule.line, Some(4), "{rule:?}");
        let _log = crate::activity::test_capture();
        let job = save_profile(
            app.handle().clone(),
            shared(),
            st(),
            OpId(Some("op-profile".into())),
            "name = 'Saved'".into(),
        )
        .unwrap();
        crate::test_support::job_done(&app, job);
        let summary = block(crate::workspace_commands::workspace_summary(st())).unwrap();
        assert_eq!(
            crate::tools::tests::domain_event("profile.saved", "op-profile").props["Profile"],
            "Saved"
        );
        assert_eq!(summary["profileName"], "Saved");
        let doc = get_profile(st()).unwrap();
        assert_eq!(doc.name, "Saved");
        let saved = PathBuf::from(doc.path.unwrap());
        assert!(saved.exists());

        // A fresh start picks the saved profile up; an invalid one falls back to the default.
        let fresh = || Workspace::new(Profile::default_profile()).unwrap();
        assert_eq!(load_profile(Some(&dir.0), &fresh()).1.name, "Saved");
        std::fs::write(&saved, "[parse]\nregexes = ['(']").unwrap();
        assert_eq!(load_profile(Some(&dir.0), &fresh()).1.name, "Default (Serilog)");
        assert_eq!(load_profile(None, &fresh()).1.path, None);

        let exported = dir.0.join("out.imtprofile.toml");
        let grants = || crate::paths::WindowGrants::of(app.handle(), "main");
        let refused = export_profile(grants(), exported.to_string_lossy().into_owned(), "name = 'Exported'".into(), None).unwrap_err();
        assert!(refused.contains("only writes where you choose"), "{refused}");
        assert!(!exported.exists());
        crate::paths::grant(
            app.handle(),
            "main",
            &exported,
            crate::paths::Access::Write,
            crate::paths::GrantSource::Dialog,
        )
        .unwrap();
        export_profile(grants(), exported.to_string_lossy().into_owned(), "name = 'Exported'".into(), None).unwrap();
        let written = crate::tools::tests::domain_events();
        let w = written
            .iter()
            .find(|c| c.props["Operation"] == "export.written" && c.props["Export"] == "profile")
            .expect("profile export");
        assert_eq!(w.props["Bytes"], "17");
        // Replacing the file needs the user's confirmation.
        let again = export_profile(grants(), exported.to_string_lossy().into_owned(), "name = 'Other'".into(), None).unwrap_err();
        assert_eq!(
            again,
            crate::error::file_exists(format!("{} already exists; confirm to replace it", exported.display()))
        );
        export_profile(
            grants(),
            exported.to_string_lossy().into_owned(),
            "name = 'Exported'".into(),
            Some(true),
        )
        .unwrap();
        // Reading needs an open grant (a save grant is not one).
        let refused = import_profile(grants(), exported.to_string_lossy().into_owned()).unwrap_err();
        assert!(refused.contains("only reads files you choose"), "{refused}");
        crate::paths::grant(
            app.handle(),
            "main",
            &exported,
            crate::paths::Access::Read,
            crate::paths::GrantSource::Dialog,
        )
        .unwrap();
        let imported = import_profile(grants(), exported.to_string_lossy().into_owned()).unwrap();
        assert_eq!(imported.name, "Exported");
        assert!(import_profile(grants(), dir.0.join("missing.toml").to_string_lossy().into_owned()).is_err());
        std::fs::write(&exported, "[parse").unwrap();
        assert!(import_profile(grants(), exported.to_string_lossy().into_owned()).is_err());
        assert!(export_profile(
            grants(),
            dir.0.join("no/such/dir/x.toml").to_string_lossy().into_owned(),
            String::new(),
            None
        )
        .is_err());
    }

    #[test]
    fn saving_without_a_config_dir_still_applies() {
        let app = test_app(None);
        crate::test_support::job_done(
            &app,
            save_profile(
                app.handle().clone(),
                app.state::<AppState>(),
                main_window(&app),
                OpId::default(),
                "name = 'Memory only'".into(),
            )
            .unwrap(),
        );
        let summary = block(crate::workspace_commands::workspace_summary(main_window(&app))).unwrap();
        assert_eq!(summary["profileName"], "Memory only");
        assert_eq!(get_profile(main_window(&app)).unwrap().path, None);
    }

    #[test]
    fn history_keeps_previous_versions() {
        let dir = TempDir::new("history");
        let app = test_app(Some(dir.0.clone()));
        let st = || main_window(&app);
        let shared = || app.state::<AppState>();
        // An invalid profile is refused and nothing is written.
        assert!(save_profile(
            app.handle().clone(),
            shared(),
            st(),
            OpId::default(),
            "[[pipeline]]\nstep='plugin'\nid='nope'".into()
        )
        .is_err());
        assert!(!dir.0.join(PROFILE_FILE).exists());
        for name in ["One", "Two", "Three"] {
            crate::test_support::job_done(
                &app,
                save_profile(app.handle().clone(), shared(), st(), OpId::default(), format!("name = '{name}'")).unwrap(),
            );
        }
        let history = profile_history(shared());
        assert_eq!(history.len(), 2, "the first save had nothing to archive");
        assert_eq!(history[0]["name"], "Two");
        let at = history[0]["savedAtMs"].as_i64().unwrap();
        assert!((chrono::Utc::now().timestamp_millis() - at).abs() < 60_000, "saved just now");
        let file = history[1]["file"].as_str().unwrap().to_string();
        assert_eq!(profile_version(shared(), file).unwrap().name, "One");
        assert!(profile_version(shared(), "../x".into()).is_err());
        let none = test_app(None);
        assert!(profile_history(none.state::<AppState>()).is_empty());
        assert!(profile_version(none.state::<AppState>(), "x".into()).is_err());
    }

    #[test]
    fn templates_are_listed_with_their_samples() {
        let list = serde_json::to_value(profile_templates()).unwrap();
        let list = list.as_array().unwrap();
        assert!(list.len() >= 13);
        assert!(list
            .iter()
            .all(|t| t["id"].is_string() && t["toml"].is_string() && t["sampleName"].is_string()));
    }

    #[test]
    fn history_stamps_are_read_as_utc_milliseconds() {
        assert_eq!(stamp_ms("20261017-095244123"), Some(1_792_230_764_123));
        for bad in ["", "12", "nope-stamp-xyz", "20261017-095244abc", "2026101x-095244123"] {
            assert_eq!(stamp_ms(bad), None, "{bad}");
        }
    }
}
