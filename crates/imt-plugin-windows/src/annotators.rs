//! Annotators over Windows event logs: downtime windows (reboots) and
//! service uptime spans. Both group by the `Computer` field and produce
//! machine-wide ranges (every instance of that computer).

use crate::markers::{DOWNTIME, EVENTLOG, KERNEL_GENERAL, KERNEL_POWER, SCM, SERVICE_UP};
use anyhow::Result;
use imt_core::api::Severity;
use imt_core::condition::{CondValue, ConditionCfg};
use imt_core::plugin::annotate::{Annotation, AnnotationView, Annotator, FieldMatch, RecordCursor, ViewFilter, ViewRecord};
use imt_core::query::{FieldFilter, Query};
use std::collections::BTreeMap;

/// Most events each annotator reads per kind of event.
const LIMIT: usize = 20_000;
/// Boot events closer than this belong to the same boot (6005 and Kernel-General 12).
const SAME_BOOT_MS: i64 = 5 * 60_000;
/// 6008 / Kernel-Power 41 this soon after a boot mark that boot unexpected.
const AFTER_BOOT_MS: i64 = 15 * 60_000;

fn cond(field: &str, op: &str, value: CondValue) -> ConditionCfg {
    ConditionCfg {
        field: field.into(),
        op: op.into(),
        value: Some(value),
    }
}

fn events(view: &dyn AnnotationView, provider: &str, ids: &[&str]) -> Result<Vec<ViewRecord>> {
    let filter = ViewFilter {
        when: vec![
            cond("Provider", "eq", CondValue::Text(provider.into())),
            cond("EventId", "in", CondValue::List(ids.iter().map(|s| s.to_string()).collect())),
        ],
        ..ViewFilter::default()
    };
    view.select(&filter, LIMIT)
}

fn text(r: &ViewRecord, field: &str) -> String {
    r.record
        .fields
        .get(field)
        .map(|v| imt_core::model::value_text(v).into_owned())
        .unwrap_or_default()
}

fn on_computer(computer: &str) -> Option<FieldMatch> {
    (!computer.is_empty()).then(|| FieldMatch {
        field: "Computer".into(),
        value: computer.into(),
    })
}

fn evidence(from: i64, to: i64, computer: &str) -> Query {
    Query {
        from: Some(from),
        to: Some(to),
        filters: if computer.is_empty() {
            Vec::new()
        } else {
            vec![FieldFilter {
                field: "Computer".into(),
                op: "eq".into(),
                value: Some(computer.into()),
            }]
        },
        ..Query::default()
    }
}

/// Ordered so that at equal times a shutdown sorts before a start.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
enum Boot {
    /// 6006 or Kernel-General 13.
    CleanShutdown,
    /// 6005 or Kernel-General 12.
    Started,
    /// 6008 or Kernel-Power 41 (logged at the next boot).
    WasUnexpected,
}

/// Downtime windows: from a clean shutdown (6006 / Kernel-General 13) to the
/// next boot (6005 / Kernel-General 12), or, when there was no clean
/// shutdown, from the computer's last event before the boot. 6008 and
/// Kernel-Power 41 are logged after the reboot, so they only mark that
/// window as unexpected; the real moment of failure is not known more
/// precisely than "after the last event we have".
pub struct RebootAnnotator;

impl Annotator for RebootAnnotator {
    fn id(&self) -> &str {
        "windows.reboots"
    }

