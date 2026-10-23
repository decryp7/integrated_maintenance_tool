//! Update policies and scheduling, as pure functions of the time (epoch seconds) and the record.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// What Debrief does when it finds an update of one kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum UpdatePolicy {
    /// Download and install in the background (a job), keeping the previous version until the new one verifies.
    Automatic,
    /// Announce it ("Update", "Later", "Skip this version").
    #[default]
    Ask,
    /// Only the Settings "Check now" button looks for updates; nothing is announced.
    Manual,
}

impl UpdatePolicy {
    pub const ALL: [UpdatePolicy; 3] = [UpdatePolicy::Automatic, UpdatePolicy::Ask, UpdatePolicy::Manual];

    pub fn as_str(self) -> &'static str {
        match self {
            UpdatePolicy::Automatic => "automatic",
            UpdatePolicy::Ask => "ask",
            UpdatePolicy::Manual => "manual",
        }
    }

    pub fn parse(s: &str) -> Option<UpdatePolicy> {
        UpdatePolicy::ALL.into_iter().find(|p| p.as_str() == s.trim().to_ascii_lowercase())
    }
}

impl Serialize for UpdatePolicy {
    fn serialize<S: Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str(self.as_str())
    }
}

/// Unknown words mean the default (a newer Debrief's policy read by an older one).
impl<'de> Deserialize<'de> for UpdatePolicy {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(String::deserialize(d)
            .ok()
            .and_then(|s| UpdatePolicy::parse(&s))
            .unwrap_or_default())
    }
}

/// What is updated.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateKind {
    /// An inference runtime (llama-server build).
    Runtime,
    /// A model file.
    Model,
}

impl UpdateKind {
    pub const ALL: [UpdateKind; 2] = [UpdateKind::Runtime, UpdateKind::Model];

    pub fn as_str(self) -> &'static str {
        match self {
            UpdateKind::Runtime => "runtime",
            UpdateKind::Model => "model",
        }
    }

    pub fn parse(s: &str) -> Option<UpdateKind> {
        UpdateKind::ALL.into_iter().find(|k| k.as_str() == s)
    }
}

/// The first retry after a failed check; each further failure doubles it, up to the check interval.
pub const FIRST_RETRY_SECS: u64 = 5 * 60;

/// Seconds to wait after `failures` failed checks in a row (0 after none). Never more than the interval, so a
/// long outage costs at most one check per interval.
pub fn backoff_secs(failures: u32, interval_secs: u64) -> u64 {
    if failures == 0 {
        return 0;
    }
    let exp = FIRST_RETRY_SECS.saturating_mul(1u64 << (failures - 1).min(30));
    exp.min(interval_secs.max(FIRST_RETRY_SECS))
}

/// Whether the scheduled check of a kind is due at `now`. Manual never checks by itself; after a failure the
/// retry time decides; otherwise the interval since the last check. A last check in the future (the clock was
/// set back) is due at once rather than never.
pub fn check_due(policy: UpdatePolicy, now: i64, last_check: Option<i64>, next_retry: Option<i64>, interval_secs: u64) -> bool {
    if policy == UpdatePolicy::Manual {
        return false;
    }
    if let Some(retry) = next_retry {
        return now >= retry;
    }
    match last_check {
        None => true,
        Some(t) if now < t => true,
        Some(t) => (now - t) as u64 >= interval_secs,
    }
}

/// What is going on when an update is found.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Situation {
    /// The user chose "Skip this version" for it.
    pub skipped: bool,
    /// It was already announced (and not put off with "Later").
    pub announced: bool,
    /// A chat or another AI request is running: an automatic install waits for it.
    pub busy: bool,
}

/// What to do with one update.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Start the install job now.
    Install,
    /// Install as soon as nothing is running (checked again shortly).
    Defer,
    /// Post the "Update / Later / Skip this version" message.
    Announce,
    /// Show it in Settings only.
    List,
}

