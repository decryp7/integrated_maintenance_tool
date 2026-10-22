//! Share a redacted copy (ADR 0041): preview what redaction would change in the window's project and write the project
//! as an `.debriefz` in which every copyable source holds redacted text only, together with the redacted UI state (notes,
//! chats, saved queries, reports). The engine does the work (`Redaction::export_redacted_project`); this module turns
//! the user's choices into a redaction configuration, redacts the UI state the webview sends and checks the target path.
//!
//! Left out of every copy, and said so in the preview: sources in formats that cannot be rebuilt from redacted text,
//! note images and video frames (pictures cannot be redacted), and the profile's own redaction rules.

use crate::activity::{domain_event, privacy, OpId};
use crate::windows::CurrentWindow;
use crate::{CmdError, CmdResult};
use debrief_core::package::ProjectManifest;
use debrief_core::ports::Filter;
use debrief_core::redact::copy::{excluded_reason, ExcludedSource, RedactedExport, RedactedExportRequest};
use debrief_core::redact::{Counts, Detector, Mode, RedactCfg, RedactionSummary, RedactionSummaryRequest, Redactor, Salt};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tauri::{AppHandle, Runtime};

/// What the user picked; every part is optional and falls back to the profile's `[redact]` section.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(crate) struct CopyOptions {
    /// Built-in detectors to run.
    pub detectors: Option<Vec<Detector>>,
    pub mode: Option<Mode>,
    /// Names of the profile's custom `[[redact.rule]]` entries to run.
    pub rules: Option<Vec<String>>,
}

/// The redaction configuration for `options` over the profile's `base`: the picked detectors, mode and custom rules.
/// Refused, saying what to pick, when that leaves nothing to run or names a rule the profile does not have.
pub(crate) fn config_for(base: &RedactCfg, options: &CopyOptions) -> Result<RedactCfg, String> {
    let mut cfg = base.clone();
    if let Some(d) = &options.detectors {
        cfg.detectors = d.clone();
    }
    if let Some(m) = options.mode {
        cfg.mode = m;
    }
    if let Some(names) = &options.rules {
        if let Some(missing) = names.iter().find(|n| !base.rule.iter().any(|r| &r.name == *n)) {
            return Err(format!("the profile has no redaction rule named \"{missing}\""));
        }
        cfg.rule.retain(|r| names.contains(&r.name));
    }
    if cfg.detectors.is_empty() && cfg.rule.is_empty() {
        return Err("pick at least one detector or rule to redact with".into());
    }
    Ok(cfg)
}

/// What a copy would do: counts per rule over the events, and the sources it leaves out.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyPreview {
    pub summary: RedactionSummary,
    pub excluded: Vec<ExcludedSource>,
    /// Sources the copy contains.
    pub included: usize,
}

/// The redaction rules a profile offers, for the picker: the built-in detectors and the profile's custom rules.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CopyChoices {
    pub detectors: Vec<String>,
    pub rules: Vec<String>,
    /// What the profile selects now.
    pub selected: RedactCfg,
}

/// The detectors, rules and defaults the picker shows.
#[tauri::command]
pub(crate) async fn redacted_copy_choices(state: CurrentWindow) -> CmdResult<CopyChoices> {
    let selected = state.host.compiled().profile.redact.clone();
    Ok(CopyChoices {
        detectors: Detector::ALL
            .iter()
            .filter_map(|d| serde_json::to_value(d).ok().and_then(|v| v.as_str().map(String::from)))
            .collect(),
        rules: selected.rule.iter().map(|r| r.name.clone()).collect(),
        selected,
    })
}

/// Counts per rule over the events (at most `redact::view::MAX_SCANNED` of them) with before/after samples, and the
/// sources the copy would leave out.
#[tauri::command]
pub(crate) async fn redacted_copy_preview(state: CurrentWindow, options: CopyOptions) -> CmdResult<CopyPreview> {
    let base = state.host.compiled().profile.redact.clone();
    let cfg = config_for(&base, &options).map_err(|m| CmdError::invalid(m).field("options"))?;
    crate::engine::with_store(&state, move |s| {
        let summary = debrief_core::redact::view::summary(
            s,
            &cfg,
            &RedactionSummaryRequest {
                query: Filter::default(),
                samples: 5,
            },
        )?;
        let sources = s.summary()?.sources;
        let excluded: Vec<ExcludedSource> = sources
            .iter()
            .filter_map(|x| {
                excluded_reason(x).map(|reason| ExcludedSource {
                    name: x.name.clone(),
                    reason,
                })
            })
            .collect();
        Ok(CopyPreview {
            summary,
            included: sources.len() - excluded.len(),
            excluded,
        })
    })
    .await
}

/// Redacts every string of the UI state and reports; the result is `ui.json`'s text.
fn redacted_ui(redactor: &Redactor, ui: &Value, reports: &[Value]) -> Result<String, CmdError> {
    let mut counts = Counts::new();
    let doc = serde_json::json!({
        "version": 1,
        "ui": redactor.apply_json(ui, &mut counts),
        "reports": reports.iter().map(|r| redactor.apply_json(r, &mut counts)).collect::<Vec<_>>(),
    });
    serde_json::to_string(&doc).map_err(|e| CmdError::new(crate::ErrorCode::Internal, e.to_string()))
}

