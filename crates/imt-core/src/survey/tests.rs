use super::*;
use crate::condition::{CondValue, ConditionCfg};
use crate::model::{Level, Record};
use crate::pipeline::StepCfg;
use crate::profile::{ExtractCfg, MarkerCfg};
use crate::{Profile, Workspace, DEFAULT_PROFILE_TOML};
use serde_json::json;
use std::collections::BTreeMap;

fn ev(i: usize, ts: i64, level: Level, msg: &str, fields: serde_json::Value) -> Sample {
    let mut rec = Record::new(i as u32 + 1);
    rec.id = i as u32;
    rec.ts = Some(ts);
    rec.level = level;
    rec.message = msg.into();
    if let serde_json::Value::Object(m) = fields {
        rec.fields = m;
    }
    Sample {
        rec,
        source_name: "app.log".into(),
        markers: vec![],
        lane: "app.log".into(),
        group: 0,
    }
}

fn info(i: usize, msg: &str, fields: serde_json::Value) -> Sample {
    ev(i, 1_000 * i as i64, Level::Information, msg, fields)
}

// ---- instance key ----

#[test]
fn the_machine_beats_categories_ids_and_single_values() {
    let mut events = Vec::new();
    for i in 0..200 {
        events.push(info(
            i,
            "Handled",
            json!({
                "MachineName": format!("WEB0{}", i % 3),
                "SourceContext": format!("Ctx{}", i % 5),
                "RequestId": format!("r{i}"),
                "Environment": "Production",
                "ProcessId": 1000 + (i % 6),
            }),
        ));
    }
    let fields = field_summaries(&events);
    let c = instance_candidates(&events, &fields, &[]);
    assert_eq!(c[0].field, "MachineName");
    assert_eq!(c[0].lanes, 3);
    // A category (SourceContext), unique ids and a constant are never keys.
    assert!(c
        .iter()
        .all(|c| !["SourceContext", "RequestId", "Environment"].contains(&c.field.as_str())));
    // A process id is a key candidate (by its name), below the machine.
    assert!(c.iter().any(|c| c.field == "ProcessId"));
}

#[test]
fn too_many_lanes_or_too_little_coverage_is_no_key() {
    let events: Vec<Sample> = (0..300)
        .map(|i| {
            info(
                i,
                "x",
                if i < 50 {
                    json!({"Host": format!("h{}", i % 2)})
                } else {
                    json!({"Node": format!("n{i}")})
                },
            )
        })
        .collect();
    let fields = field_summaries(&events);
    assert!(instance_candidates(&events, &fields, &[]).is_empty());
}

#[test]
fn an_instance_named_in_the_messages_is_extracted() {
    let events: Vec<Sample> = (0..100)
        .map(|i| info(i, &format!("worker-{} picked up job {i}", i % 4), json!({})))
        .collect();
    let fields = field_summaries(&events);
    let c = instance_candidates(&events, &fields, &[]);
    let w = c.iter().find(|c| c.field == "Worker").expect("a worker key");
    assert_eq!(w.lanes, 4);
    let KeyOrigin::Message { regex } = &w.origin else {
        panic!("{:?}", w.origin)
    };
    let re = regex::Regex::new(regex).unwrap();
    assert_eq!(&re.captures("worker-3 picked up").unwrap()["Worker"], "3");
}

#[test]
fn field_kinds_and_ranges() {
    let events = vec![
        info(0, "a", json!({"n": 1, "b": true, "t": "x", "m": 1})),
        info(1, "b", json!({"n": 5, "b": false, "t": "y", "m": "z"})),
    ];
    let f = field_summaries(&events);
    let get = |n: &str| f.iter().find(|x| x.name == n).unwrap();
    assert_eq!(get("n").kind, FieldKind::Number);
    assert_eq!((get("n").min, get("n").max), (Some(1.0), Some(5.0)));
    assert_eq!(get("b").kind, FieldKind::Bool);
    assert_eq!(get("t").kind, FieldKind::Text);
    assert_eq!(get("m").kind, FieldKind::Mixed);
    assert_eq!(get("t").coverage, 1.0);
}

// ---- lifecycle ----

