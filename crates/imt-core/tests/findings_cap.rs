//! The health check lists a bounded number of anomalies, the highest ranked first, and says how many it left out
//! (a noisy log can hold thousands; listing them all froze the Home dashboard).

use imt_core::analysis::moment::{findings, OverviewInputs, MAX_ANOMALY_FINDINGS};
use imt_core::analysis::{Anomaly, AnomalyKind};
use imt_core::api::{FindingKind, Severity};
use std::time::Instant;

fn anomaly(kind: AnomalyKind, i: i64, score: f64) -> Anomaly {
    Anomaly {
        kind,
        instance: None,
        from: i * 1000,
        to: i * 1000 + 500,
        score,
        message: format!("anomaly {i}"),
        sample_ids: vec![i as u32],
    }
}

fn inputs() -> OverviewInputs<'static> {
    OverviewInputs {
        sources: &[],
        unrecognized: &[],
        quarantined: Vec::new(),
    }
}

#[test]
fn many_anomalies_are_capped_ranked_and_summarised() {
    // 50k rate changes with rising scores, a few crashes and error spikes hidden among them.
    let mut all: Vec<Anomaly> = (0..50_000).map(|i| anomaly(AnomalyKind::RateChange, i, i as f64)).collect();
    all.push(anomaly(AnomalyKind::Crash, 7, 1.0));
    all.push(anomaly(AnomalyKind::ErrorSpike, 9, 2.0));
    let started = Instant::now();
    let out = findings(&all, &[], &inputs());
    let took = started.elapsed();

    // Bounded: the capped anomalies plus the one summary.
    assert_eq!(out.len(), MAX_ANOMALY_FINDINGS + 1);
    // Most severe first: the crash, then the error spike, then the best-scored rate changes.
    assert_eq!(out[0].kind, FindingKind::Crash);
    assert_eq!(out[1].kind, FindingKind::ErrorSpike);
    assert_eq!(out[2].detail, "anomaly 49999");
    // Rate changes (Info) come last, with the summary of what was left out.
    let last = out.last().unwrap();
    assert_eq!(last.kind, FindingKind::NotShown);
    assert_eq!(last.severity, Severity::Info);
    assert!(
        last.detail.starts_with(&format!("{} ", 50_002 - MAX_ANOMALY_FINDINGS)),
        "{}",
        last.detail
    );
    // Selecting the top N is linear: 50k anomalies are ranked in well under a second even in a debug build.
    assert!(took.as_secs() < 5, "ranking took {took:?}");
}

#[test]
fn few_anomalies_are_all_listed_without_a_summary() {
    let all: Vec<Anomaly> = (0..MAX_ANOMALY_FINDINGS as i64)
        .map(|i| anomaly(AnomalyKind::Silence, i, 1.0))
        .collect();
    let out = findings(&all, &[], &inputs());
    assert_eq!(out.len(), MAX_ANOMALY_FINDINGS);
    assert!(out.iter().all(|f| f.kind != FindingKind::NotShown));
    // Equal scores keep time order.
    assert_eq!(out[0].detail, "anomaly 0");
}
