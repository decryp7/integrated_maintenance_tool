//! Identities and small typed values of an event.

use std::fmt;

/// A stable event identity: assigned once by the store's writer, never reused or renumbered.
/// It is not the event's position in time order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct EventId(pub u64);

impl fmt::Display for EventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// A stable source (file or archive member) identity within one store.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(transparent))]
pub struct SourceId(pub u32);

impl fmt::Display for SourceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Where an event's timestamp came from (ADR 0014 §5). Stored as `u8` in the `ts_state` column.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u8)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "camelCase"))]
pub enum TsState {
    /// Read from the event itself.
    Parsed = 0,
    /// Taken from the event above (a format whose lines carry no time of their own).
    Inherited = 1,
    /// No valid timestamp: the event stays in the grid but is left out of the timeline, time filters
    /// and charts.
    Untimed = 2,
}

impl TsState {
    /// The state stored as `code`, if it is one.
    pub fn from_code(code: u8) -> Option<TsState> {
        match code {
            0 => Some(TsState::Parsed),
            1 => Some(TsState::Inherited),
            2 => Some(TsState::Untimed),
            _ => None,
        }
    }

    pub fn code(self) -> u8 {
        self as u8
    }
}

/// An event's time with its state, so a timestamp and its state cannot disagree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EventTime {
    Parsed(i64),
    Inherited(i64),
    Untimed,
}

impl EventTime {
    /// A parsed timestamp, or untimed when there is none.
    pub fn parsed(ts: Option<i64>) -> EventTime {
        ts.map_or(EventTime::Untimed, EventTime::Parsed)
    }

    /// UTC milliseconds, unless untimed.
    pub fn ts(self) -> Option<i64> {
        match self {
            EventTime::Parsed(ts) | EventTime::Inherited(ts) => Some(ts),
            EventTime::Untimed => None,
        }
    }

    pub fn state(self) -> TsState {
        match self {
            EventTime::Parsed(_) => TsState::Parsed,
            EventTime::Inherited(_) => TsState::Inherited,
            EventTime::Untimed => TsState::Untimed,
        }
    }
}

/// A severity code: the index of the level from Verbose (0) to Fatal (5).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LevelCode(u8);

impl LevelCode {
    /// Highest valid code (Fatal).
    pub const MAX: u8 = 5;
    /// The most severe level.
    pub const FATAL: LevelCode = LevelCode(Self::MAX);

    pub fn new(code: u8) -> Option<LevelCode> {
        (code <= Self::MAX).then_some(LevelCode(code))
    }

    pub fn get(self) -> u8 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ts_state_codes_round_trip() {
        for s in [TsState::Parsed, TsState::Inherited, TsState::Untimed] {
            assert_eq!(TsState::from_code(s.code()), Some(s));
        }
        assert_eq!(TsState::from_code(3), None);
    }

    #[test]
    fn event_time_keeps_ts_and_state_together() {
        assert_eq!(EventTime::parsed(Some(5)), EventTime::Parsed(5));
        assert_eq!(EventTime::parsed(None), EventTime::Untimed);
        assert_eq!(EventTime::Inherited(7).ts(), Some(7));
        assert_eq!(EventTime::Inherited(7).state(), TsState::Inherited);
        assert_eq!(EventTime::Untimed.ts(), None);
        assert_eq!(EventTime::Untimed.state(), TsState::Untimed);
        assert_eq!(EventTime::Parsed(1).state(), TsState::Parsed);
    }

    #[test]
    fn level_codes_are_bounded() {
        assert_eq!(LevelCode::new(5).map(LevelCode::get), Some(5));
        assert_eq!(LevelCode::new(6), None);
        assert_eq!(EventId(3).to_string(), "3");
        assert_eq!(SourceId(4).to_string(), "4");
    }
}