#[test]
fn start_stop_and_crash_phrases() {
    use LifecycleRole::*;
    for (t, role) in [
        ("Application started. Press Ctrl+C to shut down.", Some(Start)),
        ("Now listening on: {address}", Some(Start)),
        ("Billing service starting (v2.8.1)", Some(Start)),
        ("Application is shutting down...", Some(Stop)),
        ("Service stopped", Some(Stop)),
        ("Worker exited with code 0", Some(Stop)),
        ("Host terminated unexpectedly", Some(Crash)),
        ("panic: runtime error: invalid memory address", Some(Crash)),
        ("Unhandled exception in worker", Some(Crash)),
        ("Request starting HTTP/1.1 GET /", None),
        ("Job 17 started", None),
        ("Dispatched 16 outbox messages", None),
    ] {
        assert_eq!(classify_lifecycle(t), role, "{t}");
    }
}

#[test]
fn lifecycle_candidates_are_rare_and_report_coverage() {
    let mut events: Vec<Sample> = (0..400).map(|i| info(i, &format!("Processed item {i}"), json!({}))).collect();
    let mut start = info(400, "Service started", json!({}));
    start.markers = vec!["start".into()];
    events.push(start);
    events.push(info(401, "Service stopped", json!({})));
    // A frequent "started" message is an operation, not a lifecycle event.
    events.extend((0..200).map(|i| info(402 + i, "Timer tick started", json!({}))));
    let mined = templates::mine(&events);
    let l = templates::lifecycle(&mined, &events);
    assert_eq!(l.len(), 2, "{l:?}");
    let s = l.iter().find(|c| c.role == LifecycleRole::Start).unwrap();
    assert_eq!(s.covered_by.as_deref(), Some("start"));
    assert_eq!(l.iter().find(|c| c.role == LifecycleRole::Stop).unwrap().covered_by, None);
}

#[test]
fn a_failed_start_logged_as_error_is_not_a_start() {
    let events = vec![ev(0, 0, Level::Error, "Service failed to start", json!({}))];
    let mined = templates::mine(&events);
    assert!(templates::lifecycle(&mined, &events).is_empty());
    assert_eq!(templates::errors(&mined, &events).len(), 1);
}

// ---- noise ----

#[test]
fn health_checks_and_constant_periodic_messages_are_noise() {
    assert!(matches!(noise_reason("GET /health responded 200", &[]), Some(NoiseReason::Keyword { word }) if word == "health"));
    assert!(matches!(noise_reason("heartbeat from <*>", &[]), Some(NoiseReason::Keyword { .. })));
    // "healthy" is not "health" as a word; and a varying message is not periodic noise.
    assert_eq!(noise_reason("Cluster is healthy now", &[]), None);
    let periodic: Vec<i64> = (0..50).map(|i| i * 30_000).collect();
    assert_eq!(
        noise_reason("Cache refreshed", &periodic),
        Some(NoiseReason::Periodic { interval_ms: 30_000 })
    );
    assert_eq!(noise_reason("Cache refreshed <*> keys", &periodic), None);
    let irregular: Vec<i64> = (0..50).map(|i| i * i * 1000).collect();
    assert_eq!(noise_reason("Cache refreshed", &irregular), None);
}

#[test]
fn noise_candidates_need_volume_and_low_levels() {
    let mut events: Vec<Sample> = (0..100).map(|i| info(i, "Health check OK", json!({}))).collect();
    events.extend((0..100).map(|i| info(100 + i, &format!("Order {i} placed for customer {}", i * 7), json!({}))));
    events.extend((0..30).map(|i| ev(200 + i, i as i64, Level::Error, "Heartbeat missed", json!({}))));
    let mined = templates::mine(&events);
    let n = templates::noise(&mined, &events);
    assert_eq!(n.len(), 1, "{n:?}");
    assert_eq!(n[0].template, "Health check OK");
    assert_eq!(n[0].count, 100);
}

// ---- embedded numbers ----

