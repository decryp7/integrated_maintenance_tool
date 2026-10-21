use super::*;
use imt_plugin_api::tools::TaskSink;
use imt_plugin_windows::eventlog::{Batch, Channel, EventLogError, EventStream, RawEvent};

fn xml(channel: &str, id: u64) -> String {
    format!(
        "<Event><System><Provider Name='Service Control Manager'/><EventID>7036</EventID><Level>4</Level>\
         <TimeCreated SystemTime='2026-10-17T10:00:00.000Z'/><EventRecordID>{id}</EventRecordID>\
         <Channel>{channel}</Channel><Computer>pc1</Computer></System>\
         <EventData><Data Name='param1'>Spooler</Data><Data Name='param2'>running</Data></EventData></Event>"
    )
}

/// A log whose `System` channel grows on demand; `Security` is refused.
#[derive(Default)]
pub(crate) struct FakeLog {
    system: Mutex<Vec<u64>>,
}

impl FakeLog {
    fn append(&self) {
        let mut s = self.system.lock().unwrap();
        let next = s.last().map_or(1, |l| l + 1);
        s.push(next);
    }
}

struct Live {
    log: Arc<FakeLog>,
    after: u64,
    /// A query: ends when nothing is left.
    query: bool,
}

impl EventStream for Live {
    fn next_batch(&mut self, max: usize, wait: Duration) -> Result<Batch, EventLogError> {
        let ids: Vec<u64> = self
            .log
            .system
            .lock()
            .unwrap()
            .iter()
            .copied()
            .filter(|i| *i > self.after)
            .take(max)
            .collect();
        if ids.is_empty() && self.query {
            return Ok(Batch::End);
        }
        if ids.is_empty() {
            std::thread::sleep(wait.min(Duration::from_millis(5)));
            return Ok(Batch::Idle);
        }
        self.after = *ids.last().unwrap();
        Ok(Batch::Events(
            ids.iter()
                .map(|i| RawEvent {
                    xml: xml("System", *i),
                    message: Some(format!("The Spooler service entered the running state ({i}).")),
                })
                .collect(),
        ))
    }
}

struct Shared(Arc<FakeLog>);

impl EventLog for Shared {
    fn channel_names(&self) -> Result<Vec<String>, EventLogError> {
        Ok(vec![
            "System".into(),
            "Microsoft-Windows-TaskScheduler/Operational".into(),
            "Microsoft-Windows-Kernel-PnP/Configuration".into(),
        ])
    }

    fn probe(&self, channel: &Channel, _: &str) -> Result<Option<u64>, EventLogError> {
        if channel.needs_admin() {
            return Err(EventLogError::AccessDenied {
                channel: channel.to_string(),
            });
        }
        Ok(self.0.system.lock().unwrap().last().copied())
    }

    fn query(&self, _: &Channel, _: &str) -> Result<Box<dyn EventStream>, EventLogError> {
        Ok(Box::new(Live {
            log: self.0.clone(),
            after: 0,
            query: true,
        }))
    }

    fn subscribe(&self, _: &Channel, _: &str, after: Option<u64>) -> Result<Box<dyn EventStream>, EventLogError> {
        Ok(Box::new(Live {
            log: self.0.clone(),
            after: after.unwrap_or(0),
            query: false,
        }))
    }
}

#[derive(Default)]
struct Tasks(Mutex<Vec<String>>);
impl TaskSink for Tasks {
    fn started(&self, tool: &str, t: &ToolTask) {
        self.0.lock().unwrap().push(format!("+{tool} {} {}", t.id, t.kind));
    }
    fn progress(&self, _: &str, _: &str, _: Option<f32>) {}
    fn finished(&self, tool: &str, task: &str) {
        self.0.lock().unwrap().push(format!("-{tool} {task}"));
    }
}

fn tool() -> (EventLogTool, Arc<FakeLog>) {
    let log = Arc::new(FakeLog::default());
    let limits = Limits {
        wait: Duration::from_millis(10),
        retry: Duration::from_millis(20),
        save_every: Duration::from_millis(10),
        ..Limits::default()
    };
    (EventLogTool::with_limits(Arc::new(Shared(log.clone())), limits), log)
}

