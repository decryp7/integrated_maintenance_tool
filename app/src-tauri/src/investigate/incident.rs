//! The incident summary: the engine builds the report; the model may add a marked narrative.

use super::{knowledge_texts, now_ms};
use crate::ai::commands::core;
use crate::ai::error::{AiCmdError, AiResult};
use crate::engine::with_store;
use crate::windows::CurrentWindow;
use crate::CmdResult;
use imt_ai::{StoreAccess, StoreRef};
use imt_core::incident::{build_incident_report, digest, IncidentOptions};
use imt_core::known_issues::{match_known_issues, KnowledgeText};
use imt_core::report::Report;
use serde::Deserialize;
use serde_json::Value;
use tauri::{AppHandle, Runtime};

/// What the user chose for the summary.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IncidentRequest {
    #[serde(default)]
    pub title: String,
    pub from: Option<i64>,
    pub to: Option<i64>,
    pub top: Option<usize>,
}

/// The incident summary of the open data as a report (known issues matched by the rules, the project's notes, every
/// ranking with evidence). The UI saves it with the project's reports; export and redaction are the report pipeline's.
#[tauri::command]
pub async fn incident_report_build<R: Runtime>(app: AppHandle<R>, win: CurrentWindow, request: IncidentRequest) -> CmdResult<Report> {
    if request.title.chars().count() > 200 {
        return Err(crate::CmdError::invalid("the title is longer than 200 characters"));
    }
    if let (Some(f), Some(t)) = (request.from, request.to) {
        if t < f {
            return Err(crate::CmdError::invalid("the end of the period is before its start"));
        }
    }
    let texts = knowledge_texts(&app, win.arc());
    let notes = crate::notes::ai_file(&win.notes.snapshot(), &Default::default())
        .map(|f| f.text)
        .unwrap_or_default();
    let now = now_ms();
    with_store(&win, move |s| {
        let files: Vec<KnowledgeText<'_>> = texts.iter().map(|(name, text)| KnowledgeText { name, text }).collect();
        let issues = match_known_issues(s, &files)?.matches;
        build_incident_report(
            s,
            &IncidentOptions {
                id: format!("incident-{now}"),
                title: request.title,
                from: request.from,
                to: request.to,
                top: request.top,
                notes,
                issues,
                now,
            },
        )
    })
    .await
}

/// The optional narrative: written by the model from the engine's report digest, citations checked. The UI shows it
/// as an AI-written block, clearly marked. Without a model this says so.
#[tauri::command]
pub async fn incident_narrative<R: Runtime>(
    app: AppHandle<R>,
    win: CurrentWindow,
    report: Value,
) -> AiResult<imt_ai::assistant::Narrative> {
    let report = Report::parse(report).map_err(|e| AiCmdError::invalid(format!("{e:#}")))?;
    let ai = core(&app)?;
    ai.availability()?;
    let w = win.arc().clone();
    let a2 = app.clone();
    crate::sync::blocking("the narrative task", move || {
        let (text, ids) = digest(&report, 12_000);
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let known = crate::ai::knowledge::for_window(&w, &Default::default());
        let assistant = core(&a2)?.assistant(&known, cancel)?;
        let access = crate::ai::commands::WindowStore(w.clone());
        let store: StoreRef<'_> = (&access as &dyn StoreAccess).into();
        Ok(assistant.incident_narrative(store, &text, &ids)?)
    })
    .await
}
