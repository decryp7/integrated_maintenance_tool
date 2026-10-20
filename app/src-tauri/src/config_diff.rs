//! Configuration diff commands: compare two configuration files (registry
//! exports, JSON/XML/INI/TOML/YAML, msinfo reports...) or two snapshots of
//! the same machine (folders such as two collector runs), page through the
//! tree diff and export it. Parsing and diffing live in `imt_core::config`;
//! the window's workspace supplies the parsers its plugins contribute.
//!
//! The last computed diff is cached (keyed by the files' paths, sizes and
//! modification times and the compare options), so paging, filtering and
//! folding do not re-read the files.

use crate::paths::WindowGrants;
use crate::windows::{read, CurrentWindow};
use crate::{err, CmdResult};
use imt_core::config::export::{parse_format, write_diff, Sides};
use imt_core::config::snapshot::{diff_folders, SnapshotDiff};
use imt_core::config::{
    diff, read_config, CompareOptions, ConfigParsers, Diff, DiffFormats, DiffOptions, DiffPage, Limits, SideInfo, DEFAULT_IGNORE,
};
use serde::Serialize;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::SystemTime;

/// A computed diff and what it was computed from.
struct Computed {
    key: Key,
    diff: Diff,
    left: SideInfo,
    right: SideInfo,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Key {
    files: [(PathBuf, u64, Option<SystemTime>); 2],
    options: CompareOptions,
}

static LAST: Mutex<Option<Arc<Computed>>> = Mutex::new(None);

fn stamp(p: &Path) -> (PathBuf, u64, Option<SystemTime>) {
    let meta = std::fs::metadata(p).ok();
    (
        p.to_path_buf(),
        meta.as_ref().map_or(0, |m| m.len()),
        meta.and_then(|m| m.modified().ok()),
    )
}

/// The diff of two files, from the cache when nothing changed.
fn computed(parsers: &ConfigParsers, l: &Path, r: &Path, options: &CompareOptions) -> anyhow::Result<Arc<Computed>> {
    let key = Key {
        files: [stamp(l), stamp(r)],
        options: options.clone(),
    };
    if let Some(c) = crate::sync::lock(&LAST).as_ref().filter(|c| c.key == key) {
        return Ok(c.clone());
    }
    let limits = Limits::default();
    let lt = read_config(parsers, l, &limits).map_err(|e| anyhow::anyhow!("left file: {e}"))?;
    let rt = read_config(parsers, r, &limits).map_err(|e| anyhow::anyhow!("right file: {e}"))?;
    let side = |p: &Path, t: &imt_core::config::ConfigTree| SideInfo {
        path: p.to_string_lossy().into_owned(),
        format: t.format.clone(),
        nodes: t.nodes.len(),
        truncated: t.truncated,
    };
    let c = Arc::new(Computed {
        diff: diff(&lt, &rt, options)?,
        left: side(l, &lt),
        right: side(r, &rt),
        key,
    });
    // Counts only: paths and contents stay out of the logs.
    log::info!(LeftNodes = c.left.nodes as u64, RightNodes = c.right.nodes as u64, Differences = c.diff.counts.differences() as u64; "Configuration diff computed");
    *crate::sync::lock(&LAST) = Some(c.clone());
    Ok(c)
}

/// The two files the user chose, each named in errors.
fn sides(grants: &WindowGrants, left: &str, right: &str) -> CmdResult<(PathBuf, PathBuf)> {
    let side = |raw: &str, what: &str| {
        grants
            .read_file(raw, &[])
            .map(|g| g.path().to_path_buf())
            .map_err(|e| format!("{what} file: {e}"))
    };
    Ok((side(left, "left")?, side(right, "right")?))
}

async fn parsers(state: &CurrentWindow) -> CmdResult<ConfigParsers> {
    read(state, |ws| Ok(ws.config_parsers())).await
}

async fn blocking<T: Send + 'static>(f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> CmdResult<T> {
    crate::sync::blocking_anyhow("the configuration diff", f).await
}

/// Supported formats and the default ignore patterns.
#[tauri::command]
pub(crate) async fn config_diff_formats(state: CurrentWindow) -> CmdResult<DiffFormats> {
    let parsers = parsers(&state).await?;
    Ok(DiffFormats {
        formats: parsers.formats(),
        default_ignore: DEFAULT_IGNORE.iter().map(|s| s.to_string()).collect(),
    })
}