fn temp(tag: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("imt-tools-eventlog-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    d
}

fn wait_until(what: &str, mut f: impl FnMut() -> bool) {
    let deadline = std::time::Instant::now() + Duration::from_secs(10);
    while !f() {
        assert!(std::time::Instant::now() < deadline, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn clef_lines(dir: &std::path::Path) -> Vec<Value> {
    let text = std::fs::read_to_string(dir.join("eventlog.clef")).unwrap_or_default();
    text.lines().map(|l| serde_json::from_str(l).unwrap()).collect()
}

#[test]
fn describes_a_windows_only_tool_and_lists_channels() {
    let (t, _) = tool();
    let info = t.describe();
    assert!(info.check().is_ok(), "{:?}", info.check());
    assert_eq!(info.platforms, [Platform::Windows]);
    assert_eq!(t.id(), EVENT_LOG);
    let ctx = ToolCtx::new(None, Arc::new(Tasks::default()));
    let channels = t.invoke("channels", json!({}), &ctx).unwrap();
    let names: Vec<&str> = channels.as_array().unwrap().iter().map(|c| c["name"].as_str().unwrap()).collect();
    assert_eq!(
        names,
        [
            "System",
            "Application",
            "Security",
            "Setup",
            "ForwardedEvents",
            "Microsoft-Windows-TaskScheduler/Operational"
        ]
    );
    assert_eq!(channels[2]["needsAdmin"], true);
    assert_eq!(channels[0]["needsAdmin"], false);
    assert!(t.invoke("nope", json!({}), &ctx).is_err());
}

#[test]
fn validate_reports_each_channel_and_the_query() {
    let (t, _) = tool();
    let ctx = ToolCtx::new(None, Arc::new(Tasks::default()));
    let v = t
        .invoke(
            "validate",
            json!({ "channels": ["System", "Security"], "levels": ["error"], "xpath": "*[System[EventID=7036]]", "backfill": "1h" }),
            &ctx,
        )
        .unwrap();
    assert_eq!(v["xpath"], "*[System[(Level=2)] and (System[EventID=7036])]");
    assert!(v["backfillXpath"].as_str().unwrap().contains("timediff"));
    assert_eq!(v["channels"][0], json!({ "name": "System", "ok": true, "error": null }));
    assert_eq!(v["channels"][1]["ok"], false);
    assert!(v["channels"][1]["error"].as_str().unwrap().contains("Access is denied"));
    let v = t.invoke("validate", json!({ "channels": ["System"] }), &ctx).unwrap();
    assert_eq!((v["xpath"].as_str(), &v["backfillXpath"]), (Some("*"), &Value::Null));
    let e = t
        .invoke("validate", json!({ "channels": ["System"], "xpath": "System[" }), &ctx)
        .unwrap_err();
    assert!(e.to_string().contains("unclosed"), "{e}");
}

#[test]
fn starts_writes_clef_pauses_and_resumes_after_a_restart() {
    let (t, log) = tool();
    let data = temp("data");
    let tasks = Arc::new(Tasks::default());
    let ctx = ToolCtx::new(Some(data.clone()), tasks.clone()).for_tool(EVENT_LOG);
    assert_eq!(
        t.invoke("status", json!({}), &ctx).unwrap(),
        json!({ "running": false, "dir": null })
    );
    assert!(t.invoke("pause", json!({}), &ctx).unwrap_err().to_string().contains("not running"));
    log.append();
    let started = t
        .invoke(
            "start",
            json!({ "channels": ["System"], "backfill": "24h", "maxFileMb": 1, "maxFiles": 2 }),
            &ctx,
        )
        .unwrap();
    let dir = PathBuf::from(started["dir"].as_str().unwrap());
    assert!(dir.starts_with(data.join("eventlog")), "{dir:?}");
    assert_eq!(started["xpath"], "*");
    assert_eq!(started["resumed"], json!([]));
    let again = t.invoke("start", json!({ "channels": ["System"] }), &ctx).unwrap_err();
    assert_eq!(again.to_string(), "the event log follower is already running");
    wait_until("the backfilled event", || clef_lines(&dir).len() == 1);
    let paused = t.invoke("pause", json!({}), &ctx).unwrap();
    assert_eq!(paused["paused"], true);
    let resumed = t.invoke("resume", json!({}), &ctx).unwrap();
    assert_eq!(resumed["paused"], false);
    log.append();
    wait_until("the live event", || clef_lines(&dir).len() == 2);
    let status = t.invoke("status", json!({}), &ctx).unwrap();
    assert_eq!(status["running"], true);
    assert_eq!(status["events"], 2);
    assert_eq!(status["dir"], json!(dir.to_string_lossy()));
    assert_eq!(status["channels"][0]["state"], "live");
    assert_eq!(status["lastEvent"]["provider"], "Service Control Manager");
    let line = &clef_lines(&dir)[1];
    // The same fields as the .evtx importer, plus the formatted message.
    assert_eq!(line["Provider"], "Service Control Manager");
    assert_eq!(line["EventId"], 7036);
    assert_eq!(line["RecordId"], 2);
    assert_eq!(line["param1"], "Spooler");
    assert_eq!(line["@m"], "The Spooler service entered the running state (2).");
    assert_eq!(line["@t"], "2026-10-17T10:00:00.000Z");
    let stopped = t.invoke("stop", json!({}), &ctx).unwrap();
    assert_eq!(stopped, json!(dir.to_string_lossy()));
    assert_eq!(t.invoke("stop", json!({}), &ctx).unwrap(), Value::Null);
    assert!(data.join("eventlog").join("bookmarks.json").is_file());
    assert_eq!(
        *tasks.0.lock().unwrap(),
        ["+event-log-follower eventlog event-log-follower", "-event-log-follower eventlog"]
    );

    // A new run resumes after the bookmark: only the event written meanwhile.
    log.append();
    let started = t
        .invoke("start", json!({ "channels": ["System"], "backfill": "24h" }), &ctx)
        .unwrap();
    assert_eq!(started["resumed"], json!(["System"]));
    let dir2 = PathBuf::from(started["dir"].as_str().unwrap());
    wait_until("the missed event", || clef_lines(&dir2).len() == 1);
    assert_eq!(clef_lines(&dir2)[0]["RecordId"], 3);
    t.stop(TASK, &ctx).unwrap();
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn missing_rights_and_folders_are_refused_up_front() {
    let (t, _) = tool();
    let tasks = Arc::new(Tasks::default());
    let data = temp("denied");
    let ctx = ToolCtx::new(Some(data.clone()), tasks.clone());
    let e = t
        .invoke("start", json!({ "channels": ["System", "Security"] }), &ctx)
        .unwrap_err()
        .to_string();
    // tool-errors.ts offers "Restart as administrator" for this text.
    assert!(e.contains("Access is denied") && e.contains("administrator"), "{e}");
    assert!(tasks.0.lock().unwrap().is_empty(), "nothing started");
    assert_eq!(t.status().unwrap()["running"], false);
    let no_data = ToolCtx::new(None, Arc::new(Tasks::default()));
    let e = t.invoke("start", json!({ "channels": ["System"] }), &no_data).unwrap_err();
    assert_eq!(e.to_string(), "choose a folder for the event log files");
    let e = t.invoke("start", json!({ "channels": [] }), &no_data).unwrap_err();
    assert!(e.to_string().contains("at least one"), "{e}");
    assert!(t.stop("monitor", &ctx).is_err());
    let _ = std::fs::remove_dir_all(data);
}

#[test]
fn a_failed_write_ends_the_background_task() {
    let (t, log) = tool();
    let dir = temp("gone");
    let tasks = Arc::new(Tasks::default());
    let ctx = ToolCtx::new(None, tasks.clone()).for_tool(EVENT_LOG);
    t.invoke("start", json!({ "channels": ["System"], "dir": dir }), &ctx).unwrap();
    wait_until("live", || t.status().unwrap()["channels"][0]["state"] == "live");
    // The folder disappears (and a file takes its place): the next write fails.
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::write(&dir, b"in the way").unwrap();
    log.append();
    wait_until("the end of the run", || t.status().unwrap()["running"] == false);
    assert!(tasks.0.lock().unwrap().contains(&"-event-log-follower eventlog".to_string()));
    let status = t.status().unwrap();
    assert!(status["errors"][0]["message"]
        .as_str()
        .unwrap()
        .contains("writing the event log files failed"));
    let e = t.invoke("stop", json!({}), &ctx).unwrap_err();
    assert!(e.to_string().contains("writing the event log files failed"), "{e}");
    let _ = std::fs::remove_file(&dir);
}
