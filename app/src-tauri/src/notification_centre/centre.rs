//! The notification centre's messages and the pure list that keeps them: dedupe (the same key merges into
//! one message with a count), caps (read messages are trimmed first), read state and dismissal. No
//! threads, no clock (`now` is passed in) and no I/O, so every rule is unit-tested.
//!
//! Wire contract: `app/src/app/notifications/notifications.types.ts`.

use serde::{Deserialize, Serialize};

/// Project-scoped messages kept (and saved with the project).
pub const PROJECT_CAP: usize = 200;
/// App-scoped messages kept (in the app data folder).
pub const APP_CAP: usize = 100;
const MAX_TITLE: usize = 160;
const MAX_BODY: usize = 1000;
const MAX_KEY: usize = 200;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NoticeId(pub u64);

/// What raised the message (the view groups by it).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeSource {
    Job,
    LiveAlert,
    Import,
    Plugin,
    Ai,
    Update,
    Session,
    /// Memory caps of Debrief's processes (Settings ▸ Performance).
    Performance,
    /// Things about the files and folders the user opened (a new folder worth setting up).
    Workspace,
    /// A Debrief service stopped (service health: the log store, the AI worker, plugins, the browser, downloads).
    Health,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeSeverity {
    Info,
    Success,
    Warning,
    Error,
    /// A failure that stops the work and stays until it is resolved or dismissed.
    Critical,
}

/// Where a message lives: with the window's project, or with the app (update, crash recovery, AI).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum NoticeScope {
    App,
    Project,
}

/// What a message's button does. The UI maps each to the existing feature (it never runs anything the
/// engine did not offer here).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "type")]
pub enum NoticeAction {
    /// Opens a finished job's result.
    ViewJob {
        #[serde(rename = "jobId")]
        job_id: u64,
    },
    RetryJob {
        #[serde(rename = "jobId")]
        job_id: u64,
    },
    /// Reviews AI proposals a job produced.
    ReviewProposals {
        #[serde(rename = "jobId")]
        job_id: u64,
    },
    ResetPlugin {
        #[serde(rename = "pluginId")]
        plugin_id: String,
    },
    /// Opens the format wizard for a file that was not imported.
    SetUpFormat {
        path: String,
    },
    /// Opens the format wizard for a file whose timestamps were not read, with the format the engine
    /// verified (none: the wizard starts empty).
    FixTimestamps {
        path: String,
        format: Option<String>,
    },
    ResetAi,
    /// Opens the assistant (its chat view).
    OpenAssistant,
    /// Sets the assistant up again with this model (after a failed background set-up).
    SetUpAssistant {
        #[serde(rename = "modelId")]
        model_id: String,
    },
    InstallUpdate,
    /// Shows a live alert's evidence (its time range in the grid).
    ShowAlert {
        #[serde(rename = "alertId")]
        alert_id: String,
    },
    /// Opens the Jobs pane.
    ShowJobs,
    /// Shows a job's file in Finder or Explorer.
    ShowInFolder {
        #[serde(rename = "jobId")]
        job_id: u64,
    },
    /// Installs an AI runtime or model update (`key` from the update check).
    UpdateAi {
        key: String,
    },
    /// Puts an AI update off until the next check.
    LaterAiUpdate {
        key: String,
    },
    /// Never offers this version of an AI update again.
    SkipAiUpdate {
        key: String,
    },
    /// Repairs a service from the shell (`health_repair`).
    RepairService {
        service: String,
    },
    /// Opens Settings at `section` (a service fixed by the user's choice there, e.g. Settings ▸ Web browser).
    OpenSettings {
        section: String,
    },
}

/// One of Debrief's capped processes, as a message's action names it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "kind")]
pub enum ProcessRef {
    StoreWorker,
    Ai,
    Plugin {
        #[serde(rename = "pluginId")]
        plugin_id: String,
    },
}

/// What a message refers to.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NoticeLink {
    pub window: Option<String>,
    pub job_id: Option<u64>,
    pub record_id: Option<u32>,
    pub plugin_id: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notice {
    pub id: NoticeId,
    pub source: NoticeSource,
    pub severity: NoticeSeverity,
    pub title: String,
    pub body: String,
    /// Epoch ms of the latest occurrence.
    pub at: i64,
    pub first_at: i64,
    /// How many times it happened (dedupe merges).
    pub count: u32,
    pub read: bool,
    pub primary: Option<NoticeAction>,
    pub secondary: Option<NoticeAction>,
    /// A third button (e.g. "Skip this version" next to "Update" and "Later").
    #[serde(default)]
    pub tertiary: Option<NoticeAction>,
    #[serde(default)]
    pub link: NoticeLink,
    /// The cause key (`store.down`, `job:12`, `update:1.2`): a second post with the same cause updates this message in
    /// place; a cause that clears resolves (removes) it.
    pub cause: String,
    pub scope: NoticeScope,
}

