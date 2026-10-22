//! Known-issue commands: the rules matched by the engine, and AI suggestions the engine verified.

use super::knowledge_texts;
use crate::ai::commands::core;
use crate::ai::error::AiResult;
use crate::engine::with_store;
use crate::windows::CurrentWindow;
use crate::CmdResult;
use imt_ai::StoreAccess;
use imt_core::known_issues::{match_known_issues, IssueMatch, IssueReport, KnowledgeText};
use tauri::{AppHandle, Runtime};

/// Matches the known issues declared in the enabled knowledge files against the open data (no AI).
#[tauri::command]
pub async fn known_issues_match<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> CmdResult<IssueReport> {
    let texts = knowledge_texts(&app, win.arc());
    with_store(&win, move |s| {
        let files: Vec<KnowledgeText<'_>> = texts.iter().map(|(name, text)| KnowledgeText { name, text }).collect();
        match_known_issues(s, &files)
    })
    .await
}

/// A digest of the data's problems for the model: the exception types and error messages with counts.
fn problems_digest(store: &dyn imt_core::ports::Store) -> String {
    let req = imt_core::api::QueryRequest {
        query: imt_core::query::Query {
            ql: "level >= error".into(),
            ..Default::default()
        },
    };
    let mut out = String::new();
    for g in store.exception_groups(&req).map(|a| a.result).unwrap_or_default().iter().take(8) {
        out.push_str(&format!("- exception {}: {} ({} events)\n", g.type_name, g.message.lines().next().unwrap_or(""), g.count));
    }
    for p in store.patterns(&req).map(|a| a.result).unwrap_or_default().iter().take(8) {
        out.push_str(&format!("- message: {} ({} events)\n", p.template, p.count));
    }
    out
}

/// The AI's suggestions of which knowledge notes describe the problems in the data. Each is verified by the engine
/// (the fix is quoted from a note, the query is valid and selects events); without a model this says so.
#[tauri::command]
pub async fn known_issues_suggest<R: Runtime>(app: AppHandle<R>, win: CurrentWindow) -> AiResult<Vec<IssueMatch>> {
    let ai = core(&app)?;
    ai.availability()?;
    let texts = knowledge_texts(&app, win.arc());
    let w = win.arc().clone();
    let a2 = app.clone();
    crate::sync::blocking("the known-issue task", move || {
        let files: Vec<KnowledgeText<'_>> = texts.iter().map(|(name, text)| KnowledgeText { name, text }).collect();
        let cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let known = crate::ai::knowledge::for_window(&w, &Default::default());
        let assistant = core(&a2)?.assistant(&known, cancel)?;
        let access = crate::ai::commands::WindowStore(w.clone());
        let store: imt_ai::StoreRef<'_> = (&access as &dyn StoreAccess).into();
        let digest = store.with(problems_digest);
        Ok(assistant.suggest_known_issues(store, &files, &digest)?)
    })
    .await
}
