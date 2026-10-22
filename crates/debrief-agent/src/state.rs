//! `state.json`: when each schedule last ran, so timers survive restarts.
//! Versioned; a missing file means "never ran"; a corrupt or newer file is
//! set aside as `state.json.corrupt` and the agent starts from defaults.
//! Written atomically (temporary file, then rename).

use crate::run::RunStatus;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

pub const STATE_VERSION: u32 = 1;
pub const STATE_FILE: &str = "state.json";

#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ScheduleState {
    #[serde(default)]
    pub last_run_ms: Option<i64>,
    #[serde(default)]
    pub last_status: Option<RunStatus>,
    #[serde(default)]
    pub last_run_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentState {
    pub version: u32,
    #[serde(default)]
    pub schedules: BTreeMap<String, ScheduleState>,
}

impl Default for AgentState {
    fn default() -> Self {
        AgentState {
            version: STATE_VERSION,
            schedules: BTreeMap::new(),
        }
    }
}

/// What happened when loading the state.
#[derive(Debug, PartialEq)]
pub enum Loaded {
    Fresh,
    Read,
    /// The file was unusable; the reason, and where it was moved.
    Recovered(String),
}

impl AgentState {
    pub fn load(dir: &Path) -> (AgentState, Loaded) {
        let path = dir.join(STATE_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (AgentState::default(), Loaded::Fresh),
            Err(e) => {
                return (
                    AgentState::default(),
                    Loaded::Recovered(format!("cannot read {}: {e}", path.display())),
                )
            }
        };
        let reason = match serde_json::from_str::<AgentState>(&text) {
            Ok(s) if s.version <= STATE_VERSION => return (s, Loaded::Read),
            Ok(s) => format!("{} has unknown version {}", path.display(), s.version),
            Err(e) => format!("{} is corrupt ({e})", path.display()),
        };
        let aside = dir.join(format!("{STATE_FILE}.corrupt"));
        let _ = std::fs::rename(&path, &aside);
        (
            AgentState::default(),
            Loaded::Recovered(format!("{reason}; moved to {}", aside.display())),
        )
    }

    pub fn save(&self, dir: &Path) -> std::io::Result<()> {
        std::fs::create_dir_all(dir)?;
        let tmp = dir.join(format!("{STATE_FILE}.tmp"));
        std::fs::write(&tmp, serde_json::to_vec_pretty(self).map_err(std::io::Error::other)?)?;
        std::fs::rename(&tmp, dir.join(STATE_FILE))
    }

    pub fn get(&self, schedule: &str) -> ScheduleState {
        self.schedules.get(schedule).cloned().unwrap_or_default()
    }

    pub fn entry(&mut self, schedule: &str) -> &mut ScheduleState {
        self.schedules.entry(schedule.to_string()).or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let (s, how) = AgentState::load(dir.path());
        assert_eq!((s.clone(), how), (AgentState::default(), Loaded::Fresh));
        let mut s = s;
        s.entry("nightly").last_run_ms = Some(5);
        s.entry("nightly").last_status = Some(RunStatus::Ok);
        s.save(&dir.path().join("sub")).unwrap();
        let (back, how) = AgentState::load(&dir.path().join("sub"));
        assert_eq!(how, Loaded::Read);
        assert_eq!(back, s);
        assert_eq!(back.get("nightly").last_run_ms, Some(5));
        assert_eq!(back.get("other"), ScheduleState::default());
    }

    #[test]
    fn corrupt_or_newer_state_falls_back_to_defaults() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(STATE_FILE), "{ garbage").unwrap();
        let (s, how) = AgentState::load(dir.path());
        assert_eq!(s, AgentState::default());
        assert!(matches!(how, Loaded::Recovered(ref m) if m.contains("corrupt")), "{how:?}");
        assert!(dir.path().join("state.json.corrupt").exists());
        assert!(!dir.path().join(STATE_FILE).exists());

        std::fs::write(dir.path().join(STATE_FILE), r#"{"version": 99}"#).unwrap();
        let (_, how) = AgentState::load(dir.path());
        assert!(matches!(how, Loaded::Recovered(ref m) if m.contains("unknown version 99")));

        // Unreadable (a folder in its place).
        std::fs::create_dir(dir.path().join(STATE_FILE)).unwrap();
        let (s, how) = AgentState::load(dir.path());
        assert_eq!(s, AgentState::default());
        assert!(matches!(how, Loaded::Recovered(ref m) if m.contains("cannot read")));
        // And it cannot be saved over.
        assert!(AgentState::default().save(dir.path()).is_err());
    }
}
