//! The Event Log follower as a tool: one app-wide run that follows Windows
//! event log channels into rolling CLEF files (opened with live tail, like
//! the process monitor). The engine is `debrief_plugin_windows::eventlog`.
//!
//! Positions are kept in `<data>/eventlog/bookmarks.json`, outside the folder
//! Debrief opens, so a new run continues where the last one stopped.

use crate::ids::EVENT_LOG;
use crate::run_slot::{Run, RunSlot, Seen};
use anyhow::{anyhow, bail, Result};
use debrief_plugin_api::clef::ClefFileSink;
use debrief_plugin_api::tools::{OpSpec, PathAccess, PathKind, Platform, Schema, Tool, ToolCtx, ToolInfo, ToolTask};
use debrief_plugin_windows::eventlog::bookmark::BookmarkStore;
use debrief_plugin_windows::eventlog::follower::Limits;
use debrief_plugin_windows::eventlog::query::{offered_channels, Backfill, EventLevel, MAX_CHANNELS, MAX_CHANNEL_LEN, MAX_XPATH_LEN};
use debrief_plugin_windows::eventlog::{EventLog, Follower, FollowerConfig};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Id of the running follower in the background task list.
pub(crate) const TASK: &str = "eventlog";
const DEFAULT_FILE_MB: i64 = 10;
const DEFAULT_FILES: i64 = 10;

impl Run for Follower {
    fn is_running(&self) -> bool {
        Follower::is_running(self)
    }

    fn snapshot(&self) -> Value {
        serde_json::to_value(self.status()).unwrap_or(Value::Null)
    }

    fn finish(self) -> Result<()> {
        self.stop()
    }
}

pub(crate) struct EventLogTool {
    log: Arc<dyn EventLog>,
    limits: Limits,
    run: RunSlot<Follower>,
}

/// The follower settings shared by `validate` and `start`.
fn query_args(schema: Schema) -> Schema {
    schema
        .field("channels", Schema::list(Schema::text(MAX_CHANNEL_LEN), MAX_CHANNELS))
        .optional(
            "levels",
            Schema::list(Schema::one_of(EventLevel::ALL.map(EventLevel::name)), EventLevel::ALL.len()),
        )
        .optional("xpath", Schema::text(MAX_XPATH_LEN))
        .optional("backfill", Schema::one_of(Backfill::NAMES))
}

impl EventLogTool {
    pub fn new(log: Arc<dyn EventLog>) -> EventLogTool {
        Self::with_limits(log, Limits::default())
    }

    pub fn with_limits(log: Arc<dyn EventLog>, limits: Limits) -> EventLogTool {
        EventLogTool {
            log,
            limits,
            run: RunSlot::default(),
        }
    }

    fn channels(&self) -> Result<Value> {
        let names = self.log.channel_names()?;
        let offered: Vec<Value> = offered_channels(names)
            .into_iter()
            .map(|c| json!({ "name": c.as_str(), "needsAdmin": c.needs_admin() }))
            .collect();
        Ok(Value::Array(offered))
    }

    /// Checks each channel with the final query, without starting anything.
    fn validate(&self, args: &Value) -> Result<Value> {
        let config = FollowerConfig::from_json(args)?;
        let query = if config.backfill == Backfill::None {
            config.live_xpath()
        } else {
            config.backfill_xpath()
        };
        let channels: Vec<Value> = config
            .channels
            .iter()
            .map(|c| match self.log.probe(c, &query) {
                Ok(_) => json!({ "name": c.as_str(), "ok": true, "error": null }),
                Err(e) => json!({ "name": c.as_str(), "ok": false, "error": e.to_string() }),
            })
            .collect();
        Ok(json!({
            "xpath": config.live_xpath(),
            "backfillXpath": (config.backfill != Backfill::None).then(|| config.backfill_xpath()),
            "channels": channels,
        }))
    }

