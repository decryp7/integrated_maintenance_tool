//! The Windows adapter of [`EventLog`]: `EvtQuery`/`EvtSubscribe` (pull
//! subscriptions signalled through an event object), `EvtNext`, `EvtRender`
//! (XML) and `EvtFormatMessage` with cached provider metadata.
//!
//! Every wait has a timeout, every buffer is bounded and every handle is
//! closed on drop. Errors are classified into [`EventLogError`].

use super::bookmark::bookmark_xml;
use super::xml::{event_json, provider_name, record_id, MAX_XML_BYTES};
use super::{classify, Batch, Channel, EventLog, EventLogError, EventStream, RawEvent};
use std::collections::HashMap;
use std::time::Duration;
use windows::core::{HSTRING, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::System::EventLog::{
    EvtClose, EvtCreateBookmark, EvtFormatMessage, EvtFormatMessageEvent, EvtNext, EvtNextChannelPath, EvtOpenChannelEnum,
    EvtOpenPublisherMetadata, EvtQuery, EvtQueryChannelPath, EvtQueryForwardDirection, EvtQueryReverseDirection, EvtRender,
    EvtRenderEventXml, EvtSubscribe, EvtSubscribeStartAfterBookmark, EvtSubscribeStartAtOldestRecord, EVT_HANDLE,
};
use windows::Win32::System::Threading::{CreateEventW, ResetEvent, WaitForSingleObject};

const ERROR_INSUFFICIENT_BUFFER: u32 = 122;
const ERROR_NO_MORE_ITEMS: u32 = 259;
const ERROR_TIMEOUT: u32 = 1460;
const ERROR_INVALID_OPERATION: u32 = 4317;
const ERROR_EVT_UNRESOLVED_VALUE_INSERT: u32 = 15029;
/// Most channel names listed.
const MAX_CHANNEL_NAMES: usize = 20_000;
/// Longest formatted message requested from the provider (UTF-16 units).
const MAX_MESSAGE_UNITS: usize = 64 * 1024;
/// Most providers whose metadata stays open per reader.
const MAX_PUBLISHERS: usize = 256;
/// Most events taken from the Event Log in one `EvtNext`.
const MAX_NEXT: usize = 512;
/// How long the probe waits for the first result of a query.
const PROBE_WAIT_MS: u32 = 5_000;
/// How long the probe lets a filtered query look for a match (a rare filter over a
/// big log may scan it all; only errors matter here, so a timeout is fine).
const FILTER_PROBE_WAIT_MS: u32 = 2_000;

/// An Event Log handle, closed on drop.
struct Evt(EVT_HANDLE);

impl Drop for Evt {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            // SAFETY: the handle came from the Event Log API and is closed once.
            unsafe {
                let _ = EvtClose(self.0);
            }
        }
    }
}

/// A manual-reset event object that signals a pull subscription.
struct Signal(HANDLE);

// SAFETY: a kernel event handle may be used and closed from any thread.
unsafe impl Send for Signal {}

impl Drop for Signal {
    fn drop(&mut self) {
        // SAFETY: the handle came from CreateEventW and is closed once.
        unsafe {
            let _ = CloseHandle(self.0);
        }
    }
}

/// The Win32 error code inside an `HRESULT` from `GetLastError`.
fn win32_code(e: &windows::core::Error) -> u32 {
    let hr = e.code().0 as u32;
    if hr & 0xFFFF_0000 == 0x8007_0000 {
        hr & 0xFFFF
    } else {
        hr
    }
}

fn error(channel: &Channel, e: &windows::core::Error) -> EventLogError {
    classify(channel.as_str(), win32_code(e), &e.message())
}

