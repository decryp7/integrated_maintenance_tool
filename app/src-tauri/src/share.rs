//! Explain and share: the redacted view, redaction previews, dashboard
//! tiles, incident report exports (HTML/PDF), chart and timeline images, and
//! shareable `.imtz` bundles. Engine work runs on the calling window's
//! workspace ([`CurrentWindow`]); every path and payload from the webview is
//! validated here before anything is written.

use crate::activity::{domain_event, privacy, OpId};
use crate::windows::{read, write, CurrentWindow};
use crate::{err, CmdResult};
use base64::Engine;
use imt_core::bundle::{BundleEstimate, BundleManifest, BundleOptions};
use imt_core::dashboard::{TileData, TileRequest};
use imt_core::overwrite::check_overwrite;
use imt_core::redact::RedactionPreview;
use imt_core::report::html::HtmlRenderer;
use imt_core::report::{Report, ReportRenderer, Theme};
use imt_core::workspace::{RedactionPreviewRequest, RedactionSummary, RedactionSummaryRequest};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Runtime};

/// Largest image a visual export writes (decoded PNG or SVG text).
const MAX_VISUAL_BYTES: usize = 32 << 20;
/// Largest report accepted from the UI (JSON).
const MAX_REPORT_JSON: usize = 64 << 20;

/// Folder under the system temp folder where bundles are extracted to be opened.
pub(crate) fn bundles_root() -> PathBuf {
    std::env::temp_dir().join("imt-bundles")
}

/// A path the user picked in a save dialog, checked: absolute, one of
/// `extensions`, in an existing folder, not a folder itself.
pub(crate) fn output_path(path: &str, extensions: &[&str]) -> anyhow::Result<PathBuf> {
    let p = Path::new(path);
    if !p.is_absolute() {
        anyhow::bail!("choose where to save the file (got a relative path)");
    }
    let ext = p
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase)
        .unwrap_or_default();
    if !extensions.contains(&ext.as_str()) {
        anyhow::bail!("the file must end in .{}", extensions.join(" or ."));
    }
    let name = p.file_name().ok_or_else(|| anyhow::anyhow!("the path has no file name"))?;
    let parent = p.parent().ok_or_else(|| anyhow::anyhow!("the path has no folder"))?;
    let parent = std::fs::canonicalize(parent).map_err(|_| anyhow::anyhow!("the folder {} does not exist", parent.display()))?;
    let target = parent.join(name);
    if target.is_dir() {
        anyhow::bail!("{} is a folder", target.display());
    }
    Ok(target)
}

/// Refuses to save a project into an opened bundle's folder (bundles open read-only).
pub(crate) fn refuse_bundle_folder(path: &str) -> CmdResult<()> {
    let root = std::fs::canonicalize(bundles_root()).unwrap_or_else(|_| bundles_root());
    let parent = Path::new(path).parent().and_then(|p| std::fs::canonicalize(p).ok());
    if parent.is_some_and(|p| p.starts_with(&root)) {
        return Err("this bundle is opened read-only: save the project somewhere else (Save As)".into());
    }
    Ok(())
}

/// Reports from the UI, each checked (limits, sanitised snapshots).
pub(crate) fn parse_reports(values: Vec<Value>) -> CmdResult<Vec<Report>> {
    values.into_iter().map(parse_report).collect()
}

fn parse_report(value: Value) -> CmdResult<Report> {
    if value.to_string().len() > MAX_REPORT_JSON {
        return Err(format!("a report is limited to {} MB", MAX_REPORT_JSON >> 20));
    }
    Report::parse(value).map_err(err)
}

/// Switches the window's redacted view on or off; returns the new summary.
#[tauri::command]
pub(crate) async fn set_redacted_view(state: CurrentWindow, op: OpId, enabled: bool) -> CmdResult<Value> {
    write(&state, move |ws| {
        ws.set_view_redaction(enabled)?;
        log::info!(Enabled = enabled; "Redacted view switched");
        domain_event(
            "redaction.toggled",
            op.as_deref(),
            &[("Enabled", if enabled { "true" } else { "false" })],
        );
        Ok(serde_json::to_value(ws.summary())?)
    })
    .await
}

/// A sample redacted with the active profile's `[redact]` rules, or an unsaved profile's.
#[tauri::command]
pub(crate) async fn preview_redaction(state: CurrentWindow, request: RedactionPreviewRequest) -> CmdResult<RedactionPreview> {
    read(&state, move |ws| Ok(ws.preview_redaction(&request))).await
}

