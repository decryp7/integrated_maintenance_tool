//! Engine-side mutes of live alerts: "mute this kind (or rule) until T".
//! A muted alert is not raised at all (and does not start its cooldown), so
//! the first finding after the mute ends or is lifted is raised as usual.
//! Mutes are per workspace (per window), survive clear and reload, and are
//! timed on the wall clock, which is injectable for tests.

use super::alerts::kind_name;
use super::types::AlertTarget;
use super::Workspace;
use anyhow::{anyhow, Result};
use std::sync::Arc;

/// Longest mute accepted, from now (one day).
pub const MAX_MUTE_MS: i64 = 86_400_000;
/// Mutes kept at once (each kind plus a generous number of rules).
pub const MAX_MUTES: usize = 256;
/// Longest rule name accepted in a mute.
const MAX_RULE_NAME: usize = 256;

/// Wall-clock time in milliseconds since the Unix epoch.
pub type Clock = Arc<dyn Fn() -> i64 + Send + Sync>;

/// Active mutes and the clock they are timed with.
#[derive(Clone)]
pub struct AlertMutes {
    entries: Vec<(AlertTarget, i64)>,
    clock: Clock,
}

impl Default for AlertMutes {
    fn default() -> Self {
        AlertMutes::with_clock(Arc::new(super::wall_ms))
    }
}

impl AlertMutes {
    /// Mutes timed with `clock` (tests pass a fake one).
    pub fn with_clock(clock: Clock) -> AlertMutes {
        AlertMutes {
            entries: Vec::new(),
            clock,
        }
    }

    fn now(&self) -> i64 {
        (self.clock)()
    }

    /// Mutes `target` until `until_ms` (wall clock), replacing an earlier mute
    /// of the same target. Fails for an empty or overlong rule name, an end in
    /// the past or more than [`MAX_MUTE_MS`] ahead, or too many mutes.
    pub fn mute(&mut self, target: AlertTarget, until_ms: i64) -> Result<()> {
        if let AlertTarget::Rule(name) = &target {
            if name.trim().is_empty() {
                return Err(anyhow!("name the alert rule to mute"));
            }
            if name.len() > MAX_RULE_NAME {
                return Err(anyhow!("the alert rule name is longer than {MAX_RULE_NAME} bytes"));
            }
        }
        let now = self.now();
        if until_ms <= now {
            return Err(anyhow!("the mute must end in the future"));
        }
        if until_ms - now > MAX_MUTE_MS {
            return Err(anyhow!("alerts can be muted for at most 24 hours"));
        }
        self.entries.retain(|(t, until)| *until > now && *t != target);
        if self.entries.len() >= MAX_MUTES {
            return Err(anyhow!("too many alert mutes (at most {MAX_MUTES}); unmute some first"));
        }
        self.entries.push((target, until_ms));
        Ok(())
    }

    /// Lifts the mute of `target`; true when there was one.
    pub fn unmute(&mut self, target: &AlertTarget) -> bool {
        let before = self.entries.len();
        self.entries.retain(|(t, _)| t != target);
        self.entries.len() != before
    }

    /// Whether alerts of `kind` (a wire name such as `errorSpike`, or `rule`
    /// with the rule's name in `who`) are muted now.
    pub fn covers(&self, kind: &str, who: Option<&str>) -> bool {
        if self.entries.is_empty() {
            return false;
        }
        let now = self.now();
        self.entries.iter().any(|(t, until)| {
            *until > now
                && match t {
                    AlertTarget::Kind(k) => kind != "rule" && kind_name(*k) == kind,
                    AlertTarget::Rule(name) => kind == "rule" && who == Some(name.as_str()),
                }
        })
    }
}

impl Workspace {
    /// Mutes live alerts of one kind or rule until `until_ms` (wall clock).
    pub fn mute_alerts(&mut self, target: AlertTarget, until_ms: i64) -> Result<()> {
        self.tail.mutes.mute(target, until_ms)
    }