/// What a producer posts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NewNotice {
    pub source: NoticeSource,
    pub severity: NoticeSeverity,
    pub title: String,
    pub body: String,
    pub primary: Option<NoticeAction>,
    pub secondary: Option<NoticeAction>,
    pub tertiary: Option<NoticeAction>,
    pub link: NoticeLink,
    /// Mandatory: what this message is about. The same cause again updates the existing message.
    pub cause: String,
    pub scope: NoticeScope,
    /// Posting the same key again is ignored instead of counted (e.g. an update announced at every check).
    pub once: bool,
    /// Listed without a toast (a warning that needs no action yet).
    pub quiet: bool,
}

impl NewNotice {
    pub fn new(
        cause: impl Into<String>,
        source: NoticeSource,
        severity: NoticeSeverity,
        title: impl Into<String>,
        scope: NoticeScope,
    ) -> NewNotice {
        NewNotice {
            source,
            severity,
            title: title.into(),
            body: String::new(),
            primary: None,
            secondary: None,
            tertiary: None,
            link: NoticeLink::default(),
            cause: cause.into(),
            scope,
            once: false,
            quiet: false,
        }
    }

    /// Listed in the centre without a toast.
    pub fn quiet(mut self) -> NewNotice {
        self.quiet = true;
        self
    }

    /// Announced once per dedupe key: posting it again changes nothing.
    pub fn once(mut self) -> NewNotice {
        self.once = true;
        self
    }

    pub fn body(mut self, body: impl Into<String>) -> NewNotice {
        self.body = body.into();
        self
    }

    pub fn primary(mut self, a: NoticeAction) -> NewNotice {
        self.primary = Some(a);
        self
    }

    pub fn secondary(mut self, a: NoticeAction) -> NewNotice {
        self.secondary = Some(a);
        self
    }

    pub fn tertiary(mut self, a: NoticeAction) -> NewNotice {
        self.tertiary = Some(a);
        self
    }

    pub fn link(mut self, link: NoticeLink) -> NewNotice {
        self.link = link;
        self
    }
}

fn clean(s: &str, max: usize) -> String {
    s.chars()
        .map(|c| if c == '\n' || c == '\t' { ' ' } else { c })
        .filter(|c| !c.is_control())
        .take(max)
        .collect::<String>()
        .trim()
        .to_string()
}

/// What a post did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Posted {
    pub notice: Notice,
    /// It merged into an existing message (same dedupe key).
    pub merged: bool,
    /// A `once` message that was already there: nothing changed.
    pub unchanged: bool,
}

/// One list of messages (the app's, or one window's project), oldest first.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Centre {
    cap: usize,
    notices: Vec<Notice>,
}

impl Centre {
    pub fn new(cap: usize) -> Centre {
        Centre { cap, notices: Vec::new() }
    }

    /// Takes saved messages (bounded by the cap, read ones dropped first).
    pub fn with(cap: usize, notices: Vec<Notice>) -> Centre {
        let mut c = Centre { cap, notices };
        c.notices.sort_by_key(|n| n.at);
        c.trim();
        c
    }

    /// Adds a message, or merges it into the one with the same dedupe key: the count goes up, the time,
    /// text, severity and actions are the latest, and it is unread again. `id` is used for a new message.
    pub fn post(&mut self, new: NewNotice, id: NoticeId, now: i64) -> Posted {
        let key = clean(&new.cause, MAX_KEY);
        let key = if key.is_empty() { format!("anonymous:{}", id.0) } else { key };
        let title = clean(&new.title, MAX_TITLE);
        let body = clean(&new.body, MAX_BODY);
        if let Some(i) = self.notices.iter().position(|n| n.cause == key) {
            if new.once {
                return Posted {
                    notice: self.notices[i].clone(),
                    merged: true,
                    unchanged: true,
                };
            }
            let mut n = self.notices.remove(i);
            n.count = n.count.saturating_add(1);
            n.at = now.max(n.at);
            n.read = false;
            n.title = title;
            n.body = body;
            n.severity = new.severity;
            n.primary = new.primary;
            n.secondary = new.secondary;
            n.tertiary = new.tertiary;
            n.link = new.link;
            self.notices.push(n.clone());
            return Posted {
                notice: n,
                merged: true,
                unchanged: false,
            };
        }
        let n = Notice {
            id,
            source: new.source,
            severity: new.severity,
            title,
            body,
            at: now,
            first_at: now,
            count: 1,
            read: false,
            primary: new.primary,
            secondary: new.secondary,
            tertiary: new.tertiary,
            link: new.link,
            cause: key,
            scope: new.scope,
        };
        self.notices.push(n.clone());
        self.trim();
        Posted {
            notice: n,
            merged: false,
            unchanged: false,
        }
    }

