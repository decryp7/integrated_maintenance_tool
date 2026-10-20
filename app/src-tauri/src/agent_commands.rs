//! Commands of the "Collection schedules" and "Runs" views: load, validate
//! and save `imt-agent.toml` (validation is the agent library's), the
//! editor's schema, the runs in the configured output folder, one run's
//! manifest, and opening a run's folder in a new window. The logic is in
//! plain functions over explicit folders so it is testable without Tauri;
//! the commands only resolve the app's default folders.

use crate::paths::WindowGrants;
use crate::{err, CmdResult};
use imt_agent::config::{describe, MAX_CONFIG_BYTES};
use imt_agent::run::{is_run_name, list_runs, read_manifest, RunManifest, RunSummary};
use imt_agent::schema::{schema, AgentSchema};
use imt_agent::{AgentConfig, AgentConfigFile, Issue};
use imt_core::overwrite::check_overwrite;
use serde::Serialize;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Runtime};

/// File name of the config in the app's config folder (the default).
pub const CONFIG_NAME: &str = "imt-agent.toml";
/// Most runs returned to the UI.
const MAX_RUNS: usize = 2000;

/// A config document as the editor sees it.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentConfigDoc {
    pub path: String,
    /// False: `config` is a starter document, not yet saved.
    pub exists: bool,
    /// `None` when the file exists but cannot be read (see `issues`).
    pub config: Option<AgentConfigFile>,
    pub issues: Vec<Issue>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunsListing {
    pub output_dir: String,
    pub runs: Vec<RunSummary>,
    /// Runs left out beyond the limit.
    pub truncated: usize,
}

/// The app's folders the commands default to.
pub struct Dirs {
    pub config: PathBuf,
    pub output: PathBuf,
}

fn dirs<R: Runtime>(app: &AppHandle<R>) -> CmdResult<Dirs> {
    let config = crate::app_dirs::config(app).map_err(err)?;
    let output = crate::app_dirs::data(app).map_err(err)?.join("agent").join("runs");
    Ok(Dirs { config, output })
}

/// The config path: the user's choice (a `.toml` file in an existing folder,
/// canonicalised) or the default in the app's config folder.
pub fn resolve_path(path: Option<&str>, dirs: &Dirs) -> CmdResult<PathBuf> {
    let Some(p) = path.map(str::trim).filter(|p| !p.is_empty()) else {
        return Ok(dirs.config.join(CONFIG_NAME));
    };
    crate::error::lift(crate::paths::output_file(p, &["toml"]).map_err(|e| format!("agent config: {e}")))
}

fn base_of(path: &Path) -> &Path {
    path.parent().unwrap_or(Path::new("."))
}

pub fn load_doc(path: Option<&str>, dirs: &Dirs) -> CmdResult<AgentConfigDoc> {
    let path = resolve_path(path, dirs)?;
    let path_s = path.to_string_lossy().into_owned();
    if !path.exists() {
        return Ok(AgentConfigDoc {
            path: path_s,
            exists: false,
            config: Some(AgentConfigFile::starter(&dirs.output.to_string_lossy())),
            issues: Vec::new(),
        });
    }
    Ok(match AgentConfigFile::load(&path) {
        Ok(file) => {
            let issues = file.validate(base_of(&path)).err().unwrap_or_default();
            AgentConfigDoc {
                path: path_s,
                exists: true,
                config: Some(file),
                issues,
            }
        }
        Err(issues) => AgentConfigDoc {
            path: path_s,
            exists: true,
            config: None,
            issues,
        },
    })
}

pub fn validate_doc(path: Option<&str>, config: &AgentConfigFile, dirs: &Dirs) -> CmdResult<Vec<Issue>> {
    let path = resolve_path(path, dirs)?;
    Ok(config.validate(base_of(&path)).err().unwrap_or_default())
}

/// Saves a valid config (atomically). An invalid one is not written: its issues are returned.
/// An existing file is replaced only with `overwrite` (else the `FILE_EXISTS` error the UI
/// confirms); its comments and layout are kept, only changed values are rewritten.
pub fn save_doc(path: Option<&str>, config: &AgentConfigFile, overwrite: bool, dirs: &Dirs) -> CmdResult<Vec<Issue>> {
    let path = resolve_path(path, dirs)?;
    if let Err(issues) = config.validate(base_of(&path)) {
        return Ok(issues);
    }
    check_overwrite(&path, overwrite)?;
    let text = match existing_text(&path)? {
        // A file that is not TOML (any more) is replaced by a fresh document.
        Some(existing) => config.to_toml_over(&existing).or_else(|_| fresh_text(config))?,
        None => fresh_text(config)?,
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(err)?;
    }
    let tmp = path.with_extension("toml.tmp");
    std::fs::write(&tmp, text).map_err(err)?;
    std::fs::rename(&tmp, &path).map_err(err)?;
    Ok(Vec::new())
}

