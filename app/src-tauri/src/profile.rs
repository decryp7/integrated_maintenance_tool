//! The profile commands: thin adapters over [`crate::services::profile`] (validation with error lines,
//! saving with version history, import/export). The profile on disk is shared by all windows; each
//! window's workspace applies its own copy (a project can carry a different one).

use crate::activity::{domain_event, privacy, OpId};
use crate::jobs::{JobKind, JobSpec};
use crate::paths::WindowGrants;
use crate::services::profile::ProfileFiles;
use crate::windows::CurrentWindow;
use crate::{err, AppState, CmdResult, State};
use debrief_core::{Profile, DEFAULT_PROFILE_TOML};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Runtime};

#[cfg(test)]
use crate::services::profile::{stamp_ms, PROFILE_FILE};

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

/// The app's profile folder.
fn files(app_state: &AppState) -> ProfileFiles {
    ProfileFiles::new(app_state.profile_dir.clone())
}

#[tauri::command]
pub(crate) async fn get_profile(state: CurrentWindow) -> CmdResult<ProfileDoc> {
    Ok(state.profile_doc())
}

/// Parses and compiles a profile with the window's plugins (the store checks it again when it applies it).
#[tauri::command]
pub(crate) async fn validate_profile(state: CurrentWindow, toml: String) -> CmdResult<ProfileValidation> {
    let win = state.arc().clone();
    crate::sync::blocking("validating the profile", move || Ok(validate(&win.host, &toml))).await
}

pub(crate) fn validate(host: &crate::engine::Host, toml: &str) -> ProfileValidation {
    match ProfileFiles::check(host, toml) {
        Ok(_) => ProfileValidation {
            ok: true,
            error: None,
            line: None,
        },
        Err(e) => ProfileValidation {
            ok: false,
            line: e.line,
            error: Some(e.message),
        },
    }
}

/// Applies the profile to the calling window (a background import job, with progress in the status bar
/// and Cancel in the Jobs pane; returned at once). Only when the window's workspace has swapped to it
/// is it saved as the active one (for new windows and the next start, the previous one kept in the
/// history) and shown as the window's profile: a failed or cancelled apply changes nothing.
#[tauri::command]
pub(crate) async fn save_profile<R: Runtime>(
    app: AppHandle<R>,
    app_state: State<'_, AppState>,
    state: CurrentWindow,
    op: OpId,
    toml: String,
) -> CmdResult<u64> {
    // Never replace a working profile with one that does not compile.
    let profile = ProfileFiles::check(&state.host, &toml)?;
    let compiled = state.host.compile(profile.clone()).map_err(err)?;
    let name = profile.name.clone();
    domain_event("profile.saved", op.as_deref(), &[("Profile", &name)]);
    let spec = JobSpec::new(JobKind::Import, format!("Apply the profile {name}"));
    let (files, win) = (files(&app_state), state.arc().clone());
    let applied = profile.clone();
    crate::engine::import::start(
        &app,
        &state,
        spec,
        Box::new(move |store| store.set_profile(applied.clone())),
        move |_report| {
            // The store re-derived what the change needs; now the window's host and new stores follow it.
            win.host.set_compiled(compiled);
            win.engine.set_profile(profile);
            let path = files.persist(&toml)?;
            win.set_profile_doc(ProfileDoc {
                name,
                path: path.map(|p| p.to_string_lossy().into_owned()),
                toml,
            });
            Ok(())
        },
    )
}

/// Previous profile versions, newest first: `[{ file, name, savedAt, savedAtMs }]`.
#[tauri::command]
pub(crate) async fn profile_history(app_state: State<'_, AppState>) -> CmdResult<Vec<Value>> {
    Ok(files(&app_state).history())
}

/// One previous version's text (not applied until saved again).
#[tauri::command]
pub(crate) async fn profile_version(app_state: State<'_, AppState>, file: String) -> CmdResult<ProfileDoc> {
    files(&app_state).version(&file)
}

