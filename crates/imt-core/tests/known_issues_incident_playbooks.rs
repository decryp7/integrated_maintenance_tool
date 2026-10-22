//! Known issues, the incident summary and playbooks against the generated samples (ADR 0039).

use imt_core::incident::{build_incident_report, IncidentOptions};
use imt_core::known_issues::{match_known_issues, KnowledgeText, Origin};
use imt_core::playbook::{run_step, Action, Expect, Playbook, PlaybookRecord, PlaybookRun, Step, StepStatus};
use imt_core::report::{Block, StoredBlock};
use imt_core::workspace::WorkspaceStore;
use imt_core::{Profile, Workspace};
use std::path::PathBuf;
use std::sync::OnceLock;

fn store() -> &'static WorkspaceStore {
    static S: OnceLock<WorkspaceStore> = OnceLock::new();
    S.get_or_init(|| {
        let s = WorkspaceStore::new(Workspace::new(Profile::default_profile()).unwrap());
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples/serilog-clef");
        imt_core::ports::Ingest::open(&s, &[dir]).unwrap().wait().unwrap();
        s
    })
}

const KNOWLEDGE: &str = "# Orders\n\n```known-issue\ntitle: Payment gateway timeouts\npattern: (?i)request timed out\nfix: Raise the gateway timeout to 30 s and check the provider status page.\n```\n\n```known-issue\ntitle: Never happens\npattern: zzz-no-such-text-zzz\nfix: n/a\n```\n\n```known-issue\ntitle: Bad field\nquery: nosuchfield = 1\nfix: n/a\n```\n\n```known-issue\ntitle: Broken\nfix: n/a\n```\n";

#[test]
fn declared_known_issues_match_with_evidence_and_bad_ones_warn() {
    let r = match_known_issues(
        store(),
        &[KnowledgeText {
            name: "orders.md",
            text: KNOWLEDGE,
        }],
    )
    .unwrap();
    assert_eq!(r.checked, 3);
    assert_eq!(r.matches.len(), 1);
    let m = &r.matches[0];
    assert_eq!(m.issue.title, "Payment gateway timeouts");
    assert_eq!(m.origin, Origin::Rule);
    assert!(m.count > 100, "{}", m.count);
    assert!(!m.sample_ids.is_empty() && m.sample_ids.len() <= 5);
    assert!(m.query.starts_with("@message ~ "));
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("Bad field") && w.contains("does not work here")),
        "{:?}",
        r.warnings
    );
    assert!(
        r.warnings
            .iter()
            .any(|w| w.contains("known issue 4") && w.contains("needs a `pattern:`")),
        "{:?}",
        r.warnings
    );
}

#[test]
fn the_number_of_checked_known_issues_is_bounded() {
    let one = "```known-issue\ntitle: T\npattern: x\nfix: y\n```\n";
    let files: Vec<String> = (0..3).map(|_| one.repeat(100)).collect();
    let texts: Vec<KnowledgeText<'_>> = files
        .iter()
        .enumerate()
        .map(|(i, t)| KnowledgeText {
            name: ["a", "b", "c"][i],
            text: t,
        })
        .collect();
    let r = match_known_issues(store(), &texts).unwrap();
    assert_eq!(r.checked, 200);
    assert_eq!(r.truncated, 100);
}

fn blocks(r: &imt_core::report::Report) -> Vec<(&str, &Block)> {
    r.sections
        .iter()
        .flat_map(|s| {
            s.blocks.iter().filter_map(move |b| match b {
                StoredBlock::Known(i) => Some((s.title.as_str(), &i.block)),
                StoredBlock::Unknown(_) => None,
            })
        })
        .collect()
}