    /// Updates the message of `cause` in place (`post` with a cause that is there does the same); false when no message
    /// has this cause, and nothing changes.
    #[allow(dead_code)] // the shared notification core's API (Paddock ADR 0010); producers post, which updates in place too
    pub fn update_cause(&mut self, cause: &str, new: NewNotice, now: i64) -> bool {
        let Some(id) = self.notices.iter().find(|n| n.cause == cause).map(|n| n.id) else {
            return false;
        };
        let mut new = new;
        new.cause = cause.to_string();
        self.post(new, id, now);
        true
    }

    /// The cause cleared: removes its message. Returns it, or `None` when it was not here.
    pub fn resolve_cause(&mut self, cause: &str) -> Option<Notice> {
        let i = self.notices.iter().position(|n| n.cause == cause)?;
        Some(self.notices.remove(i))
    }

    /// Over the cap: the oldest read messages go first, then the oldest unread ones.
    fn trim(&mut self) {
        while self.notices.len() > self.cap {
            let i = self.notices.iter().position(|n| n.read).unwrap_or(0);
            self.notices.remove(i);
        }
    }

    /// Marks one message read. False when it is not here.
    pub fn mark_read(&mut self, id: NoticeId) -> bool {
        match self.notices.iter_mut().find(|n| n.id == id) {
            Some(n) => {
                n.read = true;
                true
            }
            None => false,
        }
    }

    /// Marks every message read; returns how many were unread.
    pub fn mark_all_read(&mut self) -> usize {
        let mut n = 0;
        for m in self.notices.iter_mut().filter(|m| !m.read) {
            m.read = true;
            n += 1;
        }
        n
    }

    /// Removes a message. False when it is not here.
    pub fn dismiss(&mut self, id: NoticeId) -> bool {
        let before = self.notices.len();
        self.notices.retain(|n| n.id != id);
        self.notices.len() != before
    }

    #[cfg(test)]
    pub fn unread(&self) -> usize {
        self.notices.iter().filter(|n| !n.read).count()
    }

    pub fn notices(&self) -> &[Notice] {
        &self.notices
    }

    #[cfg(test)]
    pub fn get(&self, id: NoticeId) -> Option<&Notice> {
        self.notices.iter().find(|n| n.id == id)
    }