/// A new file: a header and every key.
fn fresh_text(config: &AgentConfigFile) -> CmdResult<String> {
    let header = "# imt-agent configuration, saved by IMT. See `imt-agent print-example` for every key.\n\n";
    Ok(format!("{header}{}", config.to_toml()?))
}

/// The text of the file being replaced; `None` when there is none, or it is too large or not
/// text (it is then replaced by a fresh document).
fn existing_text(path: &Path) -> CmdResult<Option<String>> {
    if !std::fs::metadata(path).is_ok_and(|m| m.len() <= MAX_CONFIG_BYTES) {
        return Ok(None);
    }
    crate::error::lift(
        std::fs::read(path)
            .map(|bytes| String::from_utf8(bytes).ok())
            .map_err(|e| format!("cannot read {}: {e}", path.display())),
    )
}

fn load_valid(path: Option<&str>, dirs: &Dirs) -> CmdResult<AgentConfig> {
    let path = resolve_path(path, dirs)?;
    if !path.exists() {
        return Err(crate::CmdError::from(format!(
            "{} does not exist yet; save the schedules first",
            path.display()
        )));
    }
    crate::error::lift(AgentConfig::load(&path).map_err(|i| describe(&i)))
}

pub fn runs(path: Option<&str>, dirs: &Dirs) -> CmdResult<RunsListing> {
    let cfg = load_valid(path, dirs)?;
    let mut runs = list_runs(&cfg.output_dir);
    let truncated = runs.len().saturating_sub(MAX_RUNS);
    runs.truncate(MAX_RUNS);
    Ok(RunsListing {
        output_dir: cfg.output_dir.to_string_lossy().into_owned(),
        runs,
        truncated,
    })
}

/// A run folder of the configured output, by id (no path tricks).
pub fn run_dir(path: Option<&str>, run_id: &str, dirs: &Dirs) -> CmdResult<PathBuf> {
    if !is_run_name(run_id) || run_id.contains(['/', '\\']) || run_id.contains("..") {
        return Err(crate::CmdError::from(format!("\"{run_id}\" is not a run")));
    }
    let cfg = load_valid(path, dirs)?;
    let dir = cfg.output_dir.join(run_id);
    if !dir.is_dir() {
        return Err(crate::CmdError::from(format!("the run {run_id} no longer exists")));
    }
    Ok(dir)
}

pub fn manifest(path: Option<&str>, run_id: &str, dirs: &Dirs) -> CmdResult<RunManifest> {
    crate::error::lift(read_manifest(&run_dir(path, run_id, dirs)?))
}

/// The user's config path checked against the window's grants (`None`: the default in IMT's config
/// folder, which needs none). Reading accepts a file chosen to open or to save (a new one).
fn granted(grants: &WindowGrants, path: Option<String>, access: crate::paths::Access) -> CmdResult<Option<String>> {
    let Some(p) = path.filter(|p| !p.trim().is_empty()) else {
        return Ok(None);
    };
    let g = match access {
        crate::paths::Access::Read => grants.read_or_write(&p, &["toml"]),
        crate::paths::Access::Write => grants.write(&p, &["toml"]),
    }
    .map_err(|e| format!("agent config: {e}"))?;
    Ok(Some(g.path().to_string_lossy().into_owned()))
}

#[tauri::command]
pub async fn agent_schema() -> AgentSchema {
    schema()
}

/// Loads the config the user opened; the editor may save it back to the same file.
#[tauri::command]
pub async fn agent_config_load<R: Runtime>(app: AppHandle<R>, grants: WindowGrants, path: Option<String>) -> CmdResult<AgentConfigDoc> {
    let d = dirs(&app)?;
    let path = granted(&grants, path, crate::paths::Access::Read)?;
    if let Some(p) = &path {
        let _ = grants.grant(Path::new(p), crate::paths::Access::Write, crate::paths::GrantSource::Dialog);
    }
    tauri::async_runtime::spawn_blocking(move || load_doc(path.as_deref(), &d))
        .await
        .map_err(err)?
}

#[tauri::command]
pub async fn agent_config_validate<R: Runtime>(
    app: AppHandle<R>,
    grants: WindowGrants,
    path: Option<String>,
    config: AgentConfigFile,
) -> CmdResult<Vec<Issue>> {
    let d = dirs(&app)?;
    let path = granted(&grants, path, crate::paths::Access::Read)?;
    tauri::async_runtime::spawn_blocking(move || validate_doc(path.as_deref(), &config, &d))
        .await
        .map_err(err)?
}

