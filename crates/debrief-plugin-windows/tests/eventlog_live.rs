//! The Event Log follower against this computer's real Event Log: an event
//! written with `eventcreate` is picked up live with the importer's fields
//! and its formatted message, a restart resumes after the bookmark without
//! repeating it, and a backfill reads it back with `EvtQuery`.
//!
//! Skipped (not failed) off Windows, and when `eventcreate` cannot register
//! its test source (that needs administrator rights).

#[cfg(not(windows))]
#[test]
fn follows_the_real_event_log() {
    eprintln!("skipped: the Windows Event Log exists only on Windows");
}

#[cfg(windows)]
mod live {
    use debrief_plugin_api::clef::EventSink;
    use debrief_plugin_api::model::{Level, Record};
    use debrief_plugin_windows::eventlog::bookmark::BookmarkStore;
    use debrief_plugin_windows::eventlog::follower::{ChannelState, Limits};
    use debrief_plugin_windows::eventlog::{system, Channel, Follower, FollowerConfig};
    use serde_json::json;
    use std::path::{Path, PathBuf};
    use std::process::Stdio;
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const SOURCE: &str = "Debrief-Follower-Test";
    const EVENT_ID: u32 = 913;

    #[derive(Default)]
    struct Mem(Mutex<Vec<Record>>);

    impl EventSink for Mem {
        fn write(&self, records: &[Record]) -> anyhow::Result<()> {
            self.0.lock().unwrap().extend_from_slice(records);
            Ok(())
        }
    }

    impl Mem {
        fn with(&self, text: &str) -> Vec<Record> {
            self.0
                .lock()
                .unwrap()
                .iter()
                .filter(|r| r.message.contains(text))
                .cloned()
                .collect()
        }
    }

    /// Writes an Application event with `eventcreate`; false when it cannot.
    fn write_event(description: &str) -> bool {
        let child = debrief_plugin_api::process::quiet_command("eventcreate")
            .args([
                "/T",
                "INFORMATION",
                "/ID",
                &EVENT_ID.to_string(),
                "/L",
                "APPLICATION",
                "/SO",
                SOURCE,
                "/D",
                description,
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        let Ok(mut child) = child else { return false };
        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => return status.success(),
                Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(50)),
                _ => {
                    let _ = child.kill();
                    return false;
                }
            }
        }
    }

    fn wait_for(what: &str, mut f: impl FnMut() -> bool) {
        let deadline = Instant::now() + Duration::from_secs(20);
        while !f() {
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    fn config(resume: bool, backfill: &str) -> FollowerConfig {
        FollowerConfig::from_json(&json!({
            "channels": ["Application"],
            "levels": ["information"],
            "xpath": format!("*[System[Provider[@Name='{SOURCE}'] and EventID={EVENT_ID}]]"),
            "backfill": backfill,
            "resumeFromBookmark": resume,
        }))
        .unwrap()
    }

    fn start(sink: &Arc<Mem>, store: &Path, resume: bool, backfill: &str) -> Follower {
        let limits = Limits {
            wait: Duration::from_millis(100),
            ..Limits::default()
        };
        let f = Follower::start(
            system(),
            config(resume, backfill),
            sink.clone(),
            Some(BookmarkStore::load(store)),
            limits,
            Box::new(|| {}),
        )
        .unwrap();
        wait_for("the live subscription", || f.status().channels[0].state == ChannelState::Live);
        f
    }

    #[test]
    fn follows_the_real_event_log() {
        let nonce = format!("debrief-follower-{}-{}", std::process::id(), chrono::Utc::now().timestamp_millis());
        let dir: PathBuf = std::env::temp_dir().join(&nonce);
        let store = dir.join("bookmarks.json");
        let sink = Arc::new(Mem::default());

        let f = start(&sink, &store, true, "none");
        let first = format!("{nonce} first");
        if !write_event(&first) {
            eprintln!("skipped: eventcreate could not write to the Application log (run elevated to register its source)");
            f.stop().unwrap();
            return;
        }
        wait_for("the live event", || !sink.with(&first).is_empty());
        let status = f.status();
        assert!(status.events >= 1 && status.last_event.is_some(), "{status:?}");
        f.stop().unwrap();
        let got = sink.with(&first);
        assert_eq!(got.len(), 1, "{got:?}");
        let r = &got[0];
        assert_eq!(r.fields["Provider"], SOURCE);
        assert_eq!(r.fields["EventId"], EVENT_ID);
        assert_eq!(r.fields["Channel"], "Application");
        assert!(r.fields["RecordId"].as_u64().unwrap() > 0);
        assert!(r.fields.contains_key("Computer"));
        assert_eq!(r.level, Level::Information);
        assert!(r.ts.is_some_and(|t| (chrono::Utc::now().timestamp_millis() - t).abs() < 120_000));
        // eventcreate's message table formats the description as the message.
        assert!(r.message.contains(&first), "{}", r.message);
        assert!(
            !r.message.starts_with(&format!("{SOURCE} {EVENT_ID}:")),
            "formatted, not the fallback: {}",
            r.message
        );
        let id = r.fields["RecordId"].as_u64().unwrap();
        let app = Channel::parse("Application").unwrap();
        assert_eq!(BookmarkStore::load(&store).get(&app), Some(id), "the position was saved");

        // Written while the follower is stopped: the restart resumes after the
        // bookmark, so this one arrives and the first does not repeat.
        let second = format!("{nonce} second");
        assert!(write_event(&second));
        let f = start(&sink, &store, true, "none");
        wait_for("the event written while stopped", || !sink.with(&second).is_empty());
        std::thread::sleep(Duration::from_millis(300));
        f.stop().unwrap();
        assert_eq!(sink.with(&first).len(), 1, "no duplicate after the restart");
        assert_eq!(sink.with(&second).len(), 1);

        // Afresh with a backfill window: EvtQuery reads both back.
        let fresh = Arc::new(Mem::default());
        let f = start(&fresh, &store, false, "1h");
        wait_for("the backfill", || !fresh.with(&first).is_empty() && !fresh.with(&second).is_empty());
        f.stop().unwrap();
        assert_eq!(fresh.with(&nonce).len(), 2);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn offers_the_real_channels() {
        let names = system().channel_names().unwrap();
        let offered = debrief_plugin_windows::eventlog::query::offered_channels(names);
        assert_eq!(offered[0].as_str(), "System");
        assert!(offered.iter().any(|c| c.as_str().ends_with("/Operational")), "{offered:?}");
    }
}