    pub fn max_id(&self) -> u64 {
        self.notices.iter().map(|n| n.id.0).max().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spike(host: &str) -> NewNotice {
        NewNotice::new(
            format!("live:errorSpike:{host}"),
            NoticeSource::LiveAlert,
            NoticeSeverity::Error,
            format!("ERROR spike on {host}"),
            NoticeScope::Project,
        )
        .body("12 errors in a minute")
        .primary(NoticeAction::ShowAlert { alert_id: "a1".into() })
    }

    #[test]
    fn the_same_key_merges_into_one_message_with_a_count() {
        let mut c = Centre::new(10);
        let first = c.post(spike("WEB02"), NoticeId(1), 100);
        assert!(!first.merged);
        c.mark_read(NoticeId(1));
        c.post(spike("WEB01"), NoticeId(2), 150);
        for t in [200, 300, 400] {
            let p = c.post(spike("WEB02"), NoticeId(99), t);
            assert!(p.merged);
        }
        assert_eq!(c.notices().len(), 2);
        let merged = c.get(NoticeId(1)).unwrap();
        assert_eq!((merged.count, merged.first_at, merged.at, merged.read), (4, 100, 400, false));
        assert_eq!(c.notices().last().unwrap().id, NoticeId(1), "moved to the newest place");
        assert!(c.get(NoticeId(99)).is_none());
        // A `once` message is announced once.
        let update = || {
            NewNotice::new(
                "update:2",
                NoticeSource::Update,
                NoticeSeverity::Info,
                "Debrief 2 is available",
                NoticeScope::App,
            )
            .once()
        };
        c.post(update(), NoticeId(7), 500);
        c.mark_read(NoticeId(7));
        let again = c.post(update(), NoticeId(8), 600);
        assert!(again.unchanged && again.notice.read && again.notice.count == 1);
        c.dismiss(NoticeId(7));
        // Different causes never merge.
        let plain = |cause: &str| NewNotice::new(cause, NoticeSource::Session, NoticeSeverity::Info, "Safe mode", NoticeScope::App);
        c.post(plain("a"), NoticeId(3), 1);
        c.post(plain("b"), NoticeId(4), 2);
        assert_eq!(c.notices().len(), 4);
    }

    #[test]
    fn caps_trim_read_messages_first() {
        let mut c = Centre::new(3);
        for i in 1..=3 {
            c.post(
                NewNotice::new(
                    format!("job:{i}"),
                    NoticeSource::Job,
                    NoticeSeverity::Success,
                    format!("job {i}"),
                    NoticeScope::Project,
                ),
                NoticeId(i),
                i as i64,
            );
        }
        c.mark_read(NoticeId(2));
        c.post(
            NewNotice::new("job:4", NoticeSource::Job, NoticeSeverity::Success, "job 4", NoticeScope::Project),
            NoticeId(4),
            4,
        );
        let ids: Vec<u64> = c.notices().iter().map(|n| n.id.0).collect();
        assert_eq!(ids, [1, 3, 4], "the read one went, although it was not the oldest");
        c.post(
            NewNotice::new("job:5", NoticeSource::Job, NoticeSeverity::Success, "job 5", NoticeScope::Project),
            NoticeId(5),
            5,
        );
        let ids: Vec<u64> = c.notices().iter().map(|n| n.id.0).collect();
        assert_eq!(ids, [3, 4, 5], "all unread: the oldest goes");
        let restored = Centre::with(2, c.notices().to_vec());
        assert_eq!(restored.notices().len(), 2);
        assert_eq!(restored.max_id(), 5);
    }

    #[test]
    fn read_state_dismissal_and_cleaning() {
        let mut c = Centre::new(10);
        let p = c.post(
            NewNotice::new(
                "plugin:p",
                NoticeSource::Plugin,
                NoticeSeverity::Error,
                "Plugin\nquarantined\u{7}",
                NoticeScope::Project,
            )
            .body("x".repeat(5000))
            .secondary(NoticeAction::ShowJobs),
            NoticeId(1),
            0,
        );
        assert_eq!(p.notice.title, "Plugin quarantined");
        assert_eq!(p.notice.body.chars().count(), MAX_BODY);
        assert_eq!(p.notice.cause, "plugin:p");
        c.post(
            NewNotice::new(
                "ai:paused",
                NoticeSource::Ai,
                NoticeSeverity::Warning,
                "AI paused",
                NoticeScope::App,
            ),
            NoticeId(2),
            1,
        );
        assert_eq!(c.unread(), 2);
        assert!(c.mark_read(NoticeId(1)));
        assert!(!c.mark_read(NoticeId(9)));
        assert_eq!(c.unread(), 1);
        assert_eq!(c.mark_all_read(), 1);
        assert_eq!(c.mark_all_read(), 0);
        assert!(c.dismiss(NoticeId(1)));
        assert!(!c.dismiss(NoticeId(1)));
        assert_eq!(c.notices().len(), 1);
        let wire = serde_json::to_value(&c.notices()[0]).unwrap();
        assert_eq!(
            (wire["source"].as_str(), wire["severity"].as_str(), wire["scope"].as_str()),
            (Some("ai"), Some("warning"), Some("app"))
        );
        // The workspace source (K4) is spelled like the UI's `NoticeSource`.
        assert_eq!(
            serde_json::to_value(NoticeSource::Workspace).unwrap(),
            serde_json::json!("workspace")
        );
        assert_eq!(
            serde_json::from_value::<NoticeSource>(serde_json::json!("workspace")).unwrap(),
            NoticeSource::Workspace
        );
        assert!(serde_json::from_value::<NoticeSource>(serde_json::json!("nope")).is_err());
        let action = serde_json::to_value(NoticeAction::ViewJob { job_id: 3 }).unwrap();
        assert_eq!(action, serde_json::json!({"type": "viewJob", "jobId": 3}));
    }

    #[test]
    fn a_third_action_is_kept_merged_and_optional_on_the_wire() {
        let ask = |to: &str| {
            NewNotice::new(
                "ai-update:runtime:cpu",
                NoticeSource::Ai,
                NoticeSeverity::Info,
                format!("Update to {to}"),
                NoticeScope::App,
            )
            .primary(NoticeAction::UpdateAi {
                key: format!("runtime:cpu@{to}"),
            })
            .secondary(NoticeAction::LaterAiUpdate {
                key: format!("runtime:cpu@{to}"),
            })
            .tertiary(NoticeAction::SkipAiUpdate {
                key: format!("runtime:cpu@{to}"),
            })
        };
        let mut c = Centre::new(10);
        c.post(ask("b1"), NoticeId(1), 1);
        let merged = c.post(ask("b2"), NoticeId(2), 2).notice;
        assert_eq!(
            merged.tertiary,
            Some(NoticeAction::SkipAiUpdate {
                key: "runtime:cpu@b2".into()
            })
        );
        let wire = serde_json::to_value(&merged).unwrap();
        assert_eq!(wire["primary"], serde_json::json!({"type": "updateAi", "key": "runtime:cpu@b2"}));
        assert_eq!(wire["secondary"]["type"], "laterAiUpdate");
        assert_eq!(wire["tertiary"]["type"], "skipAiUpdate");
        let mut old = wire.clone();
        old.as_object_mut().unwrap().remove("tertiary");
        let back: Notice = serde_json::from_value(old).unwrap();
        assert_eq!(back.tertiary, None, "messages saved before the third action still load");
    }

    #[test]
    fn a_blank_cause_is_replaced_by_a_unique_one_so_it_never_merges_by_accident() {
        let mut c = Centre::new(10);
        let a = c.post(
            NewNotice::new("  ", NoticeSource::Session, NoticeSeverity::Info, "a", NoticeScope::App),
            NoticeId(1),
            0,
        );
        let b = c.post(
            NewNotice::new("", NoticeSource::Session, NoticeSeverity::Info, "b", NoticeScope::App),
            NoticeId(2),
            1,
        );
        assert!(!a.merged && !b.merged);
        assert_ne!(a.notice.cause, b.notice.cause);
    }

    #[test]
    fn update_cause_changes_the_message_in_place_and_reports_a_cause_that_is_not_there() {
        let mut c = Centre::new(10);
        c.post(spike("WEB02"), NoticeId(1), 100);
        let newer = NewNotice::new(
            "ignored",
            NoticeSource::LiveAlert,
            NoticeSeverity::Critical,
            "Worse",
            NoticeScope::Project,
        )
        .body("more");
        assert!(c.update_cause("live:errorSpike:WEB02", newer, 200));
        assert_eq!(c.notices().len(), 1);
        let n = &c.notices()[0];
        assert_eq!(
            (n.id, n.title.as_str(), n.severity, n.count, n.cause.as_str()),
            (NoticeId(1), "Worse", NoticeSeverity::Critical, 2, "live:errorSpike:WEB02")
        );
        assert!(!c.update_cause("nope", spike("x"), 300));
        assert_eq!(c.notices().len(), 1);
    }

    #[test]
    fn a_cause_that_clears_removes_its_message_only() {
        let mut c = Centre::new(10);
        c.post(spike("WEB01"), NoticeId(1), 1);
        c.post(spike("WEB02"), NoticeId(2), 2);
        assert_eq!(c.resolve_cause("live:errorSpike:WEB01").map(|n| n.id), Some(NoticeId(1)));
        assert_eq!(c.resolve_cause("live:errorSpike:WEB01"), None);
        assert_eq!(c.notices().len(), 1);
    }

    #[test]
    fn the_cause_and_the_new_severity_and_source_are_spelled_like_the_ui() {
        let n = Centre::new(1).post(spike("W"), NoticeId(1), 0).notice;
        let wire = serde_json::to_value(&n).unwrap();
        assert_eq!(wire["cause"], "live:errorSpike:W");
        assert!(wire.get("dedupeKey").is_none());
        assert_eq!(
            serde_json::to_value(NoticeSeverity::Critical).unwrap(),
            serde_json::json!("critical")
        );
        assert_eq!(serde_json::to_value(NoticeSource::Health).unwrap(), serde_json::json!("health"));
        assert_eq!(
            serde_json::to_value(NoticeAction::RepairService { service: "store".into() }).unwrap(),
            serde_json::json!({"type": "repairService", "service": "store"})
        );
        assert_eq!(
            serde_json::to_value(NoticeAction::OpenSettings { section: "browser".into() }).unwrap(),
            serde_json::json!({"type": "openSettings", "section": "browser"})
        );
    }
}
