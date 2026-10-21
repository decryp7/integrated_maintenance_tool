//! "Set up analysis for this folder" end to end on /samples: survey, heuristic proposals, validation and
//! composing the selected proposals into a profile that the workspace accepts.

use imt_core::survey::{self, Fragment, KeyOrigin, LifecycleRole, ProposalGroup, ProposalPreview, SurveyControl, SurveyRequest};
use imt_core::workspace::WorkspaceStore;
use imt_core::{Profile, Workspace, DEFAULT_PROFILE_TOML};
use std::path::PathBuf;

fn samples() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../samples")
}

/// The log samples (profiler captures are not logs; the profile file is not a log either).
fn workspace() -> WorkspaceStore {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(samples())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n != "profiling" && n != "profiles" && n != "generate.py")
        })
        .collect();
    paths.sort();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(&paths).unwrap();
    WorkspaceStore::new(ws)
}

fn setup(ws: &WorkspaceStore) -> survey::FolderSetup {
    survey::run(ws, DEFAULT_PROFILE_TOML, &SurveyRequest::default(), &SurveyControl::new()).unwrap()
}

#[test]
fn survey_of_the_samples_finds_groups_fields_and_lifecycle() {
    let ws = workspace();
    let s = setup(&ws).survey;
    assert!(s.groups.len() >= 5, "{:?}", s.groups.iter().map(|g| &g.id).collect::<Vec<_>>());
    assert!(s.sampled_events > 1000 && s.sampled_events <= 20_000);
    assert!(s.groups.iter().all(|g| g.sampled <= 2_000));
    assert!(s.fields.iter().any(|f| f.name == "MachineName"));
    // The best instance key is the machine: a handful of lanes, on most events.
    let best = &s.instance_candidates[0];
    assert_eq!(best.field, "MachineName", "{:?}", s.instance_candidates);
    assert_eq!(best.origin, KeyOrigin::Field);
    assert!((2..=16).contains(&best.lanes));
    // Starts and stops of the services are found and already covered by the built-in markers.
    let starts: Vec<_> = s.lifecycle.iter().filter(|c| c.role == LifecycleRole::Start).collect();
    let stops: Vec<_> = s.lifecycle.iter().filter(|c| c.role == LifecycleRole::Stop).collect();
    assert!(!starts.is_empty() && !stops.is_empty(), "{:?}", s.lifecycle);
    assert!(starts.iter().any(|c| c.covered_by.as_deref() == Some("start")), "{starts:?}");
    assert!(stops.iter().any(|c| c.covered_by.as_deref() == Some("stop")), "{stops:?}");
    // Errors and embedded numbers ("CPU 23.3%", "WorkingSet 615 MB").
    assert!(!s.errors.is_empty());
    assert!(s.numbers.iter().any(|n| n.name == "CpuPercent"), "{:?}", s.numbers);
    assert!(s.numbers.iter().any(|n| n.name == "WorkingSetMb"), "{:?}", s.numbers);
    // The unsupported files are listed with their first lines.
    assert!(s.unrecognised.iter().any(|g| g.extension == "reg" && !g.sample_lines.is_empty()));
    assert!(s.markers.iter().any(|m| m.kind == "start" && m.matches > 0));
}

#[test]
fn heuristic_proposals_on_the_samples_are_valid_and_previewed() {
    let ws = workspace();
    let setup = setup(&ws);
    let by_id = |id: &str| setup.proposals.iter().find(|p| p.id == id);
    let key = by_id("instance:MachineName").expect("an instance-key proposal");
    assert!(key.recommended);
    match &key.preview {
        ProposalPreview::Lanes { total_lanes, lanes, .. } => {
            assert!((2..=16).contains(total_lanes));
            assert!(lanes.iter().any(|l| l.name == "WEB01"));
        }
        other => panic!("{other:?}"),
    }
    let run = by_id("span:run").expect("a run rule");
    match &run.preview {
        ProposalPreview::Runs { runs, .. } => assert!(*runs >= 2),
        other => panic!("{other:?}"),
    }
    assert!(setup
        .proposals
        .iter()
        .any(|p| p.group == ProposalGroup::Extractors && p.id == "extract:CpuPercent"));
    assert!(setup.proposals.iter().any(|p| p.group == ProposalGroup::Charts));
    assert!(setup.proposals.iter().any(|p| p.group == ProposalGroup::Queries));
    // Every proposal carries its TOML and passes on its own; the unreadable formats are skipped with a reason.
    assert!(setup.proposals.iter().all(|p| !p.toml.is_empty() && !p.summary.is_empty()));
    assert!(setup.skipped.iter().all(|s| !s.reason.is_empty()));
    assert!(
        setup.skipped.iter().any(|s| s.group == ProposalGroup::Formats),
        "{:?}",
        setup.skipped
    );
}

#[test]
fn composing_the_recommended_proposals_gives_a_profile_the_workspace_applies() {
    let store = workspace();
    let setup = setup(&store);
    let fragments: Vec<Fragment> = setup
        .proposals
        .iter()
        .filter(|p| p.recommended)
        .flat_map(|p| p.fragments.iter().cloned())
        .collect();
    let composed = survey::compose(&store, DEFAULT_PROFILE_TOML, &fragments).unwrap();
    let mut ws = store.write();
    assert!(composed.added > 0);
    assert!(composed.toml.contains("key = [\"MachineName\"]"), "{}", composed.toml);
    assert!(composed.toml.contains("[[span]]") && composed.toml.contains("svc-start"));
    // The built-in markers are kept although the default profile text is the base.
    assert!(composed.toml.contains("kind = \"fatal\""));
    let before_lanes = ws.summary().default_group_by.clone();
    ws.set_profile(Profile::from_toml(&composed.toml).unwrap()).unwrap();
    let after = ws.summary();
    assert_ne!(before_lanes, after.default_group_by);
    assert_eq!(after.default_group_by, vec!["MachineName".to_string()]);
    assert!(after.fields.iter().any(|f| f.name == "CpuPercent" && f.numeric));
}

#[test]
fn survey_without_files_says_so() {
    let ws = WorkspaceStore::new(Workspace::new(Profile::default_profile()).unwrap());
    let err = survey::run(&ws, DEFAULT_PROFILE_TOML, &SurveyRequest::default(), &SurveyControl::new()).unwrap_err();
    assert_eq!(err, survey::SurveyError::NothingOpened);
}

#[test]
fn a_cancelled_survey_stops() {
    let ws = workspace();
    let control = SurveyControl::new();
    control.cancel();
    let err = survey::run(&ws, DEFAULT_PROFILE_TOML, &SurveyRequest::default(), &control).unwrap_err();
    assert_eq!(err, survey::SurveyError::Cancelled);
}

#[test]
fn limits_bound_the_sample() {
    let ws = workspace();
    let req = SurveyRequest {
        max_events_per_group: Some(50),
        max_events: Some(120),
        max_millis: None,
    };
    let s = survey::run(&ws, DEFAULT_PROFILE_TOML, &req, &SurveyControl::new()).unwrap().survey;
    assert!(s.sampled_events <= 120, "{}", s.sampled_events);
    assert!(s.groups.iter().all(|g| g.sampled <= 50));
    assert!(s.truncated);
}