/// Reads a profile file chosen by the user (not applied until saved).
#[tauri::command]
pub(crate) async fn import_profile(grants: WindowGrants, path: String) -> CmdResult<ProfileDoc> {
    let file = grants.read_file(&path, &["toml"])?;
    let toml = std::fs::read_to_string(file.path()).map_err(err)?;
    let profile = Profile::from_toml(&toml).map_err(|e| crate::CmdError::invalid(e.message).line(e.line))?;
    Ok(ProfileDoc {
        name: profile.name,
        path: Some(path),
        toml,
    })
}

/// Writes the profile text to a file the user chose; an existing file only with `overwrite`.
#[tauri::command]
pub(crate) async fn export_profile(grants: WindowGrants, op: OpId, path: String, toml: String, overwrite: Option<bool>) -> CmdResult<()> {
    let target = grants.write(&path, &["toml"])?;
    debrief_core::overwrite::check_overwrite(Path::new(&path), overwrite.unwrap_or(false))?;
    std::fs::write(target.path(), &toml).map_err(err)?;
    let (safe, bytes) = (privacy::safe_path(Path::new(&path)), toml.len().to_string());
    domain_event(
        "export.written",
        op.as_deref(),
        &[("Export", "profile"), ("Path", &safe), ("Bytes", &bytes)],
    );
    Ok(())
}