/// One page of the tree diff of two files.
#[tauri::command]
pub(crate) async fn diff_configs(
    state: CurrentWindow,
    grants: WindowGrants,
    left_path: String,
    right_path: String,
    options: DiffOptions,
) -> CmdResult<DiffPage> {
    options.view.validate().map_err(err)?;
    let (l, r) = sides(&grants, &left_path, &right_path)?;
    let parsers = parsers(&state).await?;
    blocking(move || {
        let c = computed(&parsers, &l, &r, &options.compare)?;
        let (total, rows) = c.diff.rows(&options.view);
        Ok(DiffPage {
            left: c.left.clone(),
            right: c.right.clone(),
            case_insensitive: c.diff.case_insensitive,
            counts: c.diff.counts,
            total,
            offset: options.view.offset.min(total),
            rows,
            truncated: c.diff.truncated,
        })
    })
    .await
}

/// Compares the configuration files of two folders (two snapshots of one machine).
#[tauri::command]
pub(crate) async fn diff_snapshots(
    state: CurrentWindow,
    grants: WindowGrants,
    left_dir: String,
    right_dir: String,
    compare: CompareOptions,
) -> CmdResult<SnapshotDiff> {
    let side = |raw: &str, what: &str| {
        grants
            .read_dir(raw)
            .map(|g| g.path().to_path_buf())
            .map_err(|e| format!("{what} folder: {e}"))
    };
    let (l, r) = (side(&left_dir, "left")?, side(&right_dir, "right")?);
    let parsers = parsers(&state).await?;
    blocking(move || diff_folders(&parsers, &l, &r, &compare, &Limits::default())).await
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiffExported {
    path: String,
    rows: usize,
}

/// Writes the diff (filtered by the view's kinds and search) as CSV or HTML.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // the command's arguments are its wire contract
pub(crate) async fn export_config_diff(
    state: CurrentWindow,
    grants: WindowGrants,
    op: crate::activity::OpId,
    left_path: String,
    right_path: String,
    options: DiffOptions,
    format: String,
    path: String,
    overwrite: Option<bool>,
) -> CmdResult<DiffExported> {
    options.view.validate().map_err(err)?;
    let format = parse_format(&format).map_err(err)?;
    let (l, r) = sides(&grants, &left_path, &right_path)?;
    let target = grants.write(&path, &["csv", "html", "htm"])?.path().to_path_buf();
    imt_core::overwrite::check_overwrite(Path::new(&path), overwrite.unwrap_or(false))?;
    let parsers = parsers(&state).await?;
    blocking(move || {
        let c = computed(&parsers, &l, &r, &options.compare)?;
        let file = std::fs::File::create(&target).map_err(|e| anyhow::anyhow!("cannot create {}: {e}", target.display()))?;
        let mut out = std::io::BufWriter::new(file);
        let sides = Sides {
            left: &c.left.path,
            right: &c.right.path,
        };
        let rows = write_diff(&c.diff, &options.view, format, &sides, &mut out)?;
        std::io::Write::flush(&mut out)?;
        let (safe, n) = (crate::activity::privacy::safe_path(&target), rows.to_string());
        crate::activity::domain_event(
            "export.written",
            op.as_deref(),
            &[("Export", "configDiff"), ("Path", &safe), ("Rows", &n)],
        );
        Ok(DiffExported {
            path: target.to_string_lossy().into_owned(),
            rows,
        })
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::{Access, GrantSource};
    use crate::test_support::{block, main_window, test_app, TempDir};
    use imt_core::config::{ChangeKind, DiffView};

    fn granted(app: &tauri::App<tauri::test::MockRuntime>) -> WindowGrants {
        WindowGrants::of(app.handle(), "main")
    }

    /// What the user chose in a dialog of the main window.
    fn chose(app: &tauri::App<tauri::test::MockRuntime>, path: &Path, access: Access) {
        crate::paths::grant(app.handle(), "main", path, access, GrantSource::Dialog).unwrap();
    }

    /// The cache is process-wide: these tests take turns.
    static TURN: Mutex<()> = Mutex::new(());

    fn write(dir: &Path, name: &str, text: &str) -> String {
        let p = dir.join(name);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, text).unwrap();
        p.to_string_lossy().into_owned()
    }

    #[test]
    fn diffs_pages_and_caches() {
        let _turn = crate::sync::lock(&TURN);
        let app = test_app(None);
        let tmp = TempDir::new("config-diff");
        // Nothing chosen yet: the files are refused.
        let (l0, r0) = (write(&tmp.0, "a.json", "{}"), write(&tmp.0, "b.json", "{}"));
        let refused = block(diff_configs(main_window(&app), granted(&app), l0, r0, DiffOptions::default())).unwrap_err();
        assert!(refused.contains("only reads files you choose"), "{refused}");
        chose(&app, &tmp.0, Access::Read);
        let l = write(&tmp.0, "a.json", r#"{"a": 1, "b": {"c": 2}, "InstallDate": 1}"#);
        let r = write(&tmp.0, "b.json", r#"{"a": 2, "b": {"c": 2}, "d": 3, "InstallDate": 2}"#);
        let formats = block(config_diff_formats(main_window(&app))).unwrap();
        assert!(formats.formats.iter().any(|f| f.id == "registry"), "plugins contribute formats");
        assert!(formats.formats.iter().any(|f| f.id == "msinfo"));
        assert_eq!(formats.default_ignore.len(), DEFAULT_IGNORE.len());

        let page = block(diff_configs(
            main_window(&app),
            granted(&app),
            l.clone(),
            r.clone(),
            DiffOptions::default(),
        ))
        .unwrap();
        assert_eq!((page.counts.changed, page.counts.added, page.counts.ignored), (1, 1, 2));
        assert_eq!(page.left.format, "json");
        assert_eq!(
            page.rows.iter().map(|r| r.path.as_str()).collect::<Vec<_>>(),
            ["a", "b", "b/c", "d"]
        );
        // Same files and options: served from the cache.
        let first = LAST.lock().unwrap().clone().unwrap();
        let options = DiffOptions {
            view: DiffView {
                kinds: vec![ChangeKind::Added],
                ..DiffView::default()
            },
            ..DiffOptions::default()
        };
        let page = block(diff_configs(main_window(&app), granted(&app), l.clone(), r.clone(), options)).unwrap();
        assert_eq!(page.total, 1);
        assert!(Arc::ptr_eq(&first, LAST.lock().unwrap().as_ref().unwrap()));
        // Errors name the side.
        let bad = write(&tmp.0, "bad.json", "{");
        let e = block(diff_configs(
            main_window(&app),
            granted(&app),
            bad,
            r.clone(),
            DiffOptions::default(),
        ))
        .unwrap_err();
        assert!(e.message.starts_with("left file: JSON"), "{e}");
        assert!(block(diff_configs(
            main_window(&app),
            granted(&app),
            l.clone(),
            "".into(),
            DiffOptions::default()
        ))
        .unwrap_err()
        .contains("right file: choose a file"));
        assert!(block(diff_configs(
            main_window(&app),
            granted(&app),
            l.clone(),
            tmp.0.to_string_lossy().into(),
            DiffOptions::default()
        ))
        .unwrap_err()
        .contains("is not a file"));
        assert!(block(diff_configs(
            main_window(&app),
            granted(&app),
            "/no/such/file.json".into(),
            r.clone(),
            DiffOptions::default()
        ))
        .is_err());
        let mut huge = DiffOptions::default();
        huge.view.search = "x".repeat(5000);
        assert!(block(diff_configs(main_window(&app), granted(&app), l.clone(), r.clone(), huge)).is_err());
        let mut bad_ignore = DiffOptions::default();
        bad_ignore.compare.ignore = vec!["re:(".into()];
        assert!(block(diff_configs(main_window(&app), granted(&app), l, r, bad_ignore)).is_err());
    }

    #[test]
    fn exports_csv_and_html() {
        let _turn = crate::sync::lock(&TURN);
        let app = test_app(None);
        let tmp = TempDir::new("config-diff-export");
        let l = write(&tmp.0, "a.ini", "[s]\nk=<1>\n");
        let r = write(&tmp.0, "b.ini", "[S]\nK=<2>\n");
        let csv = tmp.0.join("out.csv").to_string_lossy().into_owned();
        chose(&app, &tmp.0, Access::Read);
        // Writing needs the save dialog's grant for that very file.
        let refused = block(export_config_diff(
            main_window(&app),
            granted(&app),
            crate::activity::OpId::default(),
            l.clone(),
            r.clone(),
            DiffOptions::default(),
            "csv".into(),
            csv.clone(),
            None,
        ))
        .unwrap_err();
        assert!(refused.contains("only writes where you choose"), "{refused}");
        chose(&app, Path::new(&csv), Access::Write);
        chose(&app, &tmp.0.join("out.html"), Access::Write);
        let _log = crate::activity::test_capture();
        let done = block(export_config_diff(
            main_window(&app),
            granted(&app),
            crate::activity::OpId(Some("op-diff".into())),
            l.clone(),
            r.clone(),
            DiffOptions::default(),
            "csv".into(),
            csv.clone(),
            None,
        ))
        .unwrap();
        assert_eq!(done.rows, 2);
        let e = crate::tools::tests::domain_event("export.written", "op-diff");
        assert_eq!((e.props["Export"].as_str(), e.props["Rows"].as_str()), ("configDiff", "2"));
        assert!(std::fs::read_to_string(&csv).unwrap().contains("changed,S/K,string,<1>,string,<2>"));
        let html = tmp.0.join("out.html").to_string_lossy().into_owned();
        block(export_config_diff(
            main_window(&app),
            granted(&app),
            crate::activity::OpId::default(),
            l.clone(),
            r.clone(),
            DiffOptions::default(),
            "HTML".into(),
            html.clone(),
            None,
        ))
        .unwrap();
        assert!(std::fs::read_to_string(&html).unwrap().contains("&lt;2&gt;"));
        let export = |overwrite| {
            block(export_config_diff(
                main_window(&app),
                granted(&app),
                crate::activity::OpId::default(),
                l.clone(),
                r.clone(),
                DiffOptions::default(),
                "csv".into(),
                csv.clone(),
                overwrite,
            ))
        };
        assert_eq!(
            export(None).unwrap_err(),
            crate::error::file_exists(format!("{csv} already exists; confirm to replace it"))
        );
        assert_eq!(export(Some(true)).unwrap().rows, 2);
        assert!(block(export_config_diff(
            main_window(&app),
            granted(&app),
            crate::activity::OpId::default(),
            l.clone(),
            r.clone(),
            DiffOptions::default(),
            "pdf".into(),
            html,
            None,
        ))
        .is_err());
        let into_dir = tmp.0.to_string_lossy().into_owned();
        assert!(block(export_config_diff(
            main_window(&app),
            granted(&app),
            crate::activity::OpId::default(),
            l.clone(),
            r.clone(),
            DiffOptions::default(),
            "csv".into(),
            into_dir,
            None,
        ))
        .unwrap_err()
        .contains("is a folder"));
        assert!(block(export_config_diff(
            main_window(&app),
            granted(&app),
            crate::activity::OpId::default(),
            l.clone(),
            r.clone(),
            DiffOptions::default(),
            "csv".into(),
            "/no/such/dir/x.csv".into(),
            None,
        ))
        .is_err());
        assert!(block(export_config_diff(
            main_window(&app),
            granted(&app),
            crate::activity::OpId::default(),
            l,
            r,
            DiffOptions::default(),
            "csv".into(),
            "/".into(),
            None,
        ))
        .is_err());
    }

    #[test]
    fn diffs_snapshot_folders() {
        let _turn = crate::sync::lock(&TURN);
        let app = test_app(None);
        let tmp = TempDir::new("config-diff-snap");
        write(&tmp.0, "a/app.yaml", "x: 1\n");
        write(&tmp.0, "b/app.yaml", "x: 2\n");
        chose(&app, &tmp.0, Access::Read);
        let (a, b) = (
            tmp.0.join("a").to_string_lossy().into_owned(),
            tmp.0.join("b").to_string_lossy().into_owned(),
        );
        let d = block(diff_snapshots(
            main_window(&app),
            granted(&app),
            a.clone(),
            b,
            CompareOptions::default(),
        ))
        .unwrap();
        assert_eq!(d.files.len(), 1);
        assert_eq!(d.files[0].counts.unwrap().changed, 1);
        let file = tmp.0.join("a/app.yaml").to_string_lossy().into_owned();
        assert!(block(diff_snapshots(
            main_window(&app),
            granted(&app),
            a.clone(),
            file,
            CompareOptions::default()
        ))
        .unwrap_err()
        .contains("is not a folder"));
        assert!(block(diff_snapshots(
            main_window(&app),
            granted(&app),
            "/no/such".into(),
            a,
            CompareOptions::default()
        ))
        .is_err());
    }
}
