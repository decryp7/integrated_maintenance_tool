//! Live Windows Event Log: follows channels and turns their events into the
//! records the `.evtx` importer produces.
//!
//! * [`query`]: channels, levels, backfill window and the user's XPath filter
//!   (validated) combined into one Event Log XPath.
//! * [`xml`]: rendered event XML (`EvtRender`) to records with the importer's
//!   field names (so markers, the Windows annotators and reboot detection work
//!   unchanged), with the `EvtFormatMessage` text as the message when the
//!   provider has one.
//! * [`bookmark`]: the last written record per channel, persisted so a
//!   restarted follower resumes without gaps or duplicates.
//! * [`follower`]: the engine: one reader per channel (backfill, then live),
//!   a bounded queue, one writer to rolling CLEF files, pause, status.
//! * [`EventLog`] is the port to the Event Log service; the Windows adapter
//!   (`EvtQuery`, `EvtSubscribe`, `EvtNext`, `EvtRender`, `EvtFormatMessage`)
//!   is [`system`] on Windows. Tests use fakes.
//!
//! Remote computers are out of scope: every call uses the local session.

pub mod bookmark;
pub mod follower;
pub mod query;
#[cfg(windows)]
mod win;
pub mod xml;

pub use follower::{Follower, FollowerConfig, FollowerStatus};
pub use query::{Backfill, Channel, EventLevel, LevelSet, XPathFilter};

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

/// Why the Event Log refused or failed. The messages are for the user; the
/// access-denied one names the way forward ("Restart as administrator").
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EventLogError {
    /// Missing rights (the Security channel needs administrator rights).
    AccessDenied {
        channel: String,
    },
    ChannelNotFound {
        channel: String,
    },
    /// The Event Log rejected the XPath.
    InvalidQuery {
        channel: String,
        detail: String,
    },
    /// The result set is no longer valid (the log was cleared or wrapped):
    /// reopen after the last record.
    Stale {
        channel: String,
    },
    /// The Event Log is not available here (not Windows, service stopped).
    Unavailable(String),
    Other {
        channel: String,
        code: u32,
        message: String,
    },
}

impl EventLogError {
    /// Whether retrying later can help (a reader keeps trying these).
    pub fn is_transient(&self) -> bool {
        matches!(self, EventLogError::Stale { .. } | EventLogError::Other { .. })
    }
}

impl fmt::Display for EventLogError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventLogError::AccessDenied { channel } => write!(
                f,
                "Access is denied to the {channel} event log: reading it requires administrator rights. Restart Debrief as administrator, or leave {channel} out."
            ),
            EventLogError::ChannelNotFound { channel } => {
                write!(f, "the event log channel \"{channel}\" does not exist on this computer (or is disabled)")
            }
            EventLogError::InvalidQuery { channel, detail } => {
                write!(f, "the Event Log rejected the filter for {channel}: {detail}. Check the XPath filter.")
            }
            EventLogError::Stale { channel } => write!(f, "the {channel} event log was cleared or overwritten while it was read"),
            EventLogError::Unavailable(why) => write!(f, "the Windows Event Log is not available: {why}"),
            EventLogError::Other { channel, code, message } => {
                write!(f, "reading the {channel} event log failed (error {code}): {message}")
            }
        }
    }
}

impl std::error::Error for EventLogError {}

/// The Windows error codes the Event Log API reports, as typed errors.
pub fn classify(channel: &str, code: u32, message: &str) -> EventLogError {
    let channel = channel.to_string();
    match code {
        5 => EventLogError::AccessDenied { channel },
        15000 | 15007 => EventLogError::ChannelNotFound { channel },
        15001 => EventLogError::InvalidQuery {
            channel,
            detail: if message.trim().is_empty() {
                "the query is invalid".into()
            } else {
                message.trim().trim_end_matches('.').to_string()
            },
        },
        15011 => EventLogError::Stale { channel },
        1717 | 1722 | 1753 => EventLogError::Unavailable(format!("the Windows Event Log service is not running ({message})")),
        _ => EventLogError::Other {
            channel,
            code,
            message: message.trim().to_string(),
        },
    }
}

/// One event as the Event Log rendered it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RawEvent {
    /// `EvtRender(EvtRenderEventXml)`.
    pub xml: String,
    /// `EvtFormatMessage(EvtFormatMessageEvent)`, `None` when the provider's
    /// message table (DLL) is missing.
    pub message: Option<String>,
}

/// What one read of a stream gave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Batch {
    Events(Vec<RawEvent>),
    /// Nothing new within the wait (live streams).
    Idle,
    /// A query has no more events.
    End,
}