#[test]
fn numbers_in_messages_become_typed_candidates() {
    let events: Vec<Sample> = (0..20)
        .map(|i| {
            info(
                i,
                &format!(
                    "Process stats: CPU {}.5% · WorkingSet {} MB · Threads {} · queue={} · took {} ms",
                    20 + i,
                    600 + i,
                    60 + i % 3,
                    i % 7,
                    10 + i
                ),
                json!({}),
            )
        })
        .collect();
    let n = number_candidates(&events);
    let get = |name: &str| n.iter().find(|c| c.name == name).unwrap_or_else(|| panic!("{name}: {n:?}"));
    assert_eq!(get("CpuPercent").number_kind, NumberKind::Number);
    assert_eq!(get("CpuPercent").min, 20.5);
    assert_eq!(get("WorkingSetMb").number_kind, NumberKind::Bytes);
    assert_eq!(get("Threads").unit, None);
    assert_eq!(get("Queue").label, "queue");
    let elapsed = get("ElapsedMs");
    assert_eq!(elapsed.number_kind, NumberKind::Duration);
    let re = regex::Regex::new(&elapsed.regex).unwrap();
    assert_eq!(&re.captures("took 12 ms").unwrap()["ElapsedMs"], "12");
}

#[test]
fn ids_codes_constants_positions_and_properties_are_not_measurements() {
    let events: Vec<Sample> = (0..20)
        .map(|i| {
            info(
                i,
                &format!(
                    "Order {} saved · Error: 18456 · port 8080 · at main.go:{} · from 10.20.1.{} · Retries {} · Elapsed {} ms",
                    1000 + i,
                    200 + i,
                    i,
                    3,
                    i
                ),
                json!({"ElapsedMs": i}),
            )
        })
        .collect();
    let n = number_candidates(&events);
    assert!(n.is_empty(), "{n:?}");
    // Numbers of templated events are properties already.
    let mut templated = info(0, "Took 5 ms", json!({}));
    templated.rec.template = Some("Took {Elapsed} ms".into());
    assert!(number_candidates(&vec![templated; 10]).is_empty());
}

#[test]
fn pascal_names() {
    assert_eq!(numbers::pascal_for_tests("CPU"), "Cpu");
    assert_eq!(numbers::pascal_for_tests("WorkingSet"), "WorkingSet");
    assert_eq!(numbers::pascal_for_tests("queue_depth"), "QueueDepth");
}

// ---- template regex, fragments, diff ----

#[test]
fn template_regexes_match_their_messages() {
    let re = regex::Regex::new(&template_regex("Dispatched <*> outbox (<*>) messages.")).unwrap();
    assert!(re.is_match("Dispatched 16 outbox (fast lane) messages."));
    assert!(!re.is_match("Received 16 outbox messages"));
    let c = template_condition("a {B}", TemplateKind::Serilog);
    assert_eq!((c.field.as_str(), c.op.as_str()), ("@template", "eq"));
}

fn marker(kind: &str, value: &str) -> Fragment {
    Fragment::Marker {
        marker: MarkerCfg {
            kind: kind.into(),
            label: None,
            lifecycle: true,
            color: None,
            icon: None,
            badge: None,
            shape: None,
            description: None,
            when: vec![ConditionCfg {
                field: "@message".into(),
                op: "contains".into(),
                value: Some(CondValue::Text(value.into())),
            }],
        },
    }
}

#[test]
fn composing_keeps_defaults_puts_new_markers_first_and_rejects_duplicates() {
    let span_of = |kind: &str| Fragment::Span {
        span: crate::spans::SpanCfg {
            kind: kind.into(),
            label: None,
            start: crate::spans::OneOrMany::Many(vec!["start".into(), "svc-start".into()]),
            stop: crate::spans::OneOrMany::One("stop".into()),
            per: "instance".into(),
            color: None,
            max_duration: None,
            restart_on_start: true,
            enabled: true,
        },
    };
    let span = span_of("run");
    let frags = vec![
        marker("svc-start", "Booted"),
        Fragment::InstanceKey { key: vec!["Host".into()] },
        span,
        Fragment::ParseRegex {
            regex: "^(?P<message>.*)$".into(),
        },
    ];
    let (before, after) = compose_toml("name = \"Mine\"\n", &frags).unwrap();
    assert_eq!(before.trim(), "name = \"Mine\"");
    let p = Profile::from_toml(&after).unwrap();
    assert_eq!(p.name, "Mine");
    assert_eq!(p.instance.key, vec!["Host".to_string()]);
    assert_eq!(p.marker[0].kind, "svc-start");
    assert!(p.marker.iter().any(|m| m.kind == "fatal"), "the built-in markers are kept");
    assert!(p.parse.regexes.len() > 1 && p.parse.regexes.last().unwrap() == "^(?P<message>.*)$");
    let t: toml::Table = toml::from_str(&after).unwrap();
    let spans = t["span"].as_array().unwrap();
    // Like the Runs editor: the default list is kept, and a rule of the same kind is replaced in place.
    assert_eq!(spans.len(), 1, "{spans:?}");
    assert_eq!(p.span[0].start.list(), vec!["start".to_string(), "svc-start".to_string()]);
    let (_, two) = compose_toml(&after, &[span_of("other")]).unwrap();
    assert_eq!(Profile::from_toml(&two).unwrap().span.len(), 2, "another kind is added last");
    // `lifecycle = true` is written, empty lists and false defaults are not.
    assert!(after.contains("lifecycle = true") && !after.contains("when = []"));
    let dup = compose_toml(&after, &[marker("svc-start", "x")]).unwrap_err();
    assert!(dup.to_string().contains("already has a marker named \"svc-start\""), "{dup}");
    assert!(compose_toml("not toml [", &frags).is_err());
    assert!(compose_toml("", &[Fragment::InstanceKey { key: vec![] }]).is_err());
}