/// What redaction would change in the events of a query (shown before exporting).
#[tauri::command]
pub(crate) async fn redaction_summary(state: CurrentWindow, request: RedactionSummaryRequest) -> CmdResult<RedactionSummary> {
    read(&state, move |ws| ws.redaction_summary(&request)).await
}

/// One dashboard tile evaluated over the window's query.
#[tauri::command]
pub(crate) async fn dashboard_tile(state: CurrentWindow, request: TileRequest) -> CmdResult<TileData> {
    read(&state, move |ws| ws.dashboard_tile(&request)).await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReportExportRequest {
    pub report: Value,
    /// `html` or `pdf`.
    pub format: String,
    pub path: String,
    #[serde(default)]
    pub redact: bool,
    #[serde(default)]
    pub theme: Theme,
    /// Replace an existing file (confirmed by the user).
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReportExported {
    pub path: String,
    pub bytes: u64,
    pub redaction: Vec<imt_core::redact::RuleHits>,
}

fn renderer(format: &str) -> CmdResult<Box<dyn ReportRenderer>> {
    match format {
        "html" => Ok(Box::new(HtmlRenderer)),
        "pdf" => Ok(Box::new(imt_pdf::PdfRenderer)),
        other => Err(format!("unknown report format \"{other}\" (use html or pdf)")),
    }
}

/// Logs the `export.written` domain event (the path as the privacy settings allow, never content).
fn written(op: Option<&str>, kind: &str, target: &Path, extra: &[(&str, &str)]) {
    let path = privacy::safe_path(target);
    let mut props = vec![("Export", kind), ("Path", path.as_str())];
    props.extend_from_slice(extra);
    domain_event("export.written", op, &props);
}

/// Writes a report as self-contained HTML or PDF, redacted when asked.
#[tauri::command]
pub(crate) async fn export_report<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    op: OpId,
    request: ReportExportRequest,
) -> CmdResult<ReportExported> {
    let renderer = renderer(&request.format)?;
    let target = output_path(&request.path, &[renderer.extension()]).map_err(err)?;
    check_overwrite(Path::new(&request.path), request.overwrite).map_err(err)?;
    let report = parse_report(request.report)?;
    let (redact, theme) = (request.redact, request.theme);
    let shown = target.clone();
    crate::jobs::export(&app, &state, "Export report", &shown, move |ws| {
        let resolved = ws.resolve_report(&report, redact)?;
        let mut out = std::io::BufWriter::new(std::fs::File::create(&target)?);
        renderer.render(&resolved, theme, &mut out)?;
        drop(out);
        let bytes = std::fs::metadata(&target)?.len();
        log::info!(Format = renderer.id(), Bytes = bytes, Redacted = redact; "Report exported");
        written(
            op.as_deref(),
            "report",
            &target,
            &[("Format", renderer.id()), ("Bytes", &bytes.to_string())],
        );
        Ok(ReportExported {
            path: target.to_string_lossy().into_owned(),
            bytes,
            redaction: resolved.redaction,
        })
    })
    .await
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct VisualRequest {
    pub path: String,
    /// `png` (a `data:image/png;base64,` URL) or `svg` (markup).
    pub kind: String,
    pub data: String,
    /// Replace an existing file (confirmed by the user).
    #[serde(default)]
    pub overwrite: bool,
}

#[derive(Debug, Serialize)]
pub(crate) struct VisualSaved {
    pub path: String,
    pub bytes: usize,
}

/// The bytes to write for a visual export, checked.
fn visual_bytes(kind: &str, data: &str) -> anyhow::Result<Vec<u8>> {
    match kind {
        "png" => {
            let b64 = data
                .strip_prefix("data:image/png;base64,")
                .ok_or_else(|| anyhow::anyhow!("not a PNG image"))?;
            if b64.len() > MAX_VISUAL_BYTES / 3 * 4 + 4 {
                anyhow::bail!("images are limited to {} MB", MAX_VISUAL_BYTES >> 20);
            }
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(b64)
                .map_err(|_| anyhow::anyhow!("the PNG image is not valid base64"))?;
            if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
                anyhow::bail!("not a PNG image");
            }
            Ok(bytes)
        }
        "svg" => {
            if data.len() > MAX_VISUAL_BYTES {
                anyhow::bail!("images are limited to {} MB", MAX_VISUAL_BYTES >> 20);
            }
            let clean = imt_core::svg::sanitize(data, &imt_core::svg::FIGURE)?;
            Ok(format!("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n{clean}\n").into_bytes())
        }
        other => anyhow::bail!("unknown image kind \"{other}\" (use png or svg)"),
    }
}

/// Saves a chart or timeline image (PNG or sanitised SVG) where the user chose.
#[tauri::command]
pub(crate) async fn save_visual(op: OpId, request: VisualRequest) -> CmdResult<VisualSaved> {
    tauri::async_runtime::spawn_blocking(move || {
        let target = output_path(&request.path, &[request.kind.as_str()]).map_err(err)?;
        check_overwrite(Path::new(&request.path), request.overwrite).map_err(err)?;
        let bytes = visual_bytes(&request.kind, &request.data).map_err(err)?;
        std::fs::write(&target, &bytes).map_err(|e| format!("cannot write {}: {e}", target.display()))?;
        log::info!(Kind = request.kind.as_str(), Bytes = bytes.len() as u64; "Image exported");
        written(
            op.as_deref(),
            "visual",
            &target,
            &[("Format", &request.kind), ("Bytes", &bytes.len().to_string())],
        );
        Ok(VisualSaved {
            path: target.to_string_lossy().into_owned(),
            bytes: bytes.len(),
        })
    })
    .await
    .map_err(err)?
}

/// What a bundle with `options` would contain (sizes before compression).
#[tauri::command]
pub(crate) async fn estimate_bundle(state: CurrentWindow, options: BundleOptions) -> CmdResult<BundleEstimate> {
    let profile = state.profile_doc().toml;
    read(&state, move |ws| ws.estimate_bundle(&options, &profile)).await
}

/// Writes a shareable `.imtz` bundle of the window's workspace.
#[tauri::command]
pub(crate) async fn create_bundle<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    op: OpId,
    options: BundleOptions,
    path: String,
    overwrite: Option<bool>,
) -> CmdResult<BundleManifest> {
    let target = output_path(&path, &["imtz"]).map_err(err)?;
    check_overwrite(Path::new(&path), overwrite.unwrap_or(false)).map_err(err)?;
    parse_reports(options.reports.clone())?;
    let profile = state.profile_doc().toml;
    let shown = target.clone();
    crate::jobs::export(&app, &state, "Share bundle", &shown, move |ws| {
        let m = ws.write_bundle(&options, &profile, &target)?;
        log::info!(Entries = m.entries.len() as u64, Redacted = m.redacted; "Bundle written");
        written(op.as_deref(), "bundle", &target, &[("Entries", &m.entries.len().to_string())]);
        Ok(m)
    })
    .await
}

