//! Test doubles shared by the unit tests.

use crate::clock::Clock;
use chrono::{DateTime, TimeZone, Utc};
use debrief_core::export::{clef_value, EventSink};
use debrief_core::model::Record;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Collects CLEF lines in memory (clones share them); `failing` refuses every write.
#[derive(Default, Clone)]
pub struct MemorySink {
    lines: Arc<Mutex<Vec<Value>>>,
    fail: bool,
}

impl MemorySink {
    pub fn failing() -> MemorySink {
        MemorySink {
            lines: Arc::default(),
            fail: true,
        }
    }

    pub fn lines(&self) -> Vec<Value> {
        self.lines.lock().unwrap().clone()
    }

    pub fn messages(&self) -> Vec<String> {
        self.lines()
            .iter()
            .map(|l| l["@m"].as_str().unwrap_or_default().to_string())
            .collect()
    }
}

impl EventSink for MemorySink {
    fn write(&self, records: &[Record]) -> anyhow::Result<()> {
        if self.fail {
            anyhow::bail!("disk full");
        }
        self.lines.lock().unwrap().extend(records.iter().map(clef_value));
        Ok(())
    }
}

/// A clock that only moves when told to (or when slept on).
pub struct FakeClock(pub Mutex<DateTime<Utc>>);

impl FakeClock {
    pub fn at(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> Arc<FakeClock> {
        Arc::new(FakeClock(Mutex::new(Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap())))
    }

    pub fn advance(&self, d: Duration) {
        let mut t = self.0.lock().unwrap();
        *t += chrono::Duration::from_std(d).unwrap();
    }
}

impl Clock for FakeClock {
    fn now(&self) -> DateTime<Utc> {
        *self.0.lock().unwrap()
    }

    fn sleep(&self, d: Duration, _stop: &AtomicBool) {
        self.advance(d);
    }
}

/// Records the actions it ran and writes one file per action; panics on request.
pub struct FakeExecutor {
    pub panic_on: Option<&'static str>,
    pub ran: Mutex<Vec<String>>,
}

impl FakeExecutor {
    pub fn new(panic_on: Option<&'static str>) -> FakeExecutor {
        FakeExecutor {
            panic_on,
            ran: Mutex::default(),
        }
    }
}

impl crate::action::ActionExecutor for FakeExecutor {
    fn execute(&self, action: &crate::config::Action, dir: &std::path::Path, _stop: &AtomicBool) -> crate::run::ActionRecord {
        self.ran.lock().unwrap().push(action.kind().into());
        if self.panic_on == Some(action.kind()) {
            panic!("executor bug");
        }
        std::fs::write(dir.join(format!("{}.out", action.kind())), "data").unwrap();
        let mut r = crate::run::ActionRecord::new(action.kind(), crate::run::ActionStatus::Ok, None);
        r.outputs.push(format!("{}.out", action.kind()));
        r
    }
}

/// Every process is alive (started at time 1).
pub struct AliveProbe;

impl crate::lock::ProcessProbe for AliveProbe {
    fn start_time(&self, _pid: u32) -> Option<u64> {
        Some(1)
    }
}

/// A fixed amount of free space (`None` = unknown).
pub struct FixedDisk(pub Option<u64>);

impl crate::disk::DiskProbe for FixedDisk {
    fn free_bytes(&self, _: &std::path::Path) -> Option<u64> {
        self.0
    }
}