/// A query result or a live subscription.
pub trait EventStream: Send {
    /// Up to `max` events, waiting at most `wait` for the first.
    fn next_batch(&mut self, max: usize, wait: Duration) -> Result<Batch, EventLogError>;
}

/// The port to the Event Log service (local computer only).
pub trait EventLog: Send + Sync {
    /// Every channel name the computer has (`EvtOpenChannelEnum`).
    fn channel_names(&self) -> Result<Vec<String>, EventLogError>;
    /// Checks that `channel` can be read with `xpath` (rights, existence, a
    /// valid query) and returns the newest record id, if any.
    fn probe(&self, channel: &Channel, xpath: &str) -> Result<Option<u64>, EventLogError>;
    /// Stored events matching `xpath`, oldest first (`EvtQuery`).
    fn query(&self, channel: &Channel, xpath: &str) -> Result<Box<dyn EventStream>, EventLogError>;
    /// Events matching `xpath` after record `after` (`None`: from the oldest),
    /// then new ones as they are written (`EvtSubscribe`).
    fn subscribe(&self, channel: &Channel, xpath: &str, after: Option<u64>) -> Result<Box<dyn EventStream>, EventLogError>;
}

/// The Event Log of this computer: the Windows API on Windows, an adapter
/// that reports [`EventLogError::Unavailable`] elsewhere.
pub fn system() -> Arc<dyn EventLog> {
    #[cfg(windows)]
    {
        Arc::new(win::WinEventLog)
    }
    #[cfg(not(windows))]
    {
        Arc::new(Unavailable)
    }
}

/// The Event Log where there is none.
#[cfg_attr(windows, allow(dead_code))]
struct Unavailable;

#[cfg_attr(windows, allow(dead_code))]
impl Unavailable {
    fn error() -> EventLogError {
        EventLogError::Unavailable(format!("it exists only on Windows (this is {})", std::env::consts::OS))
    }
}

impl EventLog for Unavailable {
    fn channel_names(&self) -> Result<Vec<String>, EventLogError> {
        Err(Unavailable::error())
    }
    fn probe(&self, _: &Channel, _: &str) -> Result<Option<u64>, EventLogError> {
        Err(Unavailable::error())
    }
    fn query(&self, _: &Channel, _: &str) -> Result<Box<dyn EventStream>, EventLogError> {
        Err(Unavailable::error())
    }
    fn subscribe(&self, _: &Channel, _: &str, _: Option<u64>) -> Result<Box<dyn EventStream>, EventLogError> {
        Err(Unavailable::error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_become_typed_errors_with_user_messages() {
        let denied = classify("Security", 5, "Access is denied.");
        assert_eq!(
            denied,
            EventLogError::AccessDenied {
                channel: "Security".into()
            }
        );
        let text = denied.to_string();
        // The UI offers "Restart as administrator" for these words (tool-errors.ts).
        assert!(text.contains("Access is denied") && text.contains("administrator"), "{text}");
        assert!(!denied.is_transient());
        assert!(matches!(classify("X", 15007, ""), EventLogError::ChannelNotFound { .. }));
        assert!(classify("X", 15000, "").to_string().contains("does not exist"));
        let q = classify("System", 15001, "The specified query is invalid.\r\n");
        assert_eq!(
            q.to_string(),
            "the Event Log rejected the filter for System: The specified query is invalid. Check the XPath filter."
        );
        assert!(classify("System", 15001, " ").to_string().contains("the query is invalid"));
        let stale = classify("System", 15011, "");
        assert!(stale.is_transient() && stale.to_string().contains("cleared"));
        assert!(classify("System", 1717, "x").to_string().contains("service is not running"));
        let other = classify("System", 87, " bad parameter ");
        assert!(other.is_transient());
        assert_eq!(other.to_string(), "reading the System event log failed (error 87): bad parameter");
    }

    #[test]
    fn the_system_log_answers() {
        let log = system();
        #[cfg(not(windows))]
        {
            let c = Channel::parse("System").unwrap();
            assert!(log.channel_names().unwrap_err().to_string().contains("only on Windows"));
            assert!(log.probe(&c, "*").is_err());
            assert!(log.query(&c, "*").is_err());
            assert!(log.subscribe(&c, "*", None).is_err());
        }
        #[cfg(windows)]
        assert!(log.channel_names().unwrap().iter().any(|c| c == "System"));
        let u = Unavailable;
        let c = Channel::parse("System").unwrap();
        assert!(u.channel_names().is_err() && u.probe(&c, "*").is_err());
        assert!(u.query(&c, "*").is_err() && u.subscribe(&c, "*", Some(1)).is_err());
    }
}
