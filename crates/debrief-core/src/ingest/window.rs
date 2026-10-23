//! Time-window import (ADR 0047): only the events inside a time range are read into the store, so one hour of a 20 GB
//! folder costs one hour of memory and disk. Applied to every event before the profile's pipeline and the store's
//! writer; files that cannot hold an event of the window are not opened at all.

use serde::{Deserialize, Serialize};

/// How much earlier than `since` a file may have been last modified and still hold events from the window: log
/// timestamps are often local time, and a clock can be a day off.
pub const MTIME_SLACK_MS: i64 = 26 * 3_600_000;

/// The range to import (epoch ms, both ends inclusive; `None` is open).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ImportWindow {
    pub from: Option<i64>,
    pub to: Option<i64>,
}

impl ImportWindow {
    /// A window with at least one end; `to` before `from` is refused with a message saying what to fix.
    pub fn new(from: Option<i64>, to: Option<i64>) -> Result<Option<ImportWindow>, String> {
        match (from, to) {
            (None, None) => Ok(None),
            (Some(f), Some(t)) if t < f => Err("the end of the import window is before its start".into()),
            _ => Ok(Some(ImportWindow { from, to })),
        }
    }

    /// Whether an event at `ts` is imported. An event without a time cannot be placed in a window and is left out.
    pub fn contains(&self, ts: Option<i64>) -> bool {
        match ts {
            Some(t) => self.from.is_none_or(|f| t >= f) && self.to.is_none_or(|u| t <= u),
            None => false,
        }
    }

    /// Whether a file last modified at `mtime_ms` cannot hold an event of the window (it stopped growing before
    /// the window began, allowing for [`MTIME_SLACK_MS`]).
    pub fn rules_out_file(&self, mtime_ms: i64) -> bool {
        self.from.is_some_and(|f| mtime_ms < f.saturating_sub(MTIME_SLACK_MS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn contains_is_inclusive_and_open_ended() {
        let w = ImportWindow::new(Some(10), Some(20)).unwrap().unwrap();
        assert!(w.contains(Some(10)) && w.contains(Some(20)) && !w.contains(Some(9)) && !w.contains(Some(21)));
        assert!(!w.contains(None), "an untimed event is outside any window");
        let since = ImportWindow::new(Some(5), None).unwrap().unwrap();
        assert!(since.contains(Some(i64::MAX)) && !since.contains(Some(4)));
        let until = ImportWindow::new(None, Some(5)).unwrap().unwrap();
        assert!(until.contains(Some(i64::MIN)) && !until.contains(Some(6)));
    }

    #[test]
    fn new_parses_not_validates() {
        assert_eq!(ImportWindow::new(None, None), Ok(None));
        assert!(ImportWindow::new(Some(2), Some(1)).unwrap_err().contains("before"));
    }

    #[test]
    fn old_files_are_ruled_out_only_by_since() {
        let w = ImportWindow::new(Some(10 * 86_400_000), None).unwrap().unwrap();
        assert!(w.rules_out_file(8 * 86_400_000));
        assert!(!w.rules_out_file(10 * 86_400_000 - MTIME_SLACK_MS));
        assert!(!w.rules_out_file(i64::MAX));
        assert!(!ImportWindow::new(None, Some(1)).unwrap().unwrap().rules_out_file(i64::MIN));
        assert!(!w.rules_out_file(i64::MIN) || w.from.is_some());
    }
}