fn text(buf: &[u16], used_units: usize) -> String {
    let n = used_units.min(buf.len());
    let s = &buf[..n];
    let end = s.iter().position(|c| *c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

fn query(channel: &Channel, xpath: &str, reverse: bool) -> Result<Evt, EventLogError> {
    let direction = if reverse {
        EvtQueryReverseDirection
    } else {
        EvtQueryForwardDirection
    };
    let flags = EvtQueryChannelPath.0 | direction.0;
    // SAFETY: valid, NUL-terminated strings; the handle is owned by `Evt`.
    unsafe { EvtQuery(None, &HSTRING::from(channel.as_str()), &HSTRING::from(xpath), flags) }
        .map(Evt)
        .map_err(|e| error(channel, &e))
}

/// Up to `max` events of a result set; empty when there are none (yet).
fn next(set: &Evt, max: usize, timeout_ms: u32, channel: &Channel) -> Result<Option<Vec<Evt>>, EventLogError> {
    let mut raw = vec![0isize; max.clamp(1, MAX_NEXT)];
    let mut returned = 0u32;
    // SAFETY: `raw` has room for `raw.len()` handles; returned ones are owned by `Evt`.
    match unsafe { EvtNext(set.0, &mut raw, timeout_ms, 0, &mut returned) } {
        Ok(()) => Ok(Some(
            raw[..(returned as usize).min(raw.len())]
                .iter()
                .map(|h| Evt(EVT_HANDLE(*h)))
                .collect(),
        )),
        Err(e) => match win32_code(&e) {
            ERROR_NO_MORE_ITEMS => Ok(None),
            ERROR_TIMEOUT => Ok(Some(Vec::new())),
            _ => Err(error(channel, &e)),
        },
    }
}

/// `EvtRender(EvtRenderEventXml)`, bounded.
fn render_xml(event: &Evt) -> Option<String> {
    let mut buf = vec![0u16; 8 * 1024];
    for _ in 0..2 {
        let (mut used, mut count) = (0u32, 0u32);
        let bytes = (buf.len() * 2) as u32;
        // SAFETY: the buffer is `bytes` long; the API writes at most that.
        let r = unsafe {
            EvtRender(
                None,
                event.0,
                EvtRenderEventXml.0,
                bytes,
                Some(buf.as_mut_ptr().cast()),
                &mut used,
                &mut count,
            )
        };
        match r {
            Ok(()) => return Some(text(&buf, used as usize / 2)),
            Err(e) if win32_code(&e) == ERROR_INSUFFICIENT_BUFFER && (used as usize) <= MAX_XML_BYTES * 2 => {
                buf = vec![0u16; used as usize / 2 + 1];
            }
            Err(_) => return None,
        }
    }
    None
}

/// `EvtFormatMessage(EvtFormatMessageEvent)`; `None` when the provider has
/// no message for the event.
fn format_message(publisher: &Evt, event: &Evt) -> Option<String> {
    let mut buf = vec![0u16; 2048];
    for _ in 0..2 {
        let mut used = 0u32;
        // SAFETY: valid handles; the buffer length is passed by the wrapper.
        let r = unsafe {
            EvtFormatMessage(
                Some(publisher.0),
                Some(event.0),
                0,
                None,
                EvtFormatMessageEvent.0,
                Some(&mut buf),
                &mut used,
            )
        };
        match r {
            Ok(()) => return Some(text(&buf, used as usize)),
            Err(e) => match win32_code(&e) {
                ERROR_INSUFFICIENT_BUFFER if (used as usize) <= MAX_MESSAGE_UNITS => buf = vec![0u16; used as usize + 1],
                // Some inserts could not be resolved: the text is still useful.
                ERROR_EVT_UNRESOLVED_VALUE_INSERT if used as usize <= buf.len() && used > 0 => return Some(text(&buf, used as usize)),
                _ => return None,
            },
        }
    }
    None
}

/// Provider metadata (message tables), opened once per provider. A provider
/// without metadata (its message DLL is missing) is remembered as `None`.
#[derive(Default)]
struct Publishers(HashMap<String, Option<Evt>>);

impl Publishers {
    fn get(&mut self, provider: &str) -> Option<&Evt> {
        if !self.0.contains_key(provider) {
            if self.0.len() >= MAX_PUBLISHERS {
                self.0.clear();
            }
            // SAFETY: a valid provider name; the handle is owned by `Evt`.
            let opened = unsafe { EvtOpenPublisherMetadata(None, &HSTRING::from(provider), PCWSTR::null(), 0, 0) }
                .ok()
                .map(Evt);
            self.0.insert(provider.to_string(), opened);
        }
        self.0.get(provider).and_then(Option::as_ref)
    }

    fn raw(&mut self, event: &Evt) -> RawEvent {
        let xml = render_xml(event).unwrap_or_default();
        let message = provider_name(&xml).and_then(|p| self.get(&p).and_then(|meta| format_message(meta, event)));
        RawEvent { xml, message }
    }
}

struct QueryStream {
    set: Evt,
    channel: Channel,
    publishers: Publishers,
}

impl EventStream for QueryStream {
    fn next_batch(&mut self, max: usize, wait: Duration) -> Result<Batch, EventLogError> {
        let timeout = wait.as_millis().min(u32::MAX as u128 - 1) as u32;
        match next(&self.set, max, timeout, &self.channel)? {
            None => Ok(Batch::End),
            Some(events) if events.is_empty() => Ok(Batch::Idle),
            Some(events) => Ok(Batch::Events(events.iter().map(|e| self.publishers.raw(e)).collect())),
        }
    }
}

struct SubscriptionStream {
    // Declared first: closed before the bookmark and the signal.
    subscription: Evt,
    _bookmark: Option<Evt>,
    signal: Signal,
    channel: Channel,
    publishers: Publishers,
}

impl SubscriptionStream {
    fn take(&mut self, max: usize) -> Result<Vec<RawEvent>, EventLogError> {
        // A pull subscription with nothing to deliver answers ERROR_NO_MORE_ITEMS or (seen on
        // Windows 11, e.g. when started after a bookmark) ERROR_INVALID_OPERATION; neither ends it.
        let events = match next(&self.subscription, max, 0, &self.channel) {
            Err(EventLogError::Other {
                code: ERROR_INVALID_OPERATION,
                ..
            }) => Vec::new(),
            other => other?.unwrap_or_default(),
        };
        Ok(events.iter().map(|e| self.publishers.raw(e)).collect())
    }
}

impl EventStream for SubscriptionStream {
    fn next_batch(&mut self, max: usize, wait: Duration) -> Result<Batch, EventLogError> {
        let events = self.take(max)?;
        if !events.is_empty() {
            return Ok(Batch::Events(events));
        }
        let timeout = wait.as_millis().min(u32::MAX as u128 - 1) as u32;
        // SAFETY: a valid event handle owned by `self.signal`.
        let signalled = unsafe { WaitForSingleObject(self.signal.0, timeout) } == WAIT_OBJECT_0;
        if signalled {
            // Reset before reading so an event written meanwhile signals again.
            // SAFETY: as above.
            unsafe {
                let _ = ResetEvent(self.signal.0);
            }
        }
        let events = self.take(max)?;
        Ok(if events.is_empty() { Batch::Idle } else { Batch::Events(events) })
    }
}

/// The local Event Log through the Windows API.
#[derive(Default)]
pub(super) struct WinEventLog;

impl EventLog for WinEventLog {
    fn channel_names(&self) -> Result<Vec<String>, EventLogError> {
        let unavailable = |e: windows::core::Error| EventLogError::Unavailable(e.message().trim().to_string());
        // SAFETY: local session; the handle is owned by `Evt`.
        let list = unsafe { EvtOpenChannelEnum(None, 0) }.map(Evt).map_err(unavailable)?;
        let mut names = Vec::new();
        let mut buf = vec![0u16; 512];
        while names.len() < MAX_CHANNEL_NAMES {
            let mut used = 0u32;
            // SAFETY: the buffer length is passed by the wrapper.
            match unsafe { EvtNextChannelPath(list.0, Some(&mut buf), &mut used) } {
                Ok(()) => names.push(text(&buf, used as usize)),
                Err(e) => match win32_code(&e) {
                    ERROR_NO_MORE_ITEMS => break,
                    ERROR_INSUFFICIENT_BUFFER if (used as usize) <= 4096 => buf = vec![0u16; used as usize + 1],
                    _ => return Err(unavailable(e)),
                },
            }
        }
        Ok(names)
    }

    fn probe(&self, channel: &Channel, xpath: &str) -> Result<Option<u64>, EventLogError> {
        // Some invalid queries fail only on the first read: read one event.
        let filtered = query(channel, xpath, true)?;
        next(&filtered, 1, FILTER_PROBE_WAIT_MS, channel)?;
        let newest = query(channel, "*", true)?;
        let Some(events) = next(&newest, 1, PROBE_WAIT_MS, channel)? else {
            return Ok(None);
        };
        Ok(events
            .first()
            .and_then(render_xml)
            .and_then(|xml| event_json(&xml).ok())
            .and_then(|json| record_id(&json)))
    }

    fn query(&self, channel: &Channel, xpath: &str) -> Result<Box<dyn EventStream>, EventLogError> {
        Ok(Box::new(QueryStream {
            set: query(channel, xpath, false)?,
            channel: channel.clone(),
            publishers: Publishers::default(),
        }))
    }

    fn subscribe(&self, channel: &Channel, xpath: &str, after: Option<u64>) -> Result<Box<dyn EventStream>, EventLogError> {
        let other = |e: windows::core::Error| error(channel, &e);
        // Manual reset, initially set: the first read does not wait.
        // SAFETY: an unnamed event with default security; owned by `Signal`.
        let signal = Signal(unsafe { CreateEventW(None, true, true, PCWSTR::null()) }.map_err(other)?);
        let bookmark = match after {
            // SAFETY: valid bookmark XML; the handle is owned by `Evt`.
            Some(id) => Some(Evt(
                unsafe { EvtCreateBookmark(&HSTRING::from(bookmark_xml(channel, id))) }.map_err(other)?
            )),
            None => None,
        };
        let flags = if bookmark.is_some() {
            EvtSubscribeStartAfterBookmark.0
        } else {
            EvtSubscribeStartAtOldestRecord.0
        };
        // SAFETY: valid strings and handles; no callback (pull subscription).
        let subscription = unsafe {
            EvtSubscribe(
                None,
                Some(signal.0),
                &HSTRING::from(channel.as_str()),
                &HSTRING::from(xpath),
                bookmark.as_ref().map(|b| b.0),
                None,
                None,
                flags,
            )
        }
        .map(Evt)
        .map_err(other)?;
        Ok(Box::new(SubscriptionStream {
            subscription,
            _bookmark: bookmark,
            signal,
            channel: channel.clone(),
            publishers: Publishers::default(),
        }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn providers_without_metadata_fall_back_and_are_cached() {
        let mut p = Publishers::default();
        assert!(p.get("IMT-No-Such-Provider-For-Tests").is_none());
        assert!(p.0.contains_key("IMT-No-Such-Provider-For-Tests"), "the miss is remembered");
        assert!(p.get("Service Control Manager").is_some());
        for i in 0..MAX_PUBLISHERS + 1 {
            p.get(&format!("IMT-Missing-{i}"));
        }
        assert!(p.0.len() <= MAX_PUBLISHERS, "the cache is bounded");
    }

    #[test]
    fn hresults_unwrap_to_win32_codes() {
        let e = windows::core::Error::from_hresult(windows::core::HRESULT::from_win32(5));
        assert_eq!(win32_code(&e), 5);
        let e = windows::core::Error::from_hresult(windows::core::HRESULT(0x8000_4005u32 as i32));
        assert_eq!(win32_code(&e), 0x8000_4005);
        assert_eq!(text(&[104, 105, 0, 120], 4), "hi");
        assert_eq!(text(&[104, 105], 9), "hi");
    }

    #[test]
    fn missing_channels_and_bad_queries_are_typed_errors() {
        let log = WinEventLog;
        let system = Channel::parse("System").unwrap();
        let missing = Channel::parse("Microsoft-Windows-ImtDoesNotExist/Operational").unwrap();
        assert!(
            matches!(log.probe(&missing, "*"), Err(EventLogError::ChannelNotFound { .. })),
            "{:?}",
            log.probe(&missing, "*")
        );
        let bad = log.probe(&system, "*[System[EventID=]]");
        assert!(matches!(bad, Err(EventLogError::InvalidQuery { .. })), "{bad:?}");
        assert!(log.probe(&system, "*").unwrap().is_some(), "System has events");
        // Combined level, window and user predicates are accepted.
        let x = super::super::query::build_xpath(
            &super::super::LevelSet::new([super::super::EventLevel::Information, super::super::EventLevel::Error]),
            super::super::Backfill::Day,
            super::super::XPathFilter::parse("*[System[Provider[@Name='Service Control Manager']]]")
                .unwrap()
                .as_ref(),
        );
        log.probe(&system, &x).unwrap();
        let mut stream = log.query(&system, &x).unwrap();
        let mut seen = 0;
        for _ in 0..20 {
            match stream.next_batch(16, Duration::from_millis(500)).unwrap() {
                Batch::Events(e) => {
                    for raw in &e {
                        let r = super::super::xml::record_from_xml(&raw.xml, raw.message.as_deref(), 1).unwrap();
                        assert_eq!(r.fields["Provider"], "Service Control Manager");
                    }
                    seen += e.len();
                }
                Batch::Idle => {}
                Batch::End => break,
            }
        }
        let _ = seen;
    }
}
