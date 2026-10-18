//! Pairs start and stop events into runs, per group, in time order. Pure:
//! fed one event at a time (so live tail only feeds the new records), read
//! without consuming (so open runs stay open for the next batch).
//!
//! Rules, per group (a lane, or one field value within a lane):
//! - a start opens a run; a stop closes the open run (`Closed`);
//! - a start while running ends the previous run at the group's last event
//!   before it, as `NoStop` (a crash or restart), and opens a new one — unless
//!   `restart_on_start` is off, then the extra start is ignored;
//! - a stop while not running is a `NoStart` point;
//! - consecutive starts (nothing else logged in between in the group) count as
//!   one ("starting…", "started"); likewise consecutive stops extend the run
//!   they closed ("stopping…", "stopped");
//! - a run still open is `Open`, drawn to the group's last event.
//!
//! Because a run resets at each start, a process id reused in a later period
//! gives a separate run. Bounded: at most [`MAX_RUNS_PER_LANE`] runs per lane
//! (the oldest are dropped first) and [`MAX_GROUPS`] groups.

use serde::Serialize;
use std::collections::{HashMap, VecDeque};

/// Most runs kept per lane and rule (the oldest are dropped first).
pub const MAX_RUNS_PER_LANE: usize = 1_000;
/// Most groups (lanes, or lane + field value) one rule tracks; events of further groups are ignored.
pub const MAX_GROUPS: usize = 10_000;
/// Characters of a start/stop message kept.
pub const MESSAGE_CHARS: usize = 240;

/// What an event means for a rule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Start,
    Stop,
    Other,
}

/// How a run ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, serde::Deserialize, PartialOrd, Ord)]
#[serde(rename_all = "camelCase")]
pub enum RunStatus {
    /// Started and stopped.
    Closed,
    /// Started and still running at the end of the data.
    Open,
    /// Started, then started again without a stop (likely a crash or restart).
    NoStop,
    /// A stop without a start (a point).
    NoStart,
}

/// The start or stop event of a run.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RunEvent {
    pub id: u32,
    pub ts: i64,
    pub message: String,
}

/// One run of a rule.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Run {
    pub lane: String,
    /// The field value it was paired by (`per = "field:<Name>"`).
    pub group: Option<String>,
    pub start: Option<RunEvent>,
    pub stop: Option<RunEvent>,
    pub from: i64,
    pub to: i64,
    pub status: RunStatus,
}

#[derive(Debug, Clone)]
struct Group {
    open: Option<RunEvent>,
    last: Edge,
    last_ts: i64,
    /// Sequence number of the run the last stop closed (to extend on a repeated stop).
    closed: Option<u64>,
}

/// The pairing state of one rule.
#[derive(Debug, Clone, Default)]
pub struct Pairer {
    restart_on_start: bool,
    /// By lane, then by field value ("" without one).
    groups: HashMap<String, HashMap<String, Group>>,
    group_count: usize,
    runs: HashMap<String, VecDeque<(u64, Run)>>,
    /// Lanes in first-seen order (stable output).
    lane_order: Vec<String>,
    seq: u64,
    /// Runs dropped by [`MAX_RUNS_PER_LANE`].
    pub dropped_runs: usize,
    /// Events ignored because [`MAX_GROUPS`] was reached.
    pub ignored_events: usize,
}

impl Pairer {
    pub fn new(restart_on_start: bool) -> Pairer {
        Pairer {
            restart_on_start,
            ..Default::default()
        }
    }