/// The saved profile if it still compiles with the installed plugins, else the built-in default.
pub(crate) fn load_profile(dir: Option<&PathBuf>, registry: &debrief_core::PluginRegistry) -> (Profile, ProfileDoc) {
    ProfileFiles::new(dir.cloned()).load(registry)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Json as _;
    use crate::test_support::Now as _;
    use crate::test_support::{block, main_window, test_app, TempDir};
    use tauri::Manager;

    #[test]
    fn profiles_are_validated_saved_and_reloaded() {
        let dir = TempDir::new("profiles");
        let app = test_app(Some(dir.0.clone()));
        let st = || main_window(&app);
        let shared = || app.state::<AppState>();

        let doc = get_profile(st()).now().unwrap();
        assert_eq!((doc.path, doc.name.as_str()), (None, "Default (Serilog)"));

        let win = st();
        let validate = |t: &str| validate(&win.host, t);
        let ok = validate("name = 'x'");
        assert!(ok.ok && ok.error.is_none());
        let syntax = validate("name = 'x'\n[parse\n");
        assert_eq!((syntax.ok, syntax.line), (false, Some(2)));
        let semantic = validate("name = 'x'\n\n[[extract]]\nname = \"stats\"\nregex = '('\n");
        assert!(!semantic.ok);
        assert_eq!(semantic.line, Some(4));
        let unlocated = validate("name = 5");
        let formats = validate("name = 'x'\n[parse]\nregexes = []");
        assert_eq!((formats.ok, formats.line), (false, Some(2)), "formats are parsers now");
        assert!(block(validate_profile(st(), "name = 'y'".into())).unwrap().ok);
        assert_eq!((unlocated.ok, unlocated.line), (false, None));

        let bad = save_profile(app.handle().clone(), shared(), st(), OpId::default(), "[parse".into())
            .now()
            .unwrap_err();
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
        .now()
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
        .now()
        .unwrap();
        crate::test_support::job_done(&app, job);
        let summary = block(crate::workspace_commands::workspace_summary(st())).unwrap().json();
        assert_eq!(
            crate::tools::tests::domain_event("profile.saved", "op-profile").props["Profile"],
            "Saved"
        );
        assert_eq!(summary["profileName"], "Saved");
        let doc = get_profile(st()).now().unwrap();
        assert_eq!(doc.name, "Saved");
        let saved = PathBuf::from(doc.path.unwrap());
        assert!(saved.exists());

        // A fresh start picks the saved profile up; an invalid one falls back to the default.
        let fresh = debrief_core::PluginRegistry::new;
        assert_eq!(load_profile(Some(&dir.0), &fresh()).1.name, "Saved");
        std::fs::write(&saved, "[parse]\nregexes = ['(']").unwrap();
        assert_eq!(load_profile(Some(&dir.0), &fresh()).1.name, "Default (Serilog)");
        assert_eq!(load_profile(None, &fresh()).1.path, None);

        let exported = dir.0.join("out.debriefprofile.toml");
        let grants = || crate::paths::WindowGrants::of(app.handle(), "main");
        let refused = export_profile(
            grants(),
            OpId::default(),
            exported.to_string_lossy().into_owned(),
            "name = 'Exported'".into(),
            None,
        )
        .now()
        .unwrap_err();
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
        export_profile(
            grants(),
            OpId::default(),
            exported.to_string_lossy().into_owned(),
            "name = 'Exported'".into(),
            None,
        )
        .now()
        .unwrap();
        let written = crate::tools::tests::domain_events();
        let w = written
            .iter()
            .find(|c| c.props["Operation"] == "export.written" && c.props["Export"] == "profile")
            .expect("profile export");
        assert_eq!(w.props["Bytes"], "17");
        // Replacing the file needs the user's confirmation.
        let again = export_profile(
            grants(),
            OpId::default(),
            exported.to_string_lossy().into_owned(),
            "name = 'Other'".into(),
            None,
        )
        .now()
        .unwrap_err();
        assert_eq!(
            again,
            crate::error::file_exists(format!("{} already exists; confirm to replace it", exported.display()))
        );
        export_profile(
            grants(),
            OpId::default(),
            exported.to_string_lossy().into_owned(),
            "name = 'Exported'".into(),
            Some(true),
        )
        .now()
        .unwrap();
        // Reading needs an open grant (a save grant is not one).
        let refused = import_profile(grants(), exported.to_string_lossy().into_owned()).now().unwrap_err();
        assert!(refused.contains("only reads files you choose"), "{refused}");
        crate::paths::grant(
            app.handle(),
            "main",
            &exported,
            crate::paths::Access::Read,
            crate::paths::GrantSource::Dialog,
        )
        .unwrap();
        let imported = import_profile(grants(), exported.to_string_lossy().into_owned()).now().unwrap();
        assert_eq!(imported.name, "Exported");
        assert!(import_profile(grants(), dir.0.join("missing.toml").to_string_lossy().into_owned())
            .now()
            .is_err());
        std::fs::write(&exported, "[parse").unwrap();
        assert!(import_profile(grants(), exported.to_string_lossy().into_owned()).now().is_err());
        assert!(export_profile(
            grants(),
            OpId::default(),
            dir.0.join("no/such/dir/x.toml").to_string_lossy().into_owned(),
            String::new(),
            None
        )
        .now()
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
            .now()
            .unwrap(),
        );
        let summary = block(crate::workspace_commands::workspace_summary(main_window(&app)))
            .unwrap()
            .json();
        assert_eq!(summary["profileName"], "Memory only");
        assert_eq!(get_profile(main_window(&app)).now().unwrap().path, None);
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
        .now()
        .is_err());
        assert!(!dir.0.join(PROFILE_FILE).exists());
        for name in ["One", "Two", "Three"] {
            crate::test_support::job_done(
                &app,
                save_profile(app.handle().clone(), shared(), st(), OpId::default(), format!("name = '{name}'"))
                    .now()
                    .unwrap(),
            );
        }
        let history = profile_history(shared()).now().unwrap();
        assert_eq!(history.len(), 2, "the first save had nothing to archive");
        assert_eq!(history[0]["name"], "Two");
        let at = history[0]["savedAtMs"].as_i64().unwrap();
        assert!((chrono::Utc::now().timestamp_millis() - at).abs() < 60_000, "saved just now");
        let file = history[1]["file"].as_str().unwrap().to_string();
        assert_eq!(profile_version(shared(), file).now().unwrap().name, "One");
        assert!(profile_version(shared(), "../x".into()).now().is_err());
        let none = test_app(None);
        assert!(profile_history(none.state::<AppState>()).now().unwrap().is_empty());
        assert!(profile_version(none.state::<AppState>(), "x".into()).now().is_err());
    }

    #[test]
    fn history_stamps_are_read_as_utc_milliseconds() {
        assert_eq!(stamp_ms("20261017-095244123"), Some(1_792_230_764_123));
        for bad in ["", "12", "nope-stamp-xyz", "20261017-095244abc", "2026101x-095244123"] {
            assert_eq!(stamp_ms(bad), None, "{bad}");
        }
    }
}