    /// Lifts a mute; true when there was one.
    pub fn unmute_alerts(&mut self, target: &AlertTarget) -> bool {
        self.tail.mutes.unmute(target)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::analysis::AnomalyKind;
    use std::sync::atomic::{AtomicI64, Ordering};

    fn fake_clock() -> (Arc<AtomicI64>, AlertMutes) {
        let t = Arc::new(AtomicI64::new(1_000_000));
        let c = t.clone();
        (t, AlertMutes::with_clock(Arc::new(move || c.load(Ordering::SeqCst))))
    }

    #[test]
    fn mute_expire_and_unmute() {
        let (t, mut m) = fake_clock();
        assert!(!m.covers("errorSpike", None));
        m.mute(AlertTarget::Kind(AnomalyKind::ErrorSpike), 1_060_000).unwrap();
        assert!(m.covers("errorSpike", Some("host")));
        assert!(!m.covers("crash", None));
        // A rule is not muted by a kind mute and vice versa.
        assert!(!m.covers("rule", Some("errorSpike")));
        m.mute(AlertTarget::Rule("payments".into()), 1_030_000).unwrap();
        assert!(m.covers("rule", Some("payments")));
        assert!(!m.covers("rule", Some("other")));
        assert!(!m.covers("rule", None));
        // Expiry.
        t.store(1_030_000, Ordering::SeqCst);
        assert!(!m.covers("rule", Some("payments")));
        assert!(m.covers("errorSpike", None));
        // Re-muting replaces (here: shortens) the earlier mute.
        m.mute(AlertTarget::Kind(AnomalyKind::ErrorSpike), 1_040_000).unwrap();
        t.store(1_045_000, Ordering::SeqCst);
        assert!(!m.covers("errorSpike", None));
        // Unmute lifts it early.
        m.mute(AlertTarget::Kind(AnomalyKind::Crash), 1_100_000).unwrap();
        assert!(m.covers("crash", None));
        assert!(m.unmute(&AlertTarget::Kind(AnomalyKind::Crash)));
        assert!(!m.unmute(&AlertTarget::Kind(AnomalyKind::Crash)));
        assert!(!m.covers("crash", None));
    }

    #[test]
    fn mutes_are_validated_and_bounded() {
        let (_, mut m) = fake_clock();
        let kind = || AlertTarget::Kind(AnomalyKind::Silence);
        assert!(m.mute(kind(), 1_000_000).is_err());
        assert!(m.mute(kind(), 999_999).is_err());
        assert!(m.mute(kind(), 1_000_000 + MAX_MUTE_MS + 1).is_err());
        m.mute(kind(), 1_000_000 + MAX_MUTE_MS).unwrap();
        assert!(m.mute(AlertTarget::Rule("  ".into()), 1_001_000).is_err());
        assert!(m.mute(AlertTarget::Rule("x".repeat(MAX_RULE_NAME + 1)), 1_001_000).is_err());
        for i in 1..MAX_MUTES {
            m.mute(AlertTarget::Rule(format!("r{i}")), 1_001_000).unwrap();
        }
        let err = m.mute(AlertTarget::Rule("one too many".into()), 1_001_000).unwrap_err();
        assert!(err.to_string().contains("too many"));
        // Re-muting an existing target still works at the cap.
        m.mute(AlertTarget::Rule("r1".into()), 1_002_000).unwrap();
    }

    #[test]
    fn wire_format_names_exactly_one_target() {
        let k: AlertTarget = serde_json::from_str(r#"{"kind":"errorSpike"}"#).unwrap();
        assert_eq!(k, AlertTarget::Kind(AnomalyKind::ErrorSpike));
        let r: AlertTarget = serde_json::from_str(r#"{"rule":"payments"}"#).unwrap();
        assert_eq!(r, AlertTarget::Rule("payments".into()));
        for bad in [
            r#"{}"#,
            r#"{"kind":"errorSpike","rule":"x"}"#,
            r#"{"kind":"rule"}"#,
            r#"{"other":1}"#,
        ] {
            assert!(serde_json::from_str::<AlertTarget>(bad).is_err(), "{bad}");
        }
        assert_eq!(serde_json::to_string(&r).unwrap(), r#"{"rule":"payments"}"#);
    }

    #[test]
    fn default_clock_is_the_wall_clock() {
        let mut m = AlertMutes::default();
        let now = super::super::wall_ms();
        m.mute(AlertTarget::Kind(AnomalyKind::Crash), now + 60_000).unwrap();
        assert!(m.covers("crash", None));
    }
}