pub fn decide(policy: UpdatePolicy, s: Situation) -> Decision {
    if s.skipped {
        return Decision::List;
    }
    match policy {
        UpdatePolicy::Automatic if s.busy => Decision::Defer,
        UpdatePolicy::Automatic => Decision::Install,
        UpdatePolicy::Ask if s.announced => Decision::List,
        UpdatePolicy::Ask => Decision::Announce,
        UpdatePolicy::Manual => Decision::List,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: u64 = 86_400;

    #[test]
    fn policies_parse_and_serialize_leniently() {
        for p in UpdatePolicy::ALL {
            assert_eq!(UpdatePolicy::parse(p.as_str()), Some(p));
            let json = serde_json::to_string(&p).unwrap();
            assert_eq!(serde_json::from_str::<UpdatePolicy>(&json).unwrap(), p);
        }
        assert_eq!(UpdatePolicy::parse(" Automatic "), Some(UpdatePolicy::Automatic));
        assert_eq!(UpdatePolicy::parse("sometimes"), None);
        assert_eq!(serde_json::from_str::<UpdatePolicy>("\"future\"").unwrap(), UpdatePolicy::Ask);
        assert_eq!(serde_json::from_str::<UpdatePolicy>("12").unwrap(), UpdatePolicy::Ask);
        assert_eq!(UpdatePolicy::default(), UpdatePolicy::Ask);
        for k in UpdateKind::ALL {
            assert_eq!(UpdateKind::parse(k.as_str()), Some(k));
        }
        assert_eq!(UpdateKind::parse("plugin"), None);
    }

    #[test]
    fn backoff_doubles_up_to_the_interval() {
        assert_eq!(backoff_secs(0, DAY), 0);
        assert_eq!(backoff_secs(1, DAY), 300);
        assert_eq!(backoff_secs(2, DAY), 600);
        assert_eq!(backoff_secs(5, DAY), 4800);
        assert_eq!(backoff_secs(20, DAY), DAY);
        assert_eq!(backoff_secs(u32::MAX, DAY), DAY, "no overflow");
        assert_eq!(backoff_secs(3, 60), FIRST_RETRY_SECS, "never faster than the first retry");
    }

    #[test]
    fn checks_are_due_by_interval_retry_and_policy() {
        let now = 1_000_000;
        assert!(check_due(UpdatePolicy::Ask, now, None, None, DAY), "never checked");
        assert!(
            !check_due(UpdatePolicy::Manual, now, None, None, DAY),
            "manual never checks by itself"
        );
        assert!(!check_due(UpdatePolicy::Automatic, now, Some(now - 3600), None, DAY));
        assert!(check_due(UpdatePolicy::Automatic, now, Some(now - DAY as i64), None, DAY));
        assert!(check_due(UpdatePolicy::Ask, now, Some(now + 3600), None, DAY), "clock set back");
        // After a failure, the retry time decides (earlier than the interval).
        assert!(!check_due(UpdatePolicy::Ask, now, Some(now - 10), Some(now + 290), DAY));
        assert!(check_due(UpdatePolicy::Ask, now, Some(now - 300), Some(now), DAY));
    }

    #[test]
    fn decisions_follow_the_policy() {
        use Decision::*;
        let free = Situation::default();
        let busy = Situation { busy: true, ..free };
        let skipped = Situation { skipped: true, ..free };
        let announced = Situation { announced: true, ..free };
        assert_eq!(decide(UpdatePolicy::Automatic, free), Install);
        assert_eq!(decide(UpdatePolicy::Automatic, busy), Defer, "never during a chat");
        assert_eq!(decide(UpdatePolicy::Automatic, skipped), List);
        assert_eq!(decide(UpdatePolicy::Ask, free), Announce);
        assert_eq!(decide(UpdatePolicy::Ask, busy), Announce, "asking does not interrupt anything");
        assert_eq!(decide(UpdatePolicy::Ask, announced), List, "announced once");
        assert_eq!(decide(UpdatePolicy::Ask, skipped), List);
        assert_eq!(decide(UpdatePolicy::Manual, free), List);
    }
}