    fn start(&self, args: Value, ctx: &ToolCtx) -> Result<Value> {
        let mut reply = Value::Null;
        self.run.begin("the event log follower is already running", || {
            self.start_run(args, ctx, &mut reply)
        })?;
        ctx.task_started(ToolTask {
            id: TASK.into(),
            label: "Event Log follower".into(),
            kind: "event-log-follower".into(),
            stoppable: true,
        });
        Ok(reply)
    }

    /// Starts the follower; it and its folder, and the reply to `start` in `reply`.
    fn start_run(&self, args: Value, ctx: &ToolCtx, reply: &mut Value) -> Result<(Follower, PathBuf)> {
        let config = FollowerConfig::from_json(&args)?;
        let dir = match args.get("dir").and_then(Value::as_str) {
            Some(d) => PathBuf::from(d),
            None => {
                let base = ctx
                    .data_dir()
                    .ok_or_else(|| anyhow!("choose a folder for the event log files"))?
                    .join("eventlog");
                let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
                // A run per folder, even when two start within a second.
                (1..100)
                    .map(|i| {
                        if i == 1 {
                            base.join(&stamp)
                        } else {
                            base.join(format!("{stamp}-{i}"))
                        }
                    })
                    .find(|d| !d.exists())
                    .ok_or_else(|| anyhow!("too many event log runs in {}", base.display()))?
            }
        };
        let file_mb = args
            .get("maxFileMb")
            .and_then(Value::as_i64)
            .unwrap_or(DEFAULT_FILE_MB)
            .clamp(1, 100) as u64;
        let files = args.get("maxFiles").and_then(Value::as_i64).unwrap_or(DEFAULT_FILES).clamp(1, 50) as usize;
        let bookmarks = ctx
            .data_dir()
            .map(|d| BookmarkStore::load(d.join("eventlog").join("bookmarks.json")));
        let resumed: Vec<&str> = match &bookmarks {
            Some(b) if config.resume => config.channels.iter().filter(|c| b.get(c).is_some()).map(|c| c.as_str()).collect(),
            _ => Vec::new(),
        };
        let resumed: Vec<String> = resumed.into_iter().map(str::to_string).collect();
        let xpath = config.live_xpath();
        let sink = Arc::new(ClefFileSink::new(&dir, "eventlog", file_mb * 1024 * 1024, files)?);
        let done = ctx.clone();
        let follower = Follower::start(
            self.log.clone(),
            config,
            sink,
            bookmarks,
            self.limits.clone(),
            Box::new(move || done.task_finished(TASK)),
        )?;
        *reply = json!({ "dir": dir.to_string_lossy(), "xpath": xpath, "resumed": resumed });
        Ok((follower, dir))
    }

    /// Stops the run; its folder, or `None` when nothing was running.
    fn stop_run(&self, ctx: &ToolCtx) -> Result<Option<String>> {
        let r = self.run.end();
        if !matches!(r, Ok(None)) {
            ctx.task_finished(TASK);
        }
        r
    }

    fn set_paused(&self, paused: bool) -> Result<Value> {
        self.run.look(|s| match s {
            Seen::Running { run, .. } => {
                if paused {
                    run.pause();
                } else {
                    run.resume();
                }
                Ok(())
            }
            _ => Err(anyhow!("the event log follower is not running")),
        })?;
        self.status()
    }

    fn status(&self) -> Result<Value> {
        self.run.look(|s| {
            let (mut status, dir, error) = match s {
                Seen::Idle => return Ok(json!({ "running": false, "dir": null })),
                Seen::Running { run, dir } => (serde_json::to_value(run.status())?, dir, None),
                // The last word of a run that ended on its own, and why.
                Seen::Ended { dir, error, last } => (last.clone(), dir, Some(error.unwrap_or_default())),
            };
            if let Value::Object(map) = &mut status {
                map.insert("dir".into(), json!(dir.to_string_lossy()));
                if let Some(e) = error {
                    map.insert("running".into(), json!(false));
                    map.insert("error".into(), json!(e));
                }
            }
            Ok(status)
        })
    }
}

