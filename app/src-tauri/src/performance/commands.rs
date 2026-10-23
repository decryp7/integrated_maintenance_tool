//! The commands of Settings ▸ Performance (wire contract: `app/src/app/settings/performance/performance.types.ts`).
//! Suggest settings and the presets only propose values (queries); applying them goes through `set_settings`, which
//! validates them like any edit.

use super::live::Live;
use super::section::Preset;
use super::suggest::{probe, proposal, suggest, Proposal};
use super::watch::{MemoryState, Seen};
use crate::app_state::AppState;
use crate::error::{CmdError, CmdResult};
use crate::notification_centre::centre::ProcessRef;
use serde::Serialize;
use std::sync::Arc;
use tauri::{AppHandle, Manager, Runtime, State};

fn live(state: &AppState) -> Arc<Live> {
    state.performance.clone()
}

/// The active AI model's size with its projector, bytes (0 without one).
fn ai_model_bytes<R: Runtime>(app: &AppHandle<R>) -> u64 {
    app.try_state::<crate::ai::AiState>().map_or(0, |ai| {
        ai.0.models()
            .into_iter()
            .filter(|m| m.active)
            .map(|m| m.size_bytes + m.projector.as_ref().filter(|p| p.problem.is_none()).map_or(0, |p| p.size_bytes))
            .sum()
    })
}

/// Suggested values for this computer, next to the current ones.
#[tauri::command]
pub async fn performance_suggest<R: Runtime>(app: AppHandle<R>, state: State<'_, AppState>) -> CmdResult<Proposal> {
    let live = live(&state);
    let current = live.settings();
    let data = crate::app_dirs::data(&app).unwrap_or_else(|_| std::env::temp_dir());
    let env = probe(&data, ai_model_bytes(&app));
    Ok(proposal(Some(env.clone()), &current, &suggest(&env, &current)))
}

/// A preset's values (Standard, Low memory), next to the current ones.
#[tauri::command]
pub async fn performance_preset(state: State<'_, AppState>, preset: Preset) -> CmdResult<Proposal> {
    let current = live(&state).settings();
    Ok(proposal(None, &current, &current.with_values(&preset.values())))
}

/// One capped process: its cap and what the memory watcher last said of it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProcessMemory {
    pub process: ProcessRef,
    /// The plugin's name (plugins only).
    pub name: Option<String>,
    /// The cap its next start gets, MB.
    pub cap_mb: u64,
    /// The cap the user set for this plugin in the Plugins panel, MB.
    pub override_mb: Option<u32>,
    /// What it used and its peak at the watcher's last report, MB (none before one).
    pub used_mb: Option<u64>,
    pub peak_mb: Option<u64>,
    pub state: Option<MemoryState>,
}

/// The capped processes (store worker, AI worker, each plugin) with their caps and last reported use.
pub fn memory(live: &Live) -> Vec<ProcessMemory> {
    let s = live.settings();
    let seen: Vec<Seen> = live.hub.seen();
    let of = |p: &ProcessRef| seen.iter().find(|x| x.process == *p);
    let row = |process: ProcessRef, name: Option<String>, cap_mb: u64, override_mb: Option<u32>| {
        let last = of(&process);
        ProcessMemory {
            used_mb: last.map(|l| l.used_mb),
            peak_mb: last.map(|l| l.peak_mb),
            state: last.map(|l| l.state),
            process,
            name,
            cap_mb,
            override_mb,
        }
    };
    let mut rows = vec![
        row(ProcessRef::StoreWorker, None, u64::from(s.store_cap_mb), None),
        row(ProcessRef::Ai, None, u64::from(s.ai_worker_memory_mb), None),
    ];
    if let Some(plugins) = live.plugins() {
        for p in plugins.processes() {
            let id = p.id().to_string();
            rows.push(row(
                ProcessRef::Plugin { plugin_id: id.clone() },
                Some(p.manifest().name.to_string()),
                super::watch::mb(p.memory_cap()),
                s.plugin_memory.get(&id).copied(),
            ));
        }
    }
    rows
}

#[tauri::command]
pub async fn performance_memory(state: State<'_, AppState>) -> CmdResult<Vec<ProcessMemory>> {
    Ok(memory(&live(&state)))
}

/// Sets (`mb`) or removes (`null`) plugin `pluginId`'s memory cap; it applies from the plugin's next start.
#[tauri::command]
pub async fn performance_set_plugin_memory(
    state: State<'_, AppState>,
    op: crate::activity::OpId,
    plugin_id: String,
    mb: Option<u32>,
) -> CmdResult<Vec<ProcessMemory>> {
    let live = live(&state);
    live.set_plugin_cap(&plugin_id, mb).map_err(CmdError::invalid)?;
    crate::activity::domain_event("performance.pluginCap", op.as_deref(), &[("Plugin", &plugin_id)]);
    Ok(memory(&live))
}

/// Lets the store worker start again after the watcher stopped it (its crash count is reset; the next action starts
/// it with the current settings).
#[tauri::command]
pub async fn performance_retry_store_worker(state: State<'_, AppState>, op: crate::activity::OpId) -> CmdResult<()> {
    if let Some(w) = live(&state).worker() {
        w.reset();
    }
    crate::activity::domain_event("performance.retryStoreWorker", op.as_deref(), &[]);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::performance::PerformanceSettings;

    #[test]
    fn memory_lists_the_worker_the_ai_and_each_plugin_with_their_caps() {
        let mut s = PerformanceSettings::default();
        s.plugin_memory.insert("p.one".into(), 300);
        let live = Live::with(None, s, 8);
        let rows = memory(&live);
        assert_eq!(rows.len(), 2);
        assert_eq!((rows[0].process.clone(), rows[0].cap_mb), (ProcessRef::StoreWorker, 2048));
        assert_eq!(
            (rows[1].process.clone(), rows[1].cap_mb, rows[1].peak_mb),
            (ProcessRef::Ai, 1024, None)
        );
        let m: debrief_plugin_api::manifest::Manifest = serde_json::from_value(serde_json::json!({
            "id": "p.one", "name": "One", "version": "1.0.0", "protocol": debrief_plugin_api::PROTOCOL,
            "permissions": [], "importers": [], "configParsers": [], "markerKinds": [], "annotators": [],
            "transforms": [], "tools": [], "profileImporters": [], "documentImporters": []
        }))
        .unwrap();
        live.attach_plugins(Arc::new(debrief_plugin_host::Builtins::from_manifests(
            std::path::Path::new("/x"),
            vec![m],
        )));
        live.hub.hear(
            "plugin \"p.one\"",
            debrief_ipc::memory_watch::MemoryEvent::Warning(debrief_ipc::memory_watch::MemoryUse {
                used: 250 << 20,
                peak: 260 << 20,
                limit: 300 << 20,
            }),
        );
        let plugin = memory(&live).pop().unwrap();
        assert_eq!(plugin.name.as_deref(), Some("One"));
        assert_eq!(
            (plugin.cap_mb, plugin.override_mb, plugin.used_mb, plugin.peak_mb),
            (300, Some(300), Some(250), Some(260))
        );
        assert_eq!(plugin.state, Some(MemoryState::Warned));
        let wire = serde_json::to_value(&plugin).unwrap();
        assert_eq!(wire["process"], serde_json::json!({"kind": "plugin", "pluginId": "p.one"}));
        assert_eq!(wire["state"], "warned");
    }
}