    /// Feeds one timed event, in time order. `message` is read only for starts and stops.
    pub fn feed(&mut self, lane: &str, value: Option<&str>, edge: Edge, id: u32, ts: i64, message: impl FnOnce() -> String) {
        let value = value.unwrap_or("");
        if !self.groups.get(lane).is_some_and(|g| g.contains_key(value)) {
            if self.group_count >= MAX_GROUPS {
                self.ignored_events += 1;
                return;
            }
            if edge == Edge::Other {
                // A group is only tracked from its first start or stop.
                return;
            }
            self.group_count += 1;
            if !self.groups.contains_key(lane) {
                self.lane_order.push(lane.to_string());
            }
            self.groups.entry(lane.to_string()).or_default().insert(
                value.to_string(),
                Group {
                    open: None,
                    last: Edge::Other,
                    last_ts: ts,
                    closed: None,
                },
            );
        }
        let restart = self.restart_on_start;
        let Some(g) = self.groups.get_mut(lane).and_then(|m| m.get_mut(value)) else {
            return;
        };
        let event = |message: String| RunEvent {
            id,
            ts,
            message: message.chars().take(MESSAGE_CHARS).collect(),
        };
        let group = (!value.is_empty()).then(|| value.to_string());
        let mut done: Option<Run> = None;
        let mut extend: Option<(u64, RunEvent)> = None;
        let mut closed_now = false;
        match (edge, g.open.is_some()) {
            (Edge::Other, _) => {}
            // Consecutive starts are one start; without restarts a start while running is ignored.
            (Edge::Start, true) if g.last == Edge::Start || !restart => {}
            (Edge::Start, true) => {
                let start = g.open.take();
                let from = start.as_ref().map_or(g.last_ts, |s| s.ts);
                done = Some(Run {
                    lane: lane.to_string(),
                    group,
                    start,
                    stop: None,
                    from,
                    to: g.last_ts.max(from),
                    status: RunStatus::NoStop,
                });
                g.open = Some(event(message()));
            }
            (Edge::Start, false) => g.open = Some(event(message())),
            (Edge::Stop, true) => {
                let start = g.open.take();
                let from = start.as_ref().map_or(ts, |s| s.ts);
                done = Some(Run {
                    lane: lane.to_string(),
                    group,
                    start,
                    stop: Some(event(message())),
                    from,
                    to: ts,
                    status: RunStatus::Closed,
                });
                closed_now = true;
            }
            (Edge::Stop, false) => match g.closed.filter(|_| g.last == Edge::Stop) {
                Some(seq) => extend = Some((seq, event(message()))),
                None => {
                    done = Some(Run {
                        lane: lane.to_string(),
                        group,
                        start: None,
                        stop: Some(event(message())),
                        from: ts,
                        to: ts,
                        status: RunStatus::NoStart,
                    })
                }
            },
        }
        g.last = edge;
        g.last_ts = g.last_ts.max(ts);
        if edge != Edge::Stop || !closed_now && extend.is_none() {
            g.closed = None;
        }
        if let Some(run) = done {
            self.seq += 1;
            if closed_now {
                if let Some(g) = self.groups.get_mut(lane).and_then(|m| m.get_mut(value)) {
                    g.closed = Some(self.seq);
                }
            }
            let list = self.runs.entry(lane.to_string()).or_default();
            list.push_back((self.seq, run));
            if list.len() > MAX_RUNS_PER_LANE {
                list.pop_front();
                self.dropped_runs += 1;
            }
        }
        if let Some((seq, stop)) = extend {
            if let Some((_, run)) = self.runs.get_mut(lane).and_then(|l| l.iter_mut().rev().find(|(s, _)| *s == seq)) {
                run.to = run.to.max(stop.ts);
                run.stop = Some(stop);
            }
        }
    }

    /// Every run so far, open ones included, by lane (first seen) and start time.
    pub fn runs(&self) -> Vec<Run> {
        let mut out = Vec::new();
        for lane in &self.lane_order {
            let mut lane_runs: Vec<Run> = self.runs.get(lane).into_iter().flatten().map(|(_, r)| r.clone()).collect();
            if let Some(groups) = self.groups.get(lane) {
                let mut open: Vec<(&String, &Group)> = groups.iter().filter(|(_, g)| g.open.is_some()).collect();
                open.sort_by(|a, b| a.0.cmp(b.0));
                for (value, g) in open {
                    let Some(start) = g.open.clone() else { continue };
                    lane_runs.push(Run {
                        lane: lane.clone(),
                        group: (!value.is_empty()).then(|| value.clone()),
                        from: start.ts,
                        to: g.last_ts.max(start.ts),
                        start: Some(start),
                        stop: None,
                        status: RunStatus::Open,
                    });
                }
            }
            lane_runs.sort_by(|a, b| a.from.cmp(&b.from).then_with(|| a.group.cmp(&b.group)));
            out.extend(lane_runs);
        }
        out
    }
}
