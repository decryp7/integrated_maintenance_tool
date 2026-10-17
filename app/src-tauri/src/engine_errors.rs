//! Finds the failures the engine contained during a workspace command (plugin
//! crashes, quarantines, files whose import crashed) so the UI can be told
//! about each new one once, via `imt://error`.

use crate::snapshots::{EngineError, EngineErrorKind};
use imt_core::api::WorkspaceSummary;
use imt_core::plugin::PluginStatus;
use std::collections::{BTreeMap, BTreeSet};

/// At most this many errors are reported per command (a folder of broken files
/// must not flood the UI with snapshots).
pub const MAX_REPORTED: usize = 5;

/// What the guard text of a contained panic contains (see `imt_core::guard::catch`).
const CRASH_MARK: &str = " crashed: ";

#[derive(Debug, Clone, PartialEq, Eq)]
struct PluginHealth {
    name: String,
    status: PluginStatus,
    failures: u32,
    last_error: Option<String>,
    inactive_reason: Option<String>,
}

/// The parts of a workspace summary that record contained failures.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HealthSnapshot {
    plugins: BTreeMap<String, PluginHealth>,
    /// (file name, reason) of files whose import crashed.
    crashed_files: BTreeSet<(String, String)>,
}

impl HealthSnapshot {
    pub fn of(summary: &WorkspaceSummary) -> HealthSnapshot {
        HealthSnapshot {
            plugins: summary
                .plugins
                .iter()
                .map(|p| {
                    (
                        p.id.clone(),
                        PluginHealth {
                            name: p.name.clone(),
                            status: p.status,
                            failures: p.failures,
                            last_error: p.last_error.clone(),
                            inactive_reason: p.inactive_reason.clone(),
                        },
                    )
                })
                .collect(),
            crashed_files: summary
                .unrecognized
                .iter()
                .filter(|u| u.reason.contains(CRASH_MARK))
                .map(|u| (u.name.clone(), u.reason.clone()))
                .collect(),
        }
    }
}