/// Writes the window's project as `<name>.debriefz` with everything redacted (see the module docs). Returns what it holds
/// and what it left out.
#[tauri::command]
#[allow(clippy::too_many_arguments)] // the command's arguments are its wire contract
pub(crate) async fn export_redacted_copy<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    grants: crate::paths::WindowGrants,
    op: OpId,
    path: String,
    name: String,
    options: CopyOptions,
    ui: Value,
    reports: Option<Vec<Value>>,
    overwrite: Option<bool>,
) -> CmdResult<RedactedExport> {
    let _ = &app;
    crate::share::refuse_bundle_folder(&path)?;
    let target = grants.write(&path, &[debrief_core::package::ARCHIVE_EXTENSION])?;
    let file = target.path().to_path_buf();
    debrief_core::overwrite::check_overwrite(&file, overwrite.unwrap_or(false))?;
    let base = state.host.compiled().profile.redact.clone();
    let cfg = config_for(&base, &options).map_err(|m| CmdError::invalid(m).field("options"))?;
    let redactor = Redactor::from_config(&cfg, Salt::random()).map_err(|e| CmdError::invalid(format!("{e:#}")))?;
    let ui_json = redacted_ui(&redactor, &ui, &reports.unwrap_or_default())?;
    let mut counts = Counts::new();
    let manifest = ProjectManifest {
        name: redactor.apply(&name, &mut counts).into_owned(),
        profile_overrides: state.profile_doc().toml,
        ..ProjectManifest::default()
    };
    let request = RedactedExportRequest {
        cfg,
        manifest,
        out: file,
        ui_json: Some(ui_json),
    };
    let report = crate::engine::with_store(&state, move |s| s.export_redacted_project(&request)).await?;
    let safe = privacy::safe_path(std::path::Path::new(&path));
    domain_event("project.shared-redacted", op.as_deref(), &[("Path", &safe)]);
    log::info!(Sources = report.sources as u64, Excluded = report.excluded.len() as u64, Bytes = report.bytes; "Redacted copy written");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_core::redact::RedactRuleCfg;

    fn base() -> RedactCfg {
        RedactCfg {
            detectors: vec![Detector::Email],
            rule: vec![
                RedactRuleCfg {
                    name: "a".into(),
                    regex: "aaa".into(),
                    replace: None,
                },
                RedactRuleCfg {
                    name: "b".into(),
                    regex: "bbb".into(),
                    replace: None,
                },
            ],
            ..RedactCfg::default()
        }
    }

    #[test]
    fn choices_override_the_profile_and_are_checked() {
        let none = config_for(&base(), &CopyOptions::default()).unwrap();
        assert_eq!((none.detectors.len(), none.rule.len()), (1, 2));
        let picked = config_for(
            &base(),
            &CopyOptions {
                detectors: Some(vec![Detector::Ipv4, Detector::Jwt]),
                mode: Some(Mode::Hash),
                rules: Some(vec!["b".into()]),
            },
        )
        .unwrap();
        assert_eq!(
            (picked.detectors.len(), picked.mode, picked.rule.len(), picked.rule[0].name.as_str()),
            (2, Mode::Hash, 1, "b")
        );
        let e = config_for(
            &base(),
            &CopyOptions {
                rules: Some(vec!["zzz".into()]),
                ..CopyOptions::default()
            },
        )
        .unwrap_err();
        assert!(e.contains("zzz"));
        let e = config_for(
            &base(),
            &CopyOptions {
                detectors: Some(vec![]),
                rules: Some(vec![]),
                ..CopyOptions::default()
            },
        )
        .unwrap_err();
        assert!(e.contains("at least one"));
    }

    #[test]
    fn the_ui_state_is_redacted_everywhere_and_stays_valid_json() {
        let r = Redactor::from_config(&base(), Salt::fixed([1; 16])).unwrap();
        let ui = serde_json::json!({"notes": [{"text": "mail jo@invented.test now"}], "chats": [["jo@invented.test"]], "n": 3});
        let reports = vec![serde_json::json!({"title": "by jo@invented.test"})];
        let out = redacted_ui(&r, &ui, &reports).unwrap();
        assert!(!out.contains("invented.test"), "{out}");
        let v: Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["ui"]["n"], 3);
        assert_eq!(v["version"], 1);
    }

    #[test]
    fn a_window_previews_and_writes_a_copy_without_the_secret() {
        use crate::test_support::{block, main_window, test_app, TempDir};
        let dir = TempDir::new("redcopy-cmd");
        std::fs::write(
            dir.0.join("app.log"),
            "2026-10-14 10:00:00 INFO login carol.invented@example.test\n2026-10-14 10:00:01 INFO ok\n",
        )
        .unwrap();
        let app = test_app(None);
        crate::paths::allow_under(app.handle(), "main", &dir.0);
        block(crate::workspace_commands::open_paths(
            app.handle().clone(),
            main_window(&app),
            vec![dir.0.join("app.log").to_string_lossy().into_owned()],
        ))
        .unwrap();
        let options = || CopyOptions {
            detectors: Some(vec![Detector::Email]),
            ..CopyOptions::default()
        };
        let preview = block(redacted_copy_preview(main_window(&app), options())).unwrap();
        assert_eq!((preview.included, preview.excluded.len(), preview.summary.changed), (1, 0, 1));
        let refused = block(redacted_copy_preview(
            main_window(&app),
            CopyOptions {
                detectors: Some(vec![]),
                ..CopyOptions::default()
            },
        ))
        .unwrap_err();
        assert!(refused.message.contains("at least one"));
        let out = dir.0.join("shared.debriefz");
        let report = block(export_redacted_copy(
            app.handle().clone(),
            main_window(&app),
            crate::paths::WindowGrants::of(app.handle(), "main"),
            OpId::default(),
            out.to_string_lossy().into_owned(),
            "Shared".into(),
            options(),
            serde_json::json!({"note": "carol.invented@example.test"}),
            None,
            None,
        ))
        .unwrap();
        assert_eq!((report.sources, report.events), (1, 2));
        let bytes = std::fs::read(&out).unwrap();
        assert!(!bytes.windows(7).any(|w| w == b"invente"));
    }
}