#[test]
fn the_incident_summary_is_computed_by_the_engine_with_evidence() {
    let issues = match_known_issues(
        store(),
        &[KnowledgeText {
            name: "orders.md",
            text: KNOWLEDGE,
        }],
    )
    .unwrap()
    .matches;
    let opts = IncidentOptions {
        id: "incident-1".into(),
        notes: "Customer reported failures at 09:35.".into(),
        issues,
        now: 1_700_000_000_000,
        ..Default::default()
    };
    let r = build_incident_report(store(), &opts).unwrap();
    assert_eq!(r.title, "Incident summary");
    let titles: Vec<&str> = r.sections.iter().map(|s| s.title.as_str()).collect();
    for want in [
        "Summary",
        "Top exceptions",
        "Top problems",
        "Affected sources and hosts",
        "Known issues",
        "Notes",
    ] {
        assert!(titles.contains(&want), "{want} missing in {titles:?}");
    }
    let all = blocks(&r);
    // Facts come with evidence: every events block lists real event ids, and tables have rows.
    assert!(all
        .iter()
        .any(|(s, b)| *s == "Top exceptions" && matches!(b, Block::Table { rows, .. } if !rows.is_empty())));
    assert!(all
        .iter()
        .any(|(s, b)| *s == "Known issues" && matches!(b, Block::Events { ids, .. } if !ids.is_empty())));
    assert!(all
        .iter()
        .any(|(_, b)| matches!(b, Block::Text { markdown } if markdown.contains("**Events:**"))));
    // The host ranking uses the MachineName field of the samples.
    assert!(all
        .iter()
        .any(|(_, b)| matches!(b, Block::Table { caption, .. } if caption.contains("MachineName"))));
    // The report passes the same validation as any report from the UI.
    imt_core::report::Report::parse(serde_json::to_value(&r).unwrap()).unwrap();
    // Block ids are unique.
    let mut ids: Vec<&str> = r
        .sections
        .iter()
        .flat_map(|s| s.blocks.iter())
        .filter_map(|b| match b {
            StoredBlock::Known(i) => Some(i.id.as_str()),
            _ => None,
        })
        .collect();
    let n = ids.len();
    ids.sort_unstable();
    ids.dedup();
    assert_eq!(ids.len(), n);
}