impl Tool for EventLogTool {
    fn id(&self) -> &str {
        EVENT_LOG
    }

    fn describe(&self) -> ToolInfo {
        let quick = Some(Duration::from_secs(10));
        ToolInfo::new(EVENT_LOG, "Event Log follower")
            .describe(
                "Follows Windows event log channels (optionally reading back the last hour, day or week first) into rolling CLEF files that open with live tail. The Security channel needs administrator rights.",
            )
            .only_on(&[Platform::Windows])
            .op(OpSpec::new("channels", "Channels to follow").budget(Some(Duration::from_secs(20))))
            .op(OpSpec::new("validate", "Check channels and filter")
                .args(query_args(Schema::object()))
                .budget(Some(Duration::from_secs(60))))
            .op(OpSpec::new("start", "Start following")
                .args(
                    query_args(Schema::object())
                        .optional("resumeFromBookmark", Schema::Bool)
                        .optional("dir", Schema::path(PathKind::Dir, PathAccess::Write))
                        .optional("maxFileMb", Schema::int(1, 100))
                        .optional("maxFiles", Schema::int(1, 50)),
                )
                .budget(Some(Duration::from_secs(60))))
            .op(OpSpec::new("stop", "Stop following"))
            .op(OpSpec::new("pause", "Pause").budget(quick))
            .op(OpSpec::new("resume", "Resume").budget(quick))
            .op(OpSpec::new("status", "Events, rate, errors").budget(Some(Duration::from_secs(5))))
    }

    fn invoke(&self, op: &str, args: Value, ctx: &ToolCtx) -> Result<Value> {
        match op {
            "channels" => self.channels(),
            "validate" => self.validate(&args),
            "start" => self.start(args, ctx),
            "stop" => Ok(json!(self.stop_run(ctx)?)),
            "pause" => self.set_paused(true),
            "resume" => self.set_paused(false),
            "status" => self.status(),
            other => bail!("unknown operation \"{other}\""),
        }
    }

    fn stop(&self, task: &str, ctx: &ToolCtx) -> Result<()> {
        if task != TASK {
            bail!("the event log follower has no background task \"{task}\"");
        }
        self.stop_run(ctx).map(|_| ())
    }
}

/// An Event Log that is never there (plugin-level tests).
#[cfg(test)]
pub(crate) struct NoEventLog;

#[cfg(test)]
impl EventLog for NoEventLog {
    fn channel_names(&self) -> Result<Vec<String>, debrief_plugin_windows::eventlog::EventLogError> {
        Err(debrief_plugin_windows::eventlog::EventLogError::Unavailable("test".into()))
    }
    fn probe(
        &self,
        _: &debrief_plugin_windows::eventlog::Channel,
        _: &str,
    ) -> Result<Option<u64>, debrief_plugin_windows::eventlog::EventLogError> {
        Err(debrief_plugin_windows::eventlog::EventLogError::Unavailable("test".into()))
    }
    fn query(
        &self,
        _: &debrief_plugin_windows::eventlog::Channel,
        _: &str,
    ) -> Result<Box<dyn debrief_plugin_windows::eventlog::EventStream>, debrief_plugin_windows::eventlog::EventLogError> {
        Err(debrief_plugin_windows::eventlog::EventLogError::Unavailable("test".into()))
    }
    fn subscribe(
        &self,
        _: &debrief_plugin_windows::eventlog::Channel,
        _: &str,
        _: Option<u64>,
    ) -> Result<Box<dyn debrief_plugin_windows::eventlog::EventStream>, debrief_plugin_windows::eventlog::EventLogError> {
        Err(debrief_plugin_windows::eventlog::EventLogError::Unavailable("test".into()))
    }
}

#[cfg(test)]
mod tests;