/// Opens a `.imtz`: extracts it into its own temporary folder (archive
/// limits, no path may leave it), checks it against its manifest and opens
/// its project read-only. A bundle without a project (rare) opens its files.
#[tauri::command]
pub(crate) async fn open_bundle<R: Runtime>(app: AppHandle<R>, state: CurrentWindow, op: OpId, path: String) -> CmdResult<Value> {
    let source = PathBuf::from(&path);
    let extracted = tauri::async_runtime::spawn_blocking(move || imt_core::bundle::extract(&source, &bundles_root()))
        .await
        .map_err(err)?
        .map_err(err)?;
    let mut opened = match &extracted.project {
        Some(project) => crate::workspace_commands::open_project(app, state.clone(), op, project.to_string_lossy().into_owned()).await?,
        None => {
            let dir = extracted.dir.to_string_lossy().into_owned();
            let summary = crate::workspace_commands::open_paths(app, state.clone(), vec![dir]).await?;
            serde_json::json!({ "name": "", "summary": summary, "ui": Value::Null, "missing": [], "reports": [] })
        }
    };
    // Read-only: no project file to save over; Save asks for a new place.
    state.set_project(None);
    opened["path"] = Value::from(path);
    opened["readOnly"] = Value::Bool(true);
    opened["bundle"] = serde_json::to_value(&extracted.manifest).map_err(err)?;
    opened["folder"] = Value::from(extracted.dir.to_string_lossy().into_owned());
    log::info!(Manifest = extracted.manifest.is_some(); "Bundle opened");
    Ok(opened)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, samples, test_app, TempDir};
    use serde_json::json;
    use tauri::Manager;

    fn tiny_png() -> String {
        let bytes = b"\x89PNG\r\n\x1a\nrest-of-image";
        format!("data:image/png;base64,{}", base64::engine::general_purpose::STANDARD.encode(bytes))
    }

    #[test]
    fn output_paths_are_checked() {
        let d = TempDir::new("share-paths");
        let ok = d.0.join("chart.PNG");
        assert!(output_path(&ok.to_string_lossy(), &["png"]).is_ok());
        assert!(output_path("chart.png", &["png"]).unwrap_err().to_string().contains("relative"));
        assert!(output_path(&d.0.join("x.exe").to_string_lossy(), &["png", "svg"])
            .unwrap_err()
            .to_string()
            .contains(".png or .svg"));
        assert!(output_path(&d.0.join("no/such/x.png").to_string_lossy(), &["png"])
            .unwrap_err()
            .to_string()
            .contains("does not exist"));
        std::fs::create_dir_all(d.0.join("dir.png")).unwrap();
        assert!(output_path(&d.0.join("dir.png").to_string_lossy(), &["png"])
            .unwrap_err()
            .to_string()
            .contains("is a folder"));
        assert!(output_path("/", &["png"]).is_err());
    }

    #[test]
    fn visuals_are_validated_sanitised_and_written() {
        let _log = crate::activity::test_capture();
        let d = TempDir::new("share-visuals");
        let png = d.0.join("a.png").to_string_lossy().into_owned();
        let saved = block(save_visual(
            OpId(Some("op-visual".into())),
            VisualRequest {
                path: png.clone(),
                kind: "png".into(),
                data: tiny_png(),
                overwrite: false,
            },
        ))
        .unwrap();
        assert_eq!(saved.bytes, 21);
        let e = crate::tools::tests::domain_event("export.written", "op-visual");
        assert_eq!(
            (e.props["Export"].as_str(), e.props["Format"].as_str(), e.props["Bytes"].as_str()),
            ("visual", "png", "21")
        );
        assert!(e.props["Path"].ends_with("/a.png") && !e.props["Path"].contains("share-visuals"));
        let again = |overwrite| {
            block(save_visual(
                OpId::default(),
                VisualRequest {
                    path: png.clone(),
                    kind: "png".into(),
                    data: tiny_png(),
                    overwrite,
                },
            ))
        };
        assert_eq!(again(false).unwrap_err(), format!("{png} already exists; confirm to replace it"));
        assert_eq!(again(true).unwrap().bytes, 21);
        assert!(std::fs::read(&png).unwrap().starts_with(b"\x89PNG"));

        let svg = d.0.join("a.svg").to_string_lossy().into_owned();
        block(save_visual(
            OpId::default(),
            VisualRequest {
                path: svg.clone(),
                kind: "svg".into(),
                data: "<svg><script>alert(1)</script><text>ok</text></svg>".into(),
                overwrite: false,
            },
        ))
        .unwrap();
        let text = std::fs::read_to_string(&svg).unwrap();
        assert!(text.starts_with("<?xml") && text.contains("<text>ok</text>") && !text.contains("script"));

        let bad = |kind: &str, data: &str, path: &str| {
            block(save_visual(
                OpId::default(),
                VisualRequest {
                    path: path.into(),
                    kind: kind.into(),
                    data: data.into(),
                    overwrite: true,
                },
            ))
            .unwrap_err()
        };
        assert!(bad("png", "data:image/gif;base64,AAAA", &png).contains("not a PNG"));
        assert!(bad("png", "data:image/png;base64,!!!", &png).contains("base64"));
        let gif = format!(
            "data:image/png;base64,{}",
            base64::engine::general_purpose::STANDARD.encode(b"GIF89a")
        );
        assert!(bad("png", &gif, &png).contains("not a PNG"));
        assert!(bad("png", &format!("data:image/png;base64,{}", "A".repeat(MAX_VISUAL_BYTES * 2)), &png).contains("limited"));
        assert!(bad("svg", &"a".repeat(MAX_VISUAL_BYTES + 1), &svg).contains("limited"));
        assert!(bad("svg", "<p/>", &svg).contains("<svg> root"));
        assert!(bad("gif", "x", &d.0.join("a.gif").to_string_lossy()).contains("unknown image kind"));
        assert!(bad("png", &tiny_png(), &svg).contains(".png"));
    }

    fn opened(tag: &str) -> (tauri::App<tauri::test::MockRuntime>, TempDir) {
        let d = TempDir::new(tag);
        let app = test_app(Some(d.0.join("profiles")));
        let paths = vec![samples().join("serilog-clef").to_string_lossy().into_owned()];
        block(crate::workspace_commands::open_paths(
            app.handle().clone(),
            main_window(&app),
            paths,
        ))
        .unwrap();
        let toml = "name = 'Shared'\n[redact]\ndetectors = ['email', 'ipv4']\n";
        block(crate::profile::save_profile(
            app.state(),
            main_window(&app),
            OpId::default(),
            toml.into(),
        ))
        .unwrap();
        (app, d)
    }

    fn report_json() -> Value {
        json!({"id": "r1", "title": "Outage <b>", "sections": [{"id": "s", "title": "S", "blocks": [
            {"id": "b", "type": "events", "ids": [0, 1]},
            {"id": "c", "type": "text", "markdown": "**bold** <script>x</script>"}
        ]}]})
    }

    #[test]
    fn redacted_view_previews_and_tiles() {
        let (app, _d) = opened("share-view");
        let st = || main_window(&app);
        let _log = crate::activity::test_capture();
        let summary = block(set_redacted_view(st(), OpId(Some("op-redact".into())), true)).unwrap();
        assert_eq!(
            crate::tools::tests::domain_event("redaction.toggled", "op-redact").props["Enabled"],
            "true"
        );
        assert_eq!(summary["redactedView"], true);
        let summary = block(set_redacted_view(st(), OpId::default(), false)).unwrap();
        assert_eq!(summary["redactedView"], false);
        let p = block(preview_redaction(
            st(),
            RedactionPreviewRequest {
                sample: "a@b.io".into(),
                toml: None,
            },
        ))
        .unwrap();
        assert_eq!(p.after, "<email>");
        let s: RedactionSummaryRequest = serde_json::from_value(json!({})).unwrap();
        assert!(block(redaction_summary(st(), s)).unwrap().scanned > 0);
        let t: TileRequest = serde_json::from_value(json!({"tile": {"id": "t", "type": "stat", "query": "| stats count()"}})).unwrap();
        let data = serde_json::to_value(block(dashboard_tile(st(), t)).unwrap()).unwrap();
        assert_eq!(data["kind"], "stat");
        assert!(data["value"].as_f64().unwrap() > 0.0);
        let bad: TileRequest = serde_json::from_value(json!({"tile": {"id": "t", "type": "stat"}})).unwrap();
        assert!(block(dashboard_tile(st(), bad)).unwrap_err().contains("needs a query"));
    }

    #[test]
    fn reports_export_as_html_and_pdf() {
        let (app, d) = opened("share-report");
        let st = || main_window(&app);
        let req = |format: &str, name: &str, redact: bool| ReportExportRequest {
            report: report_json(),
            format: format.into(),
            path: d.0.join(name).to_string_lossy().into_owned(),
            redact,
            theme: Theme::Dark,
            overwrite: false,
        };
        let _log = crate::activity::test_capture();
        let html = block(export_report(
            app.handle().clone(),
            st(),
            OpId(Some("op-report".into())),
            req("html", "r.html", true),
        ))
        .unwrap();
        assert_eq!(
            crate::tools::tests::domain_event("export.written", "op-report").props["Export"],
            "report"
        );
        let text = std::fs::read_to_string(&html.path).unwrap();
        assert!(text.contains("Outage &lt;b&gt;") && !text.contains("<script"));
        assert!(html.redaction.iter().any(|h| h.rule == "email"));
        let pdf = block(export_report(
            app.handle().clone(),
            st(),
            OpId::default(),
            req("pdf", "r.pdf", false),
        ))
        .unwrap();
        assert!(std::fs::read(&pdf.path).unwrap().starts_with(b"%PDF-"));
        assert!(pdf.redaction.is_empty() && pdf.bytes > 0);
        let again = block(export_report(
            app.handle().clone(),
            st(),
            OpId::default(),
            req("pdf", "r.pdf", false),
        ))
        .unwrap_err();
        assert!(again.ends_with("r.pdf already exists; confirm to replace it"), "{again}");
        let mut confirmed = req("pdf", "r.pdf", false);
        confirmed.overwrite = true;
        assert!(block(export_report(app.handle().clone(), st(), OpId::default(), confirmed)).is_ok());
        assert!(block(export_report(
            app.handle().clone(),
            st(),
            OpId::default(),
            req("docx", "r.docx", false)
        ))
        .unwrap_err()
        .contains("unknown report format"));
        assert!(block(export_report(
            app.handle().clone(),
            st(),
            OpId::default(),
            req("pdf", "r.html", false)
        ))
        .unwrap_err()
        .contains(".pdf"));
        let mut bad = req("html", "x.html", false);
        bad.report = json!({"title": "no id"});
        assert!(block(export_report(app.handle().clone(), st(), OpId::default(), bad))
            .unwrap_err()
            .contains("not a valid report"));
        assert!(parse_report(json!({"id": "x", "title": "a".repeat(MAX_REPORT_JSON)}))
            .unwrap_err()
            .contains("limited"));
    }

    #[test]
    fn bundles_round_trip_and_open_read_only() {
        let (app, d) = opened("share-bundle");
        let st = || main_window(&app);
        let options = |redact: bool| -> BundleOptions {
            serde_json::from_value(
                json!({"name": "Case", "content": "events", "redact": redact, "ui": {"x": 1}, "reports": [report_json()]}),
            )
            .unwrap()
        };
        let est = block(estimate_bundle(st(), options(true))).unwrap();
        assert!(est.events > 0 && est.bytes > 0);
        let path = d.0.join("case.imtz").to_string_lossy().into_owned();
        let _log = crate::activity::test_capture();
        let m = block(create_bundle(
            app.handle().clone(),
            st(),
            OpId(Some("op-bundle".into())),
            options(true),
            path.clone(),
            None,
        ))
        .unwrap();
        assert_eq!(
            crate::tools::tests::domain_event("export.written", "op-bundle").props["Export"],
            "bundle"
        );
        assert!(m.redacted);
        let again = block(create_bundle(
            app.handle().clone(),
            st(),
            OpId::default(),
            options(true),
            path.clone(),
            None,
        ))
        .unwrap_err();
        assert_eq!(again, format!("{path} already exists; confirm to replace it"));
        assert!(block(create_bundle(
            app.handle().clone(),
            st(),
            OpId::default(),
            options(true),
            path.clone(),
            Some(true)
        ))
        .is_ok());
        assert!(block(create_bundle(
            app.handle().clone(),
            st(),
            OpId::default(),
            options(false),
            d.0.join("x.zip").to_string_lossy().into_owned(),
            None
        ))
        .unwrap_err()
        .contains(".imtz"));
        let mut bad = options(false);
        bad.reports = vec![json!({})];
        assert!(block(create_bundle(
            app.handle().clone(),
            st(),
            OpId::default(),
            bad,
            path.clone(),
            Some(true)
        ))
        .is_err());

        let total = block(crate::workspace_commands::workspace_summary(st())).unwrap()["total"].clone();
        let opened = block(open_bundle(app.handle().clone(), st(), OpId::default(), path.clone())).unwrap();
        assert_eq!(opened["readOnly"], true);
        assert_eq!(opened["name"], "Case");
        assert_eq!(opened["ui"]["x"], 1);
        assert_eq!(opened["reports"][0]["id"], "r1");
        assert_eq!(opened["bundle"]["format"], "imt-share");
        assert_eq!(opened["summary"]["total"], total);
        assert!(st().project().is_none());
        // Saving into the bundle's folder is refused.
        let inside = PathBuf::from(opened["folder"].as_str().unwrap()).join("p.imtproj");
        assert!(block(crate::workspace_commands::save_project(
            st(),
            OpId::default(),
            inside.to_string_lossy().into_owned(),
            "x".into(),
            Value::Null,
            None,
            None
        ))
        .unwrap_err()
        .contains("read-only"));

        // A plain zip of logs opens its files.
        let plain = d.0.join("plain.imtz");
        {
            use std::io::Write;
            let mut z = zip::ZipWriter::new(std::fs::File::create(&plain).unwrap());
            z.start_file("a.clef", zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(b"{\"@t\":\"2026-10-14T10:00:00Z\",\"@m\":\"x\"}\n").unwrap();
            z.finish().unwrap();
        }
        let opened = block(open_bundle(
            app.handle().clone(),
            st(),
            OpId::default(),
            plain.to_string_lossy().into_owned(),
        ))
        .unwrap();
        assert_eq!(opened["bundle"], Value::Null);
        assert!(block(open_bundle(
            app.handle().clone(),
            st(),
            OpId::default(),
            d.0.join("missing.imtz").to_string_lossy().into_owned()
        ))
        .is_err());
        assert!(refuse_bundle_folder(&d.0.join("ok.imtproj").to_string_lossy()).is_ok());
    }
}