    fn annotate(&self, view: &dyn AnnotationView, _since: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        let mut marks: Vec<(i64, String, Boot)> = Vec::new();
        let mut add = |records: Vec<ViewRecord>, classify: &dyn Fn(&str) -> Boot| {
            for r in records {
                if let Some(ts) = r.record.ts {
                    marks.push((ts, text(&r, "Computer"), classify(&text(&r, "EventId"))));
                }
            }
        };
        add(events(view, EVENTLOG, &["6005", "6006", "6008"])?, &|id| match id {
            "6005" => Boot::Started,
            "6006" => Boot::CleanShutdown,
            _ => Boot::WasUnexpected,
        });
        add(events(view, KERNEL_GENERAL, &["12", "13"])?, &|id| {
            if id == "12" {
                Boot::Started
            } else {
                Boot::CleanShutdown
            }
        });
        add(events(view, KERNEL_POWER, &["41"])?, &|_| Boot::WasUnexpected);
        marks.sort();

        let mut by_computer: BTreeMap<String, Vec<(i64, Boot)>> = BTreeMap::new();
        for (ts, computer, b) in marks {
            by_computer.entry(computer).or_default().push((ts, b));
        }
        let mut out = Vec::new();
        for (computer, marks) in by_computer {
            let mut shutdown: Option<i64> = None;
            let mut last_boot: Option<i64> = None;
            for (ts, b) in marks {
                match b {
                    Boot::CleanShutdown => {
                        shutdown.get_or_insert(ts);
                    }
                    Boot::Started => {
                        if last_boot.is_some_and(|l| ts - l < SAME_BOOT_MS) {
                            continue;
                        }
                        last_boot = Some(ts);
                        let (from, label, severity) = match shutdown.take() {
                            Some(s) => (s, "Reboot", Severity::Info),
                            None => (
                                self.last_event_before(view, &computer, ts)?,
                                "Downtime (no clean shutdown)",
                                Severity::Warning,
                            ),
                        };
                        out.push(Annotation {
                            kind: DOWNTIME.into(),
                            from,
                            to: ts,
                            instance: None,
                            on: on_computer(&computer),
                            label: format!("{label} of {}", if computer.is_empty() { "the computer" } else { &computer }),
                            severity,
                            evidence_query: evidence(from, ts, &computer),
                        });
                    }
                    Boot::WasUnexpected => {
                        let boot = last_boot.filter(|l| ts >= *l && ts - l <= AFTER_BOOT_MS);
                        if let Some(a) = boot.and_then(|l| out.iter_mut().rev().find(|a| a.to == l && a.on == on_computer(&computer))) {
                            if a.severity != Severity::Critical {
                                a.severity = Severity::Critical;
                                a.label = format!(
                                    "Unexpected shutdown of {}",
                                    if computer.is_empty() { "the computer" } else { &computer }
                                );
                            }
                        }
                    }
                }
            }
        }
        Ok(out)
    }
}

impl RebootAnnotator {
    /// Time of the computer's newest event before `ts` (or `ts` if none).
    fn last_event_before(&self, view: &dyn AnnotationView, computer: &str, ts: i64) -> Result<i64> {
        let mut when = Vec::new();
        if !computer.is_empty() {
            when.push(cond("Computer", "eq", CondValue::Text(computer.into())));
        }
        let before = view.select(
            &ViewFilter {
                when,
                to: Some(ts - 1),
                newest_first: true,
                ..ViewFilter::default()
            },
            1,
        )?;
        Ok(before.first().and_then(|r| r.record.ts).unwrap_or(ts))
    }
}

/// Service uptime: from SCM 7036 "entered the running state" to 7036
/// "stopped", or a crash (7031/7034), per computer and service. Open spans
/// end at the newest event. The state in 7036 is localised text: only
/// English logs ("running", "stopped") are recognised.
pub struct ServiceAnnotator;

impl Annotator for ServiceAnnotator {
    fn id(&self) -> &str {
        "windows.services"
    }

    fn annotate(&self, view: &dyn AnnotationView, _since: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        let records = events(view, SCM, &["7036", "7031", "7034"])?;
        let end = view.time_range().map(|(_, to)| to);
        let mut open: BTreeMap<(String, String), i64> = BTreeMap::new();
        let mut out = Vec::new();
        let span = |computer: &str, service: &str, from: i64, to: i64, crashed: bool| Annotation {
            kind: SERVICE_UP.into(),
            from,
            to,
            instance: None,
            on: on_computer(computer),
            label: if crashed {
                format!("{service} running (then crashed)")
            } else {
                format!("{service} running")
            },
            severity: if crashed { Severity::Warning } else { Severity::Info },
            evidence_query: Query {
                text: service.to_string(),
                ..evidence(from, to, computer)
            },
        };
        for r in records {
            let Some(ts) = r.record.ts else { continue };
            let key = (text(&r, "Computer"), text(&r, "param1"));
            if key.1.is_empty() {
                continue;
            }
            match (text(&r, "EventId").as_str(), text(&r, "param2").to_lowercase().as_str()) {
                ("7036", "running") => {
                    open.entry(key).or_insert(ts);
                }
                ("7036", "stopped") => {
                    if let Some(from) = open.remove(&key) {
                        out.push(span(&key.0, &key.1, from, ts, false));
                    }
                }
                ("7031" | "7034", _) => {
                    if let Some(from) = open.remove(&key) {
                        out.push(span(&key.0, &key.1, from, ts, true));
                    }
                }
                _ => {}
            }
        }
        if let Some(end) = end {
            for ((computer, service), from) in open {
                out.push(span(&computer, &service, from, end.max(from), false));
            }
        }
        Ok(out)
    }
}
