#![no_main]
//! The Event Log follower's untrusted inputs: rendered event XML, the user's
//! XPath filter and channel names, and the bookmark file.
use debrief_plugin_windows::eventlog::{query, xml, Channel, FollowerConfig, XPathFilter};
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let text = String::from_utf8_lossy(data);
    if let Ok(r) = xml::record_from_xml(&text, Some(&text), 1) {
        // Records cross the IPC boundary as JSON.
        serde_json::to_string(&r.fields).expect("fields serialize");
        assert!(r.message.chars().count() <= xml::MAX_MESSAGE_CHARS.max(1000));
    }
    let _ = xml::provider_name(&text);
    if let Ok(Some(f)) = XPathFilter::parse(&text) {
        let x = query::build_xpath(&query::LevelSet::default(), query::Backfill::Day, Some(&f));
        assert!(x.starts_with("*["));
    }
    let _ = Channel::parse(&text);
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(data) {
        let _ = FollowerConfig::from_json(&v);
    }
});