#[test]
fn fragment_snippets_show_the_change() {
    let t = fragment_toml(&[
        marker("m", "x"),
        Fragment::SavedQuery {
            name: "Errors".into(),
            ql: "level >= error".into(),
        },
        Fragment::Pipeline {
            step: StepCfg {
                step: "drop".into(),
                ..StepCfg::default()
            },
        },
    ])
    .unwrap();
    assert!(t.starts_with("# Saved query \"Errors\": level >= error\n"));
    assert!(t.contains("[[marker]]") && t.contains("[[pipeline]]") && t.contains("step = \"drop\""));
    assert!(!t.contains("disabled"));
}

#[test]
fn line_diff_marks_added_and_removed_lines() {
    let d = line_diff("a\nb\nc\nd", "a\nx\nc\nd\ne");
    let ops: Vec<(DiffOp, &str)> = d.iter().map(|l| (l.op, l.text.as_str())).collect();
    assert_eq!(
        ops,
        vec![
            (DiffOp::Same, "a"),
            (DiffOp::Add, "x"),
            (DiffOp::Remove, "b"),
            (DiffOp::Same, "c"),
            (DiffOp::Same, "d"),
            (DiffOp::Add, "e"),
        ]
    );
    assert!(line_diff("same", "same").iter().all(|l| l.op == DiffOp::Same));
    let big_a: String = (0..2500).map(|i| format!("a{i}\n")).collect();
    let big_b: String = (0..2500).map(|i| format!("b{i}\n")).collect();
    let d = line_diff(&big_a, &big_b);
    assert_eq!(d.iter().filter(|l| l.op == DiffOp::Remove).count(), 2500);
}

// ---- validation ----

