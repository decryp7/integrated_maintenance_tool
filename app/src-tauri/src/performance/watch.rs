//! What the memory watcher tells the app (ADR 0031 §5): each capped process' use and peak, a quiet message at 80% of
//! its cap; a stop is service health's (the process is down, with *Raise the cap…* advice and Try again). The messages and
//! the stop are built by pure functions; [`MemoryHub`] keeps the peaks and posts through whatever the app attached (the notification centre).

use crate::health::model::Cause;
use crate::notification_centre::centre::{NewNotice, NoticeLink, NoticeScope, NoticeSeverity, NoticeSource, ProcessRef};
use crate::sync::lock;
use debrief_ipc::memory_watch::{MemoryEvent, MemoryListener, MemoryUse};
use serde::Serialize;
use std::collections::{BTreeMap, HashMap};
use std::sync::{Arc, Mutex, Weak};

const MB: u64 = 1 << 20;

/// The process a watcher label names (`the store worker`, `the AI engine`, `plugin "id"`); an elevated plugin process
/// (which caps itself) and unknown labels name none.
pub fn process_of(label: &str) -> Option<ProcessRef> {
    match label {
        "the store worker" => Some(ProcessRef::StoreWorker),
        "the AI engine" => Some(ProcessRef::Ai),
        _ => {
            let id = label.strip_prefix("plugin \"")?.strip_suffix('"')?;
            (!id.is_empty() && !id.contains('"')).then(|| ProcessRef::Plugin { plugin_id: id.into() })
        }
    }
}

/// Bytes as whole MB, rounded up (as the watcher's own messages).
pub fn mb(bytes: u64) -> u64 {
    bytes.div_ceil(MB)
}

/// The cap suggested after a stop: the peak and half again, rounded up to 256 MB, within `max_mb`.
pub fn suggested_cap_mb(peak_bytes: u64, max_mb: u32) -> u32 {
    let wanted = (mb(peak_bytes) * 3 / 2).div_ceil(256) * 256;
    u32::try_from(wanted).unwrap_or(u32::MAX).clamp(256, max_mb)
}

/// The largest cap the setting of `process` accepts, MB.
pub fn max_cap_mb(process: &ProcessRef) -> u32 {
    match process {
        ProcessRef::StoreWorker => 65_536,
        ProcessRef::Ai => 16_384,
        ProcessRef::Plugin { .. } => super::section::MAX_PLUGIN_MB,
    }
}

/// How a message names the process: "The database worker", "The AI worker", "Plugin 'Event logs'".
pub fn display_name(process: &ProcessRef, plugin_name: Option<&str>) -> String {
    match process {
        ProcessRef::StoreWorker => "The database worker".into(),
        ProcessRef::Ai => "The AI worker".into(),
        ProcessRef::Plugin { plugin_id } => format!("Plugin '{}'", plugin_name.unwrap_or(plugin_id)),
    }
}

fn key(process: &ProcessRef) -> String {
    match process {
        ProcessRef::StoreWorker => "storeWorker".into(),
        ProcessRef::Ai => "ai".into(),
        ProcessRef::Plugin { plugin_id } => format!("plugin:{plugin_id}"),
    }
}

/// The message for `event` of `process` (none for a normal end, which only updates the peak).
pub fn notice(process: &ProcessRef, name: &str, event: &MemoryEvent) -> Option<NewNotice> {
    let link = NoticeLink {
        plugin_id: match process {
            ProcessRef::Plugin { plugin_id } => Some(plugin_id.clone()),
            _ => None,
        },
        ..NoticeLink::default()
    };
    match event {
        MemoryEvent::Warning(u) => Some(
            NewNotice::new(
                format!("memory:warning:{}", key(process)),
                NoticeSource::Performance,
                NoticeSeverity::Info,
                format!("{name} is using {} of {} MB", mb(u.used), mb(u.limit)),
                NoticeScope::App,
            )
            .body("It is close to its memory cap. If it needs more, Debrief stops it; the cap is in Settings ▸ Performance.")
            .link(link)
            .quiet(),
        ),
        // A stop is service health's to state (see [`stop`]), once, with Raise the cap… and Try again.
        MemoryEvent::Stopped(_) => None,
        MemoryEvent::Ended(_) => None,
    }
}

/// What a stop over the memory cap means for service health: which service, the plain cause with the advice to raise the cap.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stop {
    /// The service the process belongs to (`store`, `ai`, `plugins`).
    pub service: &'static str,
    /// The plugin, when the process is one.
    pub plugin: Option<String>,
    pub cause: Cause,
}