/// Saves the config; an existing file only with `overwrite` (the editor's own file, or confirmed).
#[tauri::command]
pub async fn agent_config_save<R: Runtime>(
    app: AppHandle<R>,
    grants: WindowGrants,
    path: Option<String>,
    config: AgentConfigFile,
    overwrite: Option<bool>,
) -> CmdResult<Vec<Issue>> {
    let d = dirs(&app)?;
    let path = granted(&grants, path, crate::paths::Access::Write)?;
    let overwrite = overwrite.unwrap_or(false);
    tauri::async_runtime::spawn_blocking(move || save_doc(path.as_deref(), &config, overwrite, &d))
        .await
        .map_err(err)?
}

#[tauri::command]
pub async fn agent_runs<R: Runtime>(app: AppHandle<R>, grants: WindowGrants, path: Option<String>) -> CmdResult<RunsListing> {
    let d = dirs(&app)?;
    let path = granted(&grants, path, crate::paths::Access::Read)?;
    tauri::async_runtime::spawn_blocking(move || runs(path.as_deref(), &d))
        .await
        .map_err(err)?
}

#[tauri::command]
pub async fn agent_run_manifest<R: Runtime>(
    app: AppHandle<R>,
    grants: WindowGrants,
    path: Option<String>,
    run_id: String,
) -> CmdResult<RunManifest> {
    let d = dirs(&app)?;
    let path = granted(&grants, path, crate::paths::Access::Read)?;
    tauri::async_runtime::spawn_blocking(move || manifest(path.as_deref(), &run_id, &d))
        .await
        .map_err(err)?
}