/// Failures present in `after` but not in `before`, most severe first, capped at [`MAX_REPORTED`].
pub fn new_errors(before: &HealthSnapshot, after: &HealthSnapshot) -> Vec<EngineError> {
    let mut out = Vec::new();
    for (id, now) in &after.plugins {
        let prev = before.plugins.get(id);
        let was_quarantined = prev.is_some_and(|p| p.status == PluginStatus::Quarantined);
        let failures_before = prev.map_or(0, |p| p.failures);
        let detail = now.last_error.clone().unwrap_or_else(|| "it crashed".into());
        if now.status == PluginStatus::Quarantined && !was_quarantined {
            out.push(EngineError {
                kind: EngineErrorKind::PluginQuarantined,
                message: format!("Plugin \"{}\" was disabled after repeated crashes: {detail}", now.name),
                plugin: Some(id.clone()),
            });
        } else if now.status == PluginStatus::Inactive && prev.is_some_and(|p| p.status != PluginStatus::Inactive) {
            let reason = now
                .inactive_reason
                .clone()
                .unwrap_or_else(|| "a plugin it needs is not running".into());
            out.push(EngineError {
                kind: EngineErrorKind::PluginInactive,
                message: format!("Plugin \"{}\" stopped: {reason}", now.name),
                plugin: Some(id.clone()),
            });
        } else if now.failures > failures_before && !was_quarantined {
            out.push(EngineError {
                kind: EngineErrorKind::PluginCrash,
                message: format!("Plugin \"{}\" crashed: {detail}", now.name),
                plugin: Some(id.clone()),
            });
        }
    }
    for (name, reason) in after.crashed_files.difference(&before.crashed_files) {
        out.push(EngineError {
            kind: EngineErrorKind::ImportCrash,
            message: format!("Importing {name} crashed: {reason}"),
            plugin: None,
        });
    }
    out.truncate(MAX_REPORTED);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::api::UnrecognizedFile;
    use imt_core::plugin::PluginInfo;

    fn plugin(id: &str, status: PluginStatus, failures: u32, last_error: Option<&str>) -> PluginInfo {
        PluginInfo {
            id: id.into(),
            name: format!("{id} plugin"),
            version: "1".into(),
            contributes: vec![],
            status,
            failures,
            last_error: last_error.map(String::from),
            requires: vec![],
            provides: vec![],
            inactive_reason: None,
            dependents: vec![],
            warnings: vec![],
            hung: false,
        }
    }

    fn file(name: &str, reason: &str) -> UnrecognizedFile {
        UnrecognizedFile {
            path: format!("/x/{name}"),
            name: name.into(),
            reason: reason.into(),
        }
    }

    fn snap(plugins: Vec<PluginInfo>, unrecognized: Vec<UnrecognizedFile>) -> HealthSnapshot {
        let mut s = imt_core::Workspace::new(imt_core::Profile::default_profile()).unwrap().summary();
        s.plugins = plugins;
        s.unrecognized = unrecognized;
        HealthSnapshot::of(&s)
    }

    #[test]
    fn nothing_new_means_nothing_reported() {
        let a = snap(
            vec![plugin("p", PluginStatus::Active, 1, Some("boom"))],
            vec![file("a.log", "parser crashed: x")],
        );
        assert!(new_errors(&a, &a.clone()).is_empty());
        assert!(new_errors(&HealthSnapshot::default(), &HealthSnapshot::default()).is_empty());
    }

    #[test]
    fn reports_new_crashes_quarantines_and_import_crashes_only() {
        let before = snap(
            vec![
                plugin("a", PluginStatus::Active, 0, None),
                plugin("b", PluginStatus::Active, 2, Some("old")),
                plugin("q", PluginStatus::Quarantined, 3, Some("old")),
            ],
            vec![file("old.log", "IMT crashed: old")],
        );
        let after = snap(
            vec![
                plugin("a", PluginStatus::Active, 1, Some("index out of bounds")),
                plugin("b", PluginStatus::Quarantined, 3, None),
                plugin("q", PluginStatus::Quarantined, 4, Some("again")),
                plugin("new", PluginStatus::Active, 1, None),
                plugin("d", PluginStatus::Disabled, 0, None),
            ],
            vec![
                file("old.log", "IMT crashed: old"),
                file("new.log", "IMT crashed: overflow (please report this file)"),
                file("text.bin", "no importer recognised the file"),
            ],
        );
        let got: Vec<(EngineErrorKind, Option<String>, String)> = new_errors(&before, &after)
            .into_iter()
            .map(|e| (e.kind, e.plugin, e.message))
            .collect();
        assert_eq!(
            got,
            vec![
                (
                    EngineErrorKind::PluginCrash,
                    Some("a".into()),
                    "Plugin \"a plugin\" crashed: index out of bounds".into()
                ),
                (
                    EngineErrorKind::PluginQuarantined,
                    Some("b".into()),
                    "Plugin \"b plugin\" was disabled after repeated crashes: it crashed".into()
                ),
                (
                    EngineErrorKind::PluginCrash,
                    Some("new".into()),
                    "Plugin \"new plugin\" crashed: it crashed".into()
                ),
                (
                    EngineErrorKind::ImportCrash,
                    None,
                    "Importing new.log crashed: IMT crashed: overflow (please report this file)".into()
                ),
            ]
        );
    }

    #[test]
    fn reports_a_plugin_that_became_inactive_once() {
        let mut dependent = plugin("dep", PluginStatus::Active, 0, None);
        let before = snap(vec![dependent.clone(), plugin("was", PluginStatus::Inactive, 0, None)], vec![]);
        dependent.status = PluginStatus::Inactive;
        dependent.inactive_reason = Some("needs q ^1.0 (q is quarantined)".into());
        let mut no_reason = plugin("bare", PluginStatus::Inactive, 0, None);
        let after = snap(vec![dependent.clone(), plugin("was", PluginStatus::Inactive, 0, None)], vec![]);
        let got = new_errors(&before, &after);
        assert_eq!(got.len(), 1, "only the plugin that changed, not one inactive from the start");
        assert_eq!(got[0].kind, EngineErrorKind::PluginInactive);
        assert_eq!(got[0].message, "Plugin \"dep plugin\" stopped: needs q ^1.0 (q is quarantined)");
        assert!(new_errors(&after, &after).is_empty());
        // Without a stated reason.
        let before = snap(vec![plugin("bare", PluginStatus::Active, 0, None)], vec![]);
        no_reason.failures = 0;
        let got = new_errors(&before, &snap(vec![no_reason], vec![]));
        assert!(got[0].message.ends_with("a plugin it needs is not running"));
        assert_eq!(serde_json::to_value(EngineErrorKind::PluginInactive).unwrap(), "plugin-inactive");
    }

    #[test]
    fn caps_the_number_of_reports() {
        let files = (0..20).map(|i| file(&format!("{i}.log"), "IMT crashed: x")).collect();
        assert_eq!(new_errors(&HealthSnapshot::default(), &snap(vec![], files)).len(), MAX_REPORTED);
    }
}