#[test]
fn the_incident_summary_needs_an_id_respects_the_top_limit_and_the_period() {
    assert!(build_incident_report(store(), &IncidentOptions::default()).is_err());
    let r = build_incident_report(
        store(),
        &IncidentOptions {
            id: "i".into(),
            top: Some(1),
            title: "  Outage ".into(),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(r.title, "Outage");
    for (_, b) in blocks(&r) {
        if let Block::Table { rows, caption, .. } = b {
            if caption.starts_with("Most frequent") || caption.contains("with most errors") {
                assert!(rows.len() <= 1, "{caption}");
            }
        }
    }
    // A period with no events leaves out the analyses.
    let empty = build_incident_report(
        store(),
        &IncidentOptions {
            id: "i".into(),
            from: Some(1),
            to: Some(2),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(empty.sections.iter().all(|s| s.title != "Top exceptions"));
    assert_eq!(empty.sections[0].title, "Summary");
}

fn step(id: &str, action: Action) -> Step {
    Step {
        id: id.into(),
        title: id.into(),
        instruction: String::new(),
        action,
    }
}

#[test]
fn playbook_steps_run_against_the_data_and_the_run_is_recorded() {
    let s = store();
    let r = run_step(
        s,
        &step(
            "q",
            Action::Query {
                ql: "level >= error | stats count() as n by MachineName".into(),
            },
        ),
    )
    .unwrap();
    assert_eq!((r.status, r.columns.len()), (StepStatus::Info, 2));
    assert!(!r.rows.is_empty() && r.rows.len() <= 20);
    let r = run_step(
        s,
        &step(
            "e",
            Action::Query {
                ql: "level >= error".into(),
            },
        ),
    )
    .unwrap();
    assert!(r.count.unwrap() > 0 && r.sample_ids.len() <= 5 && r.summary.starts_with("Found "));
    let r = run_step(
        s,
        &step(
            "f",
            Action::Filter {
                ql: "level = fatal".into(),
            },
        ),
    )
    .unwrap();
    assert!(r.summary.starts_with("The filter matches"));
    let check = |ql: &str, expect| run_step(s, &step("c", Action::Check { ql: ql.into(), expect })).unwrap();
    assert_eq!(check("level = fatal", Expect::NoMatch).status, StepStatus::Fail);
    assert_eq!(check("message ~ \"zzz-none\"", Expect::NoMatch).status, StepStatus::Pass);
    assert_eq!(check("level >= error", Expect::AnyMatch).status, StepStatus::Pass);
    assert_eq!(check("level >= error", Expect::AtLeast { n: 1_000_000 }).status, StepStatus::Fail);
    assert_eq!(check("level >= error", Expect::AtMost { n: 1_000_000 }).status, StepStatus::Pass);
    let bad = run_step(
        s,
        &step(
            "b",
            Action::Check {
                ql: "nosuchfield = 1".into(),
                expect: Expect::NoMatch,
            },
        ),
    )
    .unwrap();
    assert_eq!(bad.status, StepStatus::Error);
    let bad = run_step(s, &step("b", Action::Query { ql: "level >=".into() })).unwrap();
    assert_eq!(bad.status, StepStatus::Error);
    assert!(run_step(s, &step("m", Action::Manual)).unwrap().summary.contains("by hand"));
    assert!(run_step(s, &step("v", Action::OpenView { view: "problems".into() }))
        .unwrap()
        .summary
        .contains("problems"));
}

#[test]
fn every_template_runs_without_an_error_step_and_runs_are_recorded_in_the_project() {
    let s = store();
    let t = imt_core::playbook::templates();
    for p in &t {
        for st in &p.steps {
            let r = run_step(s, st).unwrap();
            assert_ne!(r.status, StepStatus::Error, "{} / {}: {}", p.id, st.id, r.summary);
        }
    }
    let pb: &Playbook = &t[0];
    let mut run = PlaybookRun::start("run-1", pb.clone(), 10).unwrap();
    let res = run_step(s, &pb.steps[0]).unwrap();
    run.record(&pb.steps[0].id, &res, "looked", 11).unwrap();
    run.record(&pb.steps[0].id, &res, "looked again", 12).unwrap();
    assert_eq!(run.progress(), (1, pb.steps.len()));
    assert_eq!(run.steps[0].note, "looked again");
    assert!(run.record("nope", &res, "", 13).is_err());
    assert!(run.record(&pb.steps[0].id, &res, &"x".repeat(5000), 13).is_err());
    assert!(PlaybookRun::start(" ", pb.clone(), 1).is_err());

    let mut rec = PlaybookRecord::new();
    rec.put(run.clone());
    rec.put(run.clone());
    assert_eq!(rec.runs.len(), 1);
    let (back, warnings) = PlaybookRecord::restore(serde_json::to_value(&rec).unwrap());
    assert_eq!(back, rec);
    assert!(warnings.is_empty());
    assert!(rec.remove("run-1") && !rec.remove("run-1"));
}

#[test]
fn damaged_or_newer_project_records_restore_what_they_can() {
    let (r, w) = PlaybookRecord::restore(serde_json::json!({"version": 9, "runs": []}));
    assert!(r.runs.is_empty() && w[0].contains("newer version"));
    let (r, w) = PlaybookRecord::restore(serde_json::json!("garbage"));
    assert!(r.runs.is_empty() && w[0].contains("damaged"));
    let good = serde_json::to_value(PlaybookRun::start("r", imt_core::playbook::templates()[0].clone(), 1).unwrap()).unwrap();
    let (r, w) = PlaybookRecord::restore(serde_json::json!({"version": 1, "runs": [good, {"id": "x"}]}));
    assert_eq!(r.runs.len(), 1);
    assert!(w[0].starts_with("playbook run 2 was skipped"));
    let many: Vec<_> = (0..60).map(|_| good.clone()).collect();
    let (r, w) = PlaybookRecord::restore(serde_json::json!({"version": 1, "runs": many}));
    assert_eq!(r.runs.len(), 50.min(r.runs.len()));
    assert!(w.iter().any(|m| m.contains("only the first 50")));
    let mut rec = PlaybookRecord::new();
    for i in 0..55 {
        let mut run = PlaybookRun::start(&format!("r{i}"), imt_core::playbook::templates()[0].clone(), 1).unwrap();
        run.started_at = i;
        rec.put(run);
    }
    assert_eq!(rec.runs.len(), 50);
    assert_eq!(rec.runs[0].id, "r5");
}

#[test]
fn a_filter_step_reports_a_bad_query_and_counts_the_rows_of_a_table() {
    let s = store();
    let bad = run_step(s, &step("f", Action::Filter { ql: "level >=".into() })).unwrap();
    assert_eq!(bad.status, StepStatus::Error);
    assert!(bad.summary.starts_with("The filter does not work here: "), "{}", bad.summary);
    // A query that ends in a table has no events to cite, only rows to count.
    let table = run_step(
        s,
        &step(
            "t",
            Action::Filter {
                ql: "level >= error | stats count() as n by MachineName".into(),
            },
        ),
    )
    .unwrap();
    assert_eq!(table.status, StepStatus::Info);
    assert!(table.count.unwrap() >= 1 && table.sample_ids.is_empty(), "{table:?}");
}