/// Opens a run's folder in a new IMT window (the window manager's usual path).
#[tauri::command]
pub async fn agent_open_run<R: Runtime>(
    app: AppHandle<R>,
    grants: WindowGrants,
    path: Option<String>,
    run_id: String,
) -> CmdResult<String> {
    let path = granted(&grants, path, crate::paths::Access::Read)?;
    let dir = run_dir(path.as_deref(), &run_id, &dirs(&app)?)?;
    crate::error::lift(crate::window_manager::open_new(&app, vec![dir.to_string_lossy().into_owned()]).map_err(err))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::Now as _;
    use crate::test_support::TempDir;
    use imt_agent::run::{create_run_dir, write_manifest, RunReason};

    fn dirs_in(t: &TempDir) -> Dirs {
        Dirs {
            config: t.0.join("config"),
            output: t.0.join("runs"),
        }
    }

    #[test]
    fn starter_save_load_and_validation() {
        let t = TempDir::new("agent-cfg");
        let d = dirs_in(&t);
        let doc = load_doc(None, &d).unwrap();
        assert!(!doc.exists);
        assert!(doc.path.ends_with(CONFIG_NAME));
        let mut cfg = doc.config.unwrap();
        assert_eq!(cfg.agent.output_dir, d.output.to_string_lossy());
        assert!(validate_doc(None, &cfg, &d).unwrap().is_empty());
        assert!(save_doc(None, &cfg, false, &d).unwrap().is_empty());
        let saved = std::fs::read_to_string(d.config.join(CONFIG_NAME)).unwrap();
        assert!(saved.starts_with("# imt-agent configuration"));
        let doc = load_doc(None, &d).unwrap();
        assert!(doc.exists && doc.issues.is_empty());
        assert_eq!(doc.config.as_ref(), Some(&cfg));

        // An invalid document is not saved; its issues come back with paths.
        cfg.schedules[0].cron = Some("99 * * * *".into());
        let issues = save_doc(None, &cfg, true, &d).unwrap();
        assert_eq!(issues[0].path, "schedules[0].cron");
        assert_eq!(validate_doc(None, &cfg, &d).unwrap(), issues);
        assert_eq!(
            load_doc(None, &d).unwrap().config.unwrap().schedules[0].cron.as_deref(),
            Some("0 2 * * *")
        );

        // A corrupt file loads with its problem and no document; an invalid one with both.
        std::fs::write(d.config.join(CONFIG_NAME), "version = ").unwrap();
        let doc = load_doc(None, &d).unwrap();
        assert!(doc.config.is_none() && !doc.issues.is_empty());
        std::fs::write(d.config.join(CONFIG_NAME), "version = 1").unwrap();
        let doc = load_doc(None, &d).unwrap();
        assert!(doc.config.is_some() && doc.issues[0].path == "agent.outputDir");
    }

    #[test]
    fn an_existing_config_is_replaced_only_when_confirmed_and_keeps_its_comments() {
        use imt_core::overwrite::FILE_EXISTS_SUFFIX;
        let t = TempDir::new("agent-overwrite");
        let d = dirs_in(&t);
        let file = d.config.join(CONFIG_NAME);
        std::fs::create_dir_all(&d.config).unwrap();
        let mine = format!(
            "# My schedules -- do not lose this.\nversion = 1\n\n[agent]\n# where runs go\noutputDir = {:?}\ntickSecs = 15 # check often\n\n[[schedules]]\nname = \"nightly\"\ncron = \"0 2 * * *\"\n  [[schedules.actions]]\n  kind = \"collect\"\n",
            d.output.to_string_lossy()
        );
        std::fs::write(&file, &mine).unwrap();
        let mut cfg = load_doc(None, &d).unwrap().config.unwrap();
        cfg.agent.tick_secs = 60;
        // Not confirmed: refused with the error the UI asks about; the file is untouched.
        let e = save_doc(None, &cfg, false, &d).unwrap_err();
        assert_eq!(e, crate::error::file_exists(format!("{}{FILE_EXISTS_SUFFIX}", file.display())));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), mine);
        // An invalid document is reported before asking.
        let mut bad = cfg.clone();
        bad.agent.tick_secs = 0;
        assert_eq!(save_doc(None, &bad, false, &d).unwrap()[0].path, "agent.tickSecs");
        // Confirmed: only the changed value is rewritten; comments and left-out defaults stay.
        assert!(save_doc(None, &cfg, true, &d).unwrap().is_empty());
        let saved = std::fs::read_to_string(&file).unwrap();
        assert_eq!(saved, mine.replace("tickSecs = 15", "tickSecs = 60"));
        assert_eq!(load_doc(None, &d).unwrap().config.unwrap(), cfg);

        // A file that is not TOML (or not text, or too large) is replaced by a fresh document.
        for junk in [
            b"version = ".to_vec(),
            vec![0xff, 0xfe, 0x00],
            vec![b'#'; MAX_CONFIG_BYTES as usize + 1],
        ] {
            std::fs::write(&file, junk).unwrap();
            assert!(save_doc(None, &cfg, true, &d).unwrap().is_empty());
            let saved = std::fs::read_to_string(&file).unwrap();
            assert!(saved.starts_with("# imt-agent configuration"), "{saved}");
            assert_eq!(load_doc(None, &d).unwrap().config.unwrap(), cfg);
        }
        // A folder in the way cannot be read.
        std::fs::remove_file(&file).unwrap();
        std::fs::create_dir(&file).unwrap();
        assert!(save_doc(None, &cfg, true, &d).unwrap_err().contains("cannot read"));
    }

    #[test]
    fn paths_are_checked() {
        let t = TempDir::new("agent-paths");
        let d = dirs_in(&t);
        assert_eq!(resolve_path(Some("  "), &d).unwrap(), d.config.join(CONFIG_NAME));
        assert!(resolve_path(Some(&t.0.join("x.txt").to_string_lossy()), &d)
            .unwrap_err()
            .contains(".toml"));
        assert!(resolve_path(Some("x.toml"), &d).unwrap_err().contains("absolute"));
        assert!(resolve_path(Some(&t.0.join("no/such/dir/x.toml").to_string_lossy()), &d)
            .unwrap_err()
            .contains("does not exist"));
        assert!(resolve_path(Some("a\0.toml"), &d).is_err());
        let p = t.0.join("mine.toml");
        assert_eq!(
            resolve_path(Some(p.to_str().unwrap()), &d).unwrap(),
            crate::paths::plain(std::fs::canonicalize(&t.0).unwrap()).join("mine.toml")
        );
    }

    #[test]
    fn runs_and_manifests() {
        let t = TempDir::new("agent-runs");
        let d = dirs_in(&t);
        assert!(runs(None, &d).unwrap_err().contains("save the schedules first"));
        save_doc(None, &AgentConfigFile::starter(&d.output.to_string_lossy()), false, &d).unwrap();
        assert!(runs(None, &d).unwrap().runs.is_empty());
        let now = chrono::Utc::now();
        let (id, dir) = create_run_dir(&d.output, "nightly", now).unwrap();
        write_manifest(&dir, &RunManifest::start(&id, "nightly", RunReason::Manual, now)).unwrap();
        let listing = runs(None, &d).unwrap();
        assert_eq!(listing.runs.len(), 1);
        assert_eq!(listing.truncated, 0);
        assert_eq!(manifest(None, &id, &d).unwrap().schedule, "nightly");
        assert!(manifest(None, "../etc", &d).unwrap_err().contains("not a run"));
        assert!(manifest(None, "20260101-000000-gone", &d).unwrap_err().contains("no longer exists"));
        // An invalid saved config is reported.
        std::fs::write(d.config.join(CONFIG_NAME), "version = 1").unwrap();
        assert!(runs(None, &d).unwrap_err().contains("agent.outputDir"));
        assert_eq!(agent_schema().now().version, imt_agent::CONFIG_VERSION);
    }
}
