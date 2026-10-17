//! The active profile: loading the saved one, validation with error lines,
//! saving (with version history) and import/export. The profile on disk is
//! shared by all windows; each window's workspace applies its own copy (a
//! project can carry a different one).

use crate::windows::{write, CurrentWindow};
use crate::{err, AppState, CmdResult, State};
use imt_core::{LogStore, Profile, DEFAULT_PROFILE_TOML};
use serde::Serialize;
use serde_json::Value;
use std::path::{Path, PathBuf};

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
#[tauri::command]
pub(crate) fn validate_profile(state: CurrentWindow, toml: String) -> CmdResult<ProfileValidation> {
    let store = state.store_read();
    Ok(validate(store.as_ref(), &toml))
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
                line: find_line(toml, &e),
                error: Some(err(e)),
            },
        },
    }
}

/// Points compile errors ("extract \"stats\": regex") at the rule's line.
fn find_line(toml: &str, e: &anyhow::Error) -> Option<usize> {
    let msg = e.to_string();
    let name = msg.split('"').nth(1)?;
    toml.lines().position(|l| l.contains(&format!("\"{name}\""))).map(|i| i + 1)
}

/// Saves the profile as the active one (for new windows and the next start)
/// and applies it to the calling window.
#[tauri::command]
pub(crate) async fn save_profile(app_state: State<'_, AppState>, state: CurrentWindow, toml: String) -> CmdResult<Value> {
    let profile = Profile::from_toml(&toml).map_err(err)?;
    // Never replace a working profile on disk with one that does not compile.
    state.store_read().validate_profile(profile.clone()).map_err(err)?;
    let name = profile.name.clone();
    let path = app_state.profile_dir.as_ref().map(|d| d.join(PROFILE_FILE));
    if let Some(p) = &path {
        std::fs::create_dir_all(p.parent().ok_or("invalid profile folder")?).map_err(err)?;
        archive_profile(p);
        std::fs::write(p, &toml).map_err(err)?;
    }
    state.set_profile_doc(ProfileDoc {
        name,
        path: path.map(|p| p.to_string_lossy().into_owned()),
        toml,
    });
    write(&state, move |ws| {
        ws.set_profile(profile)?;
        serde_json::to_value(ws.summary()).map_err(anyhow::Error::from)
    })
    .await
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

/// Previous profile versions, newest first: `[{ file, name, savedAt }]`.
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
            serde_json::json!({ "file": stem, "name": name, "savedAt": stem.split('.').next().unwrap_or("") })
        })
        .collect()
}

/// One previous version's text (not applied until saved again).
#[tauri::command]
pub(crate) fn profile_version(app_state: State<'_, AppState>, file: String) -> CmdResult<ProfileDoc> {
    if file.contains(['/', '\\']) || file.contains("..") {
        return Err("invalid version".into());
    }
    let dir = app_state.profile_dir.as_ref().ok_or("no profile folder")?.join("history");
    let toml = std::fs::read_to_string(dir.join(&file)).map_err(err)?;
    let name = Profile::from_toml(&toml).map_err(err)?.name;
    Ok(ProfileDoc { name, path: None, toml })
}

/// Reads a profile file chosen by the user (not applied until saved).
#[tauri::command]
pub(crate) fn import_profile(path: String) -> CmdResult<ProfileDoc> {
    let toml = std::fs::read_to_string(&path).map_err(err)?;
    let profile = Profile::from_toml(&toml).map_err(err)?;
    Ok(ProfileDoc {
        name: profile.name,
        path: Some(path),
        toml,
    })
}

#[tauri::command]
pub(crate) fn export_profile(path: String, toml: String) -> CmdResult<()> {
    std::fs::write(path, toml).map_err(err)
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
        assert!(validate_profile(st(), "name = 'y'".into()).unwrap().ok);
        drop(store);
        assert_eq!((unlocated.ok, unlocated.line), (false, None));

        assert!(block(save_profile(shared(), st(), "[parse".into())).is_err());
        let summary = block(save_profile(shared(), st(), "name = 'Saved'".into())).unwrap();
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
        export_profile(exported.to_string_lossy().into_owned(), "name = 'Exported'".into()).unwrap();
        let imported = import_profile(exported.to_string_lossy().into_owned()).unwrap();
        assert_eq!(imported.name, "Exported");
        assert!(import_profile(dir.0.join("missing.toml").to_string_lossy().into_owned()).is_err());
        std::fs::write(&exported, "[parse").unwrap();
        assert!(import_profile(exported.to_string_lossy().into_owned()).is_err());
        assert!(export_profile(dir.0.join("no/such/dir/x.toml").to_string_lossy().into_owned(), String::new()).is_err());
    }

    #[test]
    fn saving_without_a_config_dir_still_applies() {
        let app = test_app(None);
        let summary = block(save_profile(
            app.state::<AppState>(),
            main_window(&app),
            "name = 'Memory only'".into(),
        ))
        .unwrap();
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
        assert!(block(save_profile(shared(), st(), "[[pipeline]]\nstep='plugin'\nid='nope'".into())).is_err());
        assert!(!dir.0.join(PROFILE_FILE).exists());
        for name in ["One", "Two", "Three"] {
            block(save_profile(shared(), st(), format!("name = '{name}'"))).unwrap();
        }
        let history = profile_history(shared());
        assert_eq!(history.len(), 2, "the first save had nothing to archive");
        assert_eq!(history[0]["name"], "Two");
        let file = history[1]["file"].as_str().unwrap().to_string();
        assert_eq!(profile_version(shared(), file).unwrap().name, "One");
        assert!(profile_version(shared(), "../x".into()).is_err());
        let none = test_app(None);
        assert!(profile_history(none.state::<AppState>()).is_empty());
        assert!(profile_version(none.state::<AppState>(), "x".into()).is_err());
    }
}
