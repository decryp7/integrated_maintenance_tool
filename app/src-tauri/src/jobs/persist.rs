//! The job history as saved with a project or session (UI state key `jobs`): versioned, bounded, and
//! tolerant: a missing, damaged or newer record restores nothing (with a warning), a damaged job is skipped.

use super::model::Job;
use super::queue::HISTORY_CAP;
use serde::Serialize;
use serde_json::Value;

pub const VERSION: u32 = 1;

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct JobsRecord {
    pub version: u32,
    /// Oldest first.
    pub jobs: Vec<Job>,
}

impl JobsRecord {
    pub fn new(mut jobs: Vec<Job>) -> JobsRecord {
        let skip = jobs.len().saturating_sub(HISTORY_CAP);
        jobs.drain(..skip);
        JobsRecord { version: VERSION, jobs }
    }

    /// Parses a saved record. `None` (nothing saved) restores nothing without a warning.
    pub fn from_value(value: Option<&Value>) -> (Vec<Job>, Option<String>) {
        let Some(v) = value.filter(|v| !v.is_null()) else {
            return (Vec::new(), None);
        };
        let version = v.get("version").and_then(Value::as_u64);
        match version {
            Some(n) if n == u64::from(VERSION) => {}
            Some(n) if n > u64::from(VERSION) => {
                return (
                    Vec::new(),
                    Some(format!("the job history was saved by a newer Debrief (version {n})")),
                );
            }
            _ => return (Vec::new(), Some("the saved job history is damaged and was not loaded".into())),
        }
        let Some(list) = v.get("jobs").and_then(Value::as_array) else {
            return (Vec::new(), Some("the saved job history is damaged and was not loaded".into()));
        };
        let jobs: Vec<Job> = list.iter().filter_map(|j| serde_json::from_value(j.clone()).ok()).collect();
        let warning = (jobs.len() < list.len()).then(|| format!("{} damaged jobs were skipped", list.len() - jobs.len()));
        let skip = jobs.len().saturating_sub(HISTORY_CAP);
        (jobs.into_iter().skip(skip).collect(), warning)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::jobs::model::{JobKind, JobSpec};
    use crate::jobs::queue::{JobEnd, JobQueue};
    use serde_json::json;

    fn finished(n: usize) -> Vec<Job> {
        let mut q = JobQueue::new(2);
        for i in 0..n {
            let id = q.submit(JobSpec::new(JobKind::Export, format!("e{i}")).window("main"), 0).unwrap();
            q.start(id, 0).unwrap();
            q.finish(id, JobEnd::Failed("disk full".into()), 5_000).unwrap();
        }
        q.history_of("main")
    }

    #[test]
    fn round_trip() {
        let jobs = finished(3);
        let value = serde_json::to_value(JobsRecord::new(jobs.clone())).unwrap();
        assert_eq!(value["version"], 1);
        assert_eq!(value["jobs"][0]["state"], "failed");
        let (back, warning) = JobsRecord::from_value(Some(&value));
        assert_eq!((back, warning), (jobs, None));
    }

    #[test]
    fn missing_damaged_or_newer_records_restore_nothing() {
        assert_eq!(JobsRecord::from_value(None), (vec![], None));
        assert_eq!(JobsRecord::from_value(Some(&Value::Null)), (vec![], None));
        for bad in [json!("x"), json!({"version": "1"}), json!({"version": 1, "jobs": 3}), json!({})] {
            let (jobs, warning) = JobsRecord::from_value(Some(&bad));
            assert!(jobs.is_empty());
            assert!(warning.unwrap().contains("damaged"), "{bad}");
        }
        let (jobs, warning) = JobsRecord::from_value(Some(&json!({"version": 9, "jobs": []})));
        assert!(jobs.is_empty() && warning.unwrap().contains("newer Debrief"));
    }

    #[test]
    fn damaged_jobs_are_skipped_and_the_record_is_bounded() {
        let mut value = serde_json::to_value(JobsRecord::new(finished(2))).unwrap();
        value["jobs"].as_array_mut().unwrap().push(json!({"id": "nope"}));
        let (jobs, warning) = JobsRecord::from_value(Some(&value));
        assert_eq!(jobs.len(), 2);
        assert_eq!(warning.as_deref(), Some("1 damaged jobs were skipped"));
        let mut many = finished(60);
        many.extend(finished(60));
        assert_eq!(JobsRecord::new(many.clone()).jobs.len(), HISTORY_CAP);
        let raw = json!({"version": 1, "jobs": many});
        assert_eq!(JobsRecord::from_value(Some(&raw)).0.len(), HISTORY_CAP);
    }
}