fn store_with(lines: &str) -> (Workspace, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("imt-survey-{}-{}", std::process::id(), lines.len()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("app.log");
    std::fs::write(&file, lines).unwrap();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    (ws, dir)
}

fn candidate(id: &str, group: ProposalGroup, fragments: Vec<Fragment>, requires: Vec<&str>) -> Candidate {
    Candidate {
        id: id.into(),
        group,
        origin: ProposalOrigin::Ai,
        title: id.into(),
        summary: String::new(),
        fragments,
        requires: requires.into_iter().map(String::from).collect(),
        recommended: true,
    }
}

fn log_lines() -> String {
    let mut s = String::new();
    for i in 0..40 {
        s.push_str(&format!(
            "2026-10-14 09:00:{:02}.000 +00:00 [INF] worker-{} handled job in {} ms\n",
            i,
            i % 3,
            10 + i
        ));
    }
    s.push_str("2026-10-14 09:01:00.000 +00:00 [INF] Booted the engine\n");
    s
}

#[test]
fn invalid_proposals_are_skipped_with_a_reason_and_valid_ones_previewed() {
    let (ws, dir) = store_with(&log_lines());
    let (survey, sampled) = survey_only(&ws, &SurveyRequest::default(), &SurveyControl::new()).unwrap();
    assert_eq!(sampled.events.len(), 41);
    let extractor = |name: &str, regex: &str| Fragment::Extractor {
        extractor: ExtractCfg {
            name: name.into(),
            from: "@message".into(),
            regex: regex.into(),
            when: vec![],
            fields: BTreeMap::new(),
        },
    };
    let cands = vec![
        candidate("ok-marker", ProposalGroup::Markers, vec![marker("booted", "Booted")], vec![]),
        candidate("no-match", ProposalGroup::Markers, vec![marker("nothing", "never written")], vec![]),
        candidate("everything", ProposalGroup::Markers, vec![marker("all", "e")], vec![]),
        candidate("bad-regex", ProposalGroup::Extractors, vec![extractor("bad", "(")], vec![]),
        candidate(
            "ok-extract",
            ProposalGroup::Extractors,
            vec![extractor("took", r"in (?P<Took>\d+) ms")],
            vec![],
        ),
        candidate(
            "needs-missing",
            ProposalGroup::Charts,
            vec![marker("x", "Booted")],
            vec!["no-match"],
        ),
        candidate(
            "lanes",
            ProposalGroup::Instances,
            vec![
                extractor("w", r"worker-(?P<Worker>\d+)"),
                Fragment::InstanceKey {
                    key: vec!["Worker".into()],
                },
            ],
            vec![],
        ),
        candidate(
            "one-lane",
            ProposalGroup::Instances,
            vec![Fragment::InstanceKey {
                key: vec!["@level".into()],
            }],
            vec![],
        ),
        candidate(
            "drop-all",
            ProposalGroup::Noise,
            vec![Fragment::Pipeline {
                step: StepCfg {
                    step: "drop".into(),
                    ..StepCfg::default()
                },
            }],
            vec![],
        ),
        candidate(
            "bad-query",
            ProposalGroup::Queries,
            vec![Fragment::SavedQuery {
                name: "q".into(),
                ql: "level >=".into(),
            }],
            vec![],
        ),
        candidate("nothing", ProposalGroup::Charts, vec![], vec![]),
        candidate("ok-marker", ProposalGroup::Markers, vec![marker("dup", "Booted")], vec![]),
    ];
    let (ok, skipped) = validate(&ws, DEFAULT_PROFILE_TOML, &survey, &sampled, cands, &SurveyControl::new()).unwrap();
    let ids: Vec<&str> = ok.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, vec!["ok-marker", "ok-extract", "lanes"], "{skipped:?}");
    let reason = |id: &str| skipped.iter().find(|s| s.id == id).map(|s| s.reason.clone()).unwrap_or_default();
    assert!(reason("no-match").contains("matches none"));
    assert!(reason("everything").contains("every sampled event"));
    assert!(reason("bad-regex").contains("compile"), "{}", reason("bad-regex"));
    assert!(reason("needs-missing").contains("needs the proposal"));
    assert!(reason("one-lane").contains("1 lane"));
    assert!(reason("drop-all").contains("drop every"));
    assert!(reason("bad-query").contains("not valid"));
    assert!(reason("nothing").contains("changes nothing"));
    match &ok[2].preview {
        ProposalPreview::Lanes { total_lanes, .. } => assert_eq!(*total_lanes, 3),
        p => panic!("{p:?}"),
    }
    match &ok[1].preview {
        ProposalPreview::Extracted { matches, .. } => assert_eq!(*matches, 40),
        p => panic!("{p:?}"),
    }
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn heuristics_end_to_end_on_a_small_folder() {
    let (ws, dir) = store_with(&log_lines());
    let setup = ws
        .survey_folder(DEFAULT_PROFILE_TOML, &SurveyRequest::default(), &SurveyControl::new())
        .unwrap();
    assert!(setup.proposals.iter().any(|p| p.id == "instance:Worker"), "{:?}", setup.proposals);
    assert!(setup.proposals.iter().any(|p| p.id == "extract:ElapsedMs"));
    let composed = compose(&ws, DEFAULT_PROFILE_TOML, &setup.proposals[0].fragments).unwrap();
    assert!(composed.added > 0 && composed.diff.iter().any(|l| l.op == DiffOp::Add));
    assert!(compose(&ws, DEFAULT_PROFILE_TOML, &[]).is_err());
    let bad_query = compose(
        &ws,
        DEFAULT_PROFILE_TOML,
        &[Fragment::SavedQuery {
            name: "q".into(),
            ql: "((".into(),
        }],
    );
    assert!(bad_query.is_err());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn limits_are_clamped_and_progress_is_reported() {
    let l = SurveyLimits::from_request(&SurveyRequest {
        max_events_per_group: Some(usize::MAX),
        max_events: Some(0),
        max_millis: Some(u64::MAX),
    });
    assert_eq!(l.per_group, SurveyLimits::MAX_PER_GROUP);
    assert_eq!(l.total, 10);
    assert_eq!(l.time.as_millis() as u64, SurveyLimits::MAX_MILLIS);
    let c = SurveyControl::new();
    c.begin(SurveyStage::Validating, 4);
    c.advance(3);
    assert_eq!(
        c.progress(),
        SurveyProgress {
            stage: SurveyStage::Validating,
            done: 3,
            total: 4
        }
    );
    assert!(c.check().is_ok());
    c.clone().cancel();
    assert_eq!(c.check(), Err(SurveyError::Cancelled));
}

#[test]
fn unrecognised_files_are_grouped_with_their_first_lines() {
    let dir = std::env::temp_dir().join(format!("imt-survey-unrec-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("dump.reg");
    std::fs::write(&path, "Windows Registry Editor\n\n[HKEY_LOCAL_MACHINE]\n").unwrap();
    let files = vec![
        crate::api::UnrecognizedFile {
            path: path.to_string_lossy().into_owned(),
            name: "dump.reg".into(),
            reason: "no importer recognises this format".into(),
        },
        crate::api::UnrecognizedFile {
            path: "x.zip!/inner.bin".into(),
            name: "x.zip/inner.bin".into(),
            reason: "binary file".into(),
        },
    ];
    let g = unrecognised::groups(&files);
    let reg = g.iter().find(|g| g.extension == "reg").unwrap();
    assert_eq!(
        reg.sample_lines,
        vec!["Windows Registry Editor".to_string(), "[HKEY_LOCAL_MACHINE]".to_string()]
    );
    let bin = g.iter().find(|g| g.extension == "bin").unwrap();
    assert!(bin.sample_lines.is_empty() && bin.sample_file.is_none());
    assert_eq!(unrecognised::extension("noext"), "(none)");
    assert_eq!(unrecognised::head_lines("does/not/exist"), None);
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn run_rules_on_existing_markers_are_previewed_by_the_engine_and_invalid_ones_skipped() {
    let mut lines = String::new();
    for (i, msg) in ["Application started", "work", "Application stopped", "Application started", "work"]
        .iter()
        .enumerate()
    {
        lines.push_str(&format!("2026-10-14 09:00:{i:02}.000 +00:00 [INF] {msg}\n"));
    }
    let (ws, dir) = store_with(&lines);
    let (survey, sampled) = survey_only(&ws, &SurveyRequest::default(), &SurveyControl::new()).unwrap();
    let span = |kind: &str, start: &str| Fragment::Span {
        span: crate::spans::SpanCfg {
            kind: kind.into(),
            label: Some("Running".into()),
            start: crate::spans::OneOrMany::One(start.into()),
            stop: crate::spans::OneOrMany::One("stop".into()),
            per: "instance".into(),
            color: None,
            max_duration: None,
            restart_on_start: true,
            enabled: true,
        },
    };
    let cands = vec![
        candidate("span:run", ProposalGroup::Markers, vec![span("run", "start")], vec![]),
        candidate("bad-kind", ProposalGroup::Markers, vec![span("has space", "start")], vec![]),
    ];
    let (ok, skipped) = validate(&ws, DEFAULT_PROFILE_TOML, &survey, &sampled, cands, &SurveyControl::new()).unwrap();
    match &ok.first().expect("the run rule is valid").preview {
        ProposalPreview::Runs { runs, closed, open, .. } => assert_eq!((*runs, *closed, *open), (2, 1, 1)),
        p => panic!("{p:?}"),
    }
    assert!(
        skipped
            .iter()
            .any(|s| s.id == "bad-kind" && s.reason.contains("must be 1–64 letters")),
        "{skipped:?}"
    );
    std::fs::remove_dir_all(dir).ok();
}