/// The stop of `process` (`name` as [`display_name`]) that used `u`.
pub fn stop(process: &ProcessRef, name: &str, u: &MemoryUse) -> Stop {
    let suggested = suggested_cap_mb(u.peak.max(u.used), max_cap_mb(process));
    let after = match process {
        ProcessRef::StoreWorker => " The project opens again with the next action.",
        _ => "",
    };
    let (service, plugin) = match process {
        ProcessRef::StoreWorker => (crate::health::hub::STORE, None),
        ProcessRef::Ai => (crate::health::hub::AI, None),
        ProcessRef::Plugin { plugin_id } => (crate::health::hub::PLUGINS, Some(plugin_id.clone())),
    };
    let cause = Cause::new(format!("{name} was stopped: it used more than its {} MB memory cap", mb(u.limit)))
        .hint(format!(
            "It reached {} MB. Raise the cap to about {suggested} MB, or try again if this was a one-off.{after}",
            mb(u.used.max(u.peak))
        ))
        .raise_cap(key(process), suggested);
    Stop { service, plugin, cause }
}

/// How a process ended its last report.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MemoryState {
    /// Past 80% of its cap.
    Warned,
    /// Stopped over its cap.
    Stopped,
    /// Ended on its own.
    Ended,
}

/// The last the watcher said about one process.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Seen {
    pub process: ProcessRef,
    pub used_mb: u64,
    pub peak_mb: u64,
    pub cap_mb: u64,
    pub state: MemoryState,
}

/// Posts a message (the notification centre once the app is up).
pub type Poster = Box<dyn Fn(NewNotice) + Send + Sync>;

/// The app's end of every watcher: the peaks it reported and the messages it posts.
#[derive(Default)]
pub struct MemoryHub {
    seen: Mutex<BTreeMap<ProcessRef, Seen>>,
    names: Mutex<HashMap<String, String>>,
    poster: Mutex<Option<Poster>>,
}

impl MemoryHub {
    pub fn new() -> Arc<MemoryHub> {
        Arc::new(MemoryHub::default())
    }

    /// The listener to pass to a process' spec (it holds the hub weakly).
    pub fn listener(self: &Arc<MemoryHub>) -> MemoryListener {
        let hub: Weak<MemoryHub> = Arc::downgrade(self);
        MemoryListener::new(move |label, event| {
            if let Some(h) = hub.upgrade() {
                h.hear(label, event);
            }
        })
    }

    /// Where messages go from now on.
    pub fn attach(&self, poster: Poster) {
        *lock(&self.poster) = Some(poster);
    }

    /// The display names of plugins, by id.
    pub fn set_names(&self, names: HashMap<String, String>) {
        *lock(&self.names) = names;
    }

    /// One event of the process `label` names: its peak is kept and its message posted.
    pub fn hear(&self, label: &str, event: MemoryEvent) {
        let Some(process) = process_of(label) else { return };
        let MemoryUse { used, peak, limit } = event.usage();
        let state = match event {
            MemoryEvent::Warning(_) => MemoryState::Warned,
            MemoryEvent::Stopped(_) => MemoryState::Stopped,
            MemoryEvent::Ended(_) => MemoryState::Ended,
        };
        lock(&self.seen).insert(
            process.clone(),
            Seen {
                process: process.clone(),
                used_mb: mb(used),
                peak_mb: mb(peak.max(used)),
                cap_mb: mb(limit),
                state,
            },
        );
        match &event {
            MemoryEvent::Stopped(_) => {
                log::warn!(Process = key(&process).as_str(), PeakMb = mb(peak.max(used)), CapMb = mb(limit); "Process stopped over its memory cap")
            }
            MemoryEvent::Warning(_) => {
                log::info!(Process = key(&process).as_str(), UsedMb = mb(used), CapMb = mb(limit); "Process near its memory cap")
            }
            MemoryEvent::Ended(_) => {}
        }
        let name = {
            let names = lock(&self.names);
            let plugin = match &process {
                ProcessRef::Plugin { plugin_id } => names.get(plugin_id).cloned(),
                _ => None,
            };
            display_name(&process, plugin.as_deref())
        };
        if let MemoryEvent::Stopped(u) = &event {
            crate::health::hub::report_memory_stopped(&stop(&process, &name, u));
        }
        if let (Some(n), Some(post)) = (notice(&process, &name, &event), lock(&self.poster).as_ref()) {
            post(n);
        }
    }

    /// What the watchers last said, by process.
    pub fn seen(&self) -> Vec<Seen> {
        lock(&self.seen).values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn usage(used_mb: u64, peak_mb: u64, cap_mb: u64) -> MemoryUse {
        MemoryUse {
            used: used_mb * MB,
            peak: peak_mb * MB,
            limit: cap_mb * MB,
        }
    }

    #[test]
    fn watcher_labels_name_the_process() {
        assert_eq!(process_of("the store worker"), Some(ProcessRef::StoreWorker));
        assert_eq!(process_of("the AI engine"), Some(ProcessRef::Ai));
        assert_eq!(
            process_of("plugin \"net.x\""),
            Some(ProcessRef::Plugin { plugin_id: "net.x".into() })
        );
        for other in ["plugin \"net.x\" (elevated)", "plugin \"\"", "the fake child", "plugin \"a\"b\""] {
            assert_eq!(process_of(other), None, "{other}");
        }
    }

    #[test]
    fn the_suggested_cap_is_the_peak_and_half_again_in_256_mb_steps() {
        assert_eq!(suggested_cap_mb(530 * MB, 4096), 1024);
        assert_eq!(suggested_cap_mb(300 * MB, 4096), 512);
        assert_eq!(suggested_cap_mb(1, 4096), 256);
        assert_eq!(suggested_cap_mb(10_000 * MB, 4096), 4096, "within the setting's maximum");
        assert_eq!(suggested_cap_mb(u64::MAX, 65_536), 65_536);
    }

    #[test]
    fn the_80_percent_warning_is_a_quiet_message_naming_the_use() {
        let p = ProcessRef::Plugin { plugin_id: "net.x".into() };
        let n = notice(&p, "Plugin 'X'", &MemoryEvent::Warning(usage(420, 420, 512))).unwrap();
        assert_eq!(n.title, "Plugin 'X' is using 420 of 512 MB");
        assert!(n.quiet && n.primary.is_none());
        assert_eq!(
            (n.severity, n.source, n.scope),
            (NoticeSeverity::Info, NoticeSource::Performance, NoticeScope::App)
        );
        assert_eq!(n.cause, "memory:warning:plugin:net.x");
        assert_eq!(n.link.plugin_id.as_deref(), Some("net.x"));
        assert!(notice(&p, "x", &MemoryEvent::Ended(usage(1, 1, 2))).is_none());
    }

    #[test]
    fn a_stop_is_service_healths_with_raise_the_cap_advice_and_no_message_of_its_own() {
        let u = usage(2100, 2100, 2048);
        assert!(notice(&ProcessRef::StoreWorker, "The database worker", &MemoryEvent::Stopped(u)).is_none());
        let s = stop(&ProcessRef::StoreWorker, "The database worker", &u);
        assert_eq!((s.service, s.plugin.as_deref()), ("store", None));
        assert_eq!(
            s.cause.message,
            "The database worker was stopped: it used more than its 2048 MB memory cap"
        );
        let hint = s.cause.hint.as_deref().unwrap();
        assert!(
            hint.contains("2100 MB") && hint.contains("3328 MB") && hint.contains("opens again"),
            "{hint}"
        );
        let cap = s.cause.raise_cap.unwrap();
        assert_eq!((cap.target.as_str(), cap.suggested_mb), ("storeWorker", 3328));
        let p = stop(
            &ProcessRef::Plugin { plugin_id: "net.x".into() },
            "Plugin 'X'",
            &usage(530, 530, 512),
        );
        assert_eq!((p.service, p.plugin.as_deref()), ("plugins", Some("net.x")));
        assert_eq!(p.cause.raise_cap.unwrap().target, "plugin:net.x");
        assert_eq!(stop(&ProcessRef::Ai, "The AI worker", &usage(1, 1, 1)).service, "ai");
    }

    #[test]
    fn the_hub_keeps_peaks_names_plugins_and_posts_once_attached() {
        let hub = MemoryHub::new();
        let listener = hub.listener();
        // Before the app is up: the peak is kept, nothing is posted.
        listener.tell("the AI engine", MemoryEvent::Ended(usage(10, 300, 1024)));
        let posted: Arc<Mutex<Vec<NewNotice>>> = Arc::default();
        let p = posted.clone();
        hub.attach(Box::new(move |n| p.lock().unwrap().push(n)));
        hub.set_names(HashMap::from([("net.x".to_string(), "Event logs".to_string())]));
        listener.tell("plugin \"net.x\"", MemoryEvent::Warning(usage(420, 420, 512)));
        listener.tell("plugin \"net.x\"", MemoryEvent::Stopped(usage(530, 530, 512)));
        listener.tell("plugin \"net.x\" (elevated)", MemoryEvent::Stopped(usage(1, 1, 1)));
        let titles: Vec<String> = posted.lock().unwrap().iter().map(|n| n.title.clone()).collect();
        assert_eq!(
            titles,
            ["Plugin 'Event logs' is using 420 of 512 MB"],
            "a stop is service health's, not a second message"
        );
        let seen = hub.seen();
        assert_eq!(seen.len(), 2);
        let ai = seen.iter().find(|s| s.process == ProcessRef::Ai).unwrap();
        assert_eq!((ai.peak_mb, ai.cap_mb, ai.state), (300, 1024, MemoryState::Ended));
        let plugin = seen.iter().find(|s| matches!(s.process, ProcessRef::Plugin { .. })).unwrap();
        assert_eq!((plugin.used_mb, plugin.peak_mb, plugin.state), (530, 530, MemoryState::Stopped));
        // A listener outliving its hub does nothing.
        drop(hub);
        listener.tell("the AI engine", MemoryEvent::Ended(usage(1, 1, 1)));
    }
}
