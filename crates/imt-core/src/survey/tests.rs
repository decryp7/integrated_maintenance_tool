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
            file_name: "notes.txt".into(),
        },
    ];
    let (before, after) = compose_toml("name = \"Mine\"\n", &frags).unwrap();
    assert_eq!(before.trim(), "name = \"Mine\"");
    let p = Profile::from_toml(&after).unwrap();
    assert_eq!(p.name, "Mine");
    assert_eq!(p.instance.key, vec!["Host".to_string()]);
    assert_eq!(p.marker[0].kind, "svc-start");
    assert!(p.marker.iter().any(|m| m.kind == "fatal"), "the built-in markers are kept");
    // A format is a new parser, not a profile change (ADR 0026).
    assert!(!after.contains("regexes"));
    let parsers = crate::survey::parser_drafts(&frags).unwrap();
    assert_eq!(parsers.len(), 1);
    let parser = crate::parser::Parser::from_toml(&parsers[0]).unwrap();
    assert_eq!(
        (parser.id.as_str(), parser.format.regexes.as_slice()),
        ("custom-notes", &["^(?P<message>.*)$".to_string()][..])
    );
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

fn store_with(lines: &str) -> (crate::workspace::WorkspaceStore, std::path::PathBuf) {
    let dir = std::env::temp_dir().join(format!("imt-survey-{}-{}", std::process::id(), lines.len()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("app.log");
    std::fs::write(&file, lines).unwrap();
    let mut ws = Workspace::new(Profile::default_profile()).unwrap();
    ws.open(std::slice::from_ref(&dir)).unwrap();
    (crate::workspace::WorkspaceStore::new(ws), dir)
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
    let setup = run(&ws, DEFAULT_PROFILE_TOML, &SurveyRequest::default(), &SurveyControl::new()).unwrap();
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
            crashed: false,
        },
        crate::api::UnrecognizedFile {
            path: "x.zip!/inner.bin".into(),
            name: "x.zip/inner.bin".into(),
            reason: "binary file".into(),
            crashed: false,
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

// ---- validation: more previews and failure reasons ----

fn extractor_fragment(name: &str, regex: &str) -> Fragment {
    Fragment::Extractor {
        extractor: ExtractCfg {
            name: name.into(),
            from: "@message".into(),
            regex: regex.into(),
            when: vec![],
            fields: BTreeMap::new(),
        },
    }
}

fn run_validate(
    ws: &crate::workspace::WorkspaceStore,
    cands: Vec<Candidate>,
    tweak: impl FnOnce(&mut FolderSurvey),
) -> (Vec<SetupProposal>, Vec<SkippedProposal>) {
    let (mut survey, sampled) = survey_only(ws, &SurveyRequest::default(), &SurveyControl::new()).unwrap();
    tweak(&mut survey);
    validate(ws, DEFAULT_PROFILE_TOML, &survey, &sampled, cands, &SurveyControl::new()).unwrap()
}

fn skipped_reason(skipped: &[SkippedProposal], id: &str) -> String {
    skipped
        .iter()
        .find(|s| s.id == id)
        .map(|s| s.reason.clone())
        .unwrap_or_else(|| panic!("{id} was not skipped: {skipped:?}"))
}

#[test]
fn previews_reject_proposals_that_do_nothing_useful() {
    let (ws, dir) = store_with(&log_lines());
    let chart = |id: &str, y: &str| {
        candidate(
            id,
            ProposalGroup::Charts,
            vec![Fragment::Chart {
                chart: crate::profile::ChartCfg {
                    id: Some(id.into()),
                    title: id.into(),
                    y: y.into(),
                    bucket: "auto".into(),
                    split_by: vec!["@level".into()],
                    kind: "line".into(),
                    unit: String::new(),
                    when: vec![],
                },
            }],
            vec![],
        )
    };
    let cands = vec![
        chart("chart-count", "count()"),
        chart("chart-no-number", "avg(NoSuchField)"),
        chart("chart-bad-y", "median(x)"),
        candidate(
            "no-extract",
            ProposalGroup::Extractors,
            vec![extractor_fragment("none", r"zzz(?P<Z>\d+)")],
            vec![],
        ),
        candidate(
            "no-conditions",
            ProposalGroup::Markers,
            vec![Fragment::Marker {
                marker: MarkerCfg {
                    kind: "empty".into(),
                    label: None,
                    lifecycle: false,
                    color: None,
                    icon: None,
                    badge: None,
                    shape: None,
                    description: None,
                    when: vec![],
                },
            }],
            vec![],
        ),
        candidate(
            "query-none",
            ProposalGroup::Queries,
            vec![Fragment::SavedQuery {
                name: "q".into(),
                ql: "level >= fatal".into(),
            }],
            vec![],
        ),
        candidate(
            "query-ok",
            ProposalGroup::Queries,
            vec![Fragment::SavedQuery {
                name: "all".into(),
                ql: "level >= information".into(),
            }],
            vec![],
        ),
        candidate(
            "dash",
            ProposalGroup::Charts,
            vec![Fragment::Dashboard {
                dashboard: crate::dashboard::DashboardCfg {
                    id: "d".into(),
                    title: "D".into(),
                    columns: 12,
                    tile: vec![],
                },
            }],
            vec![],
        ),
    ];
    let (ok, skipped) = run_validate(&ws, cands, |_| {});
    let by = |id: &str| ok.iter().find(|p| p.id == id).map(|p| &p.preview);
    match by("chart-count") {
        Some(ProposalPreview::Chart { points, series }) => {
            assert_eq!(*points, 41);
            assert!(!series.is_empty());
        }
        p => panic!("{p:?} / {skipped:?}"),
    }
    assert!(skipped_reason(&skipped, "chart-no-number").contains("no sampled event has a number in “NoSuchField”"));
    assert!(skipped_reason(&skipped, "chart-bad-y").contains("unknown aggregate"));
    assert!(skipped_reason(&skipped, "no-extract").contains("extracts nothing"));
    assert!(skipped_reason(&skipped, "no-conditions").contains("no conditions"));
    assert!(skipped_reason(&skipped, "query-none").contains("finds no events"));
    assert!(
        matches!(by("query-ok"), Some(ProposalPreview::Query { matches: 41 })),
        "{skipped:?}"
    );
    assert!(matches!(by("dash"), Some(ProposalPreview::Dashboard { tiles: 0 })), "{skipped:?}");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn an_instance_key_with_too_many_lanes_is_unreadable() {
    let mut lines = String::new();
    for i in 0..(super::fields::MAX_LANES + 6) {
        lines.push_str(&format!("2026-10-14 09:00:00.{i:03} +00:00 [INF] worker-{i} idle\n"));
    }
    let (ws, dir) = store_with(&lines);
    let key = |name: &str| {
        candidate(
            "many",
            ProposalGroup::Instances,
            vec![
                extractor_fragment("w", r"worker-(?P<Worker>\d+)"),
                Fragment::InstanceKey { key: vec![name.into()] },
            ],
            vec![],
        )
    };
    let (ok, skipped) = run_validate(&ws, vec![key("Worker")], |_| {});
    assert!(ok.is_empty());
    assert!(skipped_reason(&skipped, "many").contains("unreadable"), "{skipped:?}");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn pipeline_steps_show_what_they_change_and_reject_no_ops() {
    let (ws, dir) = store_with(&log_lines());
    let step = |id: &str, s: StepCfg| candidate(id, ProposalGroup::Noise, vec![Fragment::Pipeline { step: s }], vec![]);
    let cands = vec![
        step(
            "set-field",
            StepCfg {
                step: "set".into(),
                field: Some("Env".into()),
                value: Some("prod".into()),
                ..StepCfg::default()
            },
        ),
        step(
            "drop-booted",
            StepCfg {
                step: "drop".into(),
                when: vec![ConditionCfg {
                    field: "@message".into(),
                    op: "contains".into(),
                    value: Some(CondValue::Text("Booted".into())),
                }],
                ..StepCfg::default()
            },
        ),
        step(
            "noop",
            StepCfg {
                step: "set".into(),
                field: Some("Never".into()),
                value: Some("x".into()),
                when: vec![ConditionCfg {
                    field: "@message".into(),
                    op: "contains".into(),
                    value: Some(CondValue::Text("never written".into())),
                }],
                ..StepCfg::default()
            },
        ),
        step(
            "bad-step",
            StepCfg {
                step: "no_such_step".into(),
                ..StepCfg::default()
            },
        ),
    ];
    let (ok, skipped) = run_validate(&ws, cands, |_| {});
    match &ok.iter().find(|p| p.id == "set-field").expect("valid").preview {
        ProposalPreview::Pipeline {
            sampled,
            kept,
            dropped,
            changed,
            examples,
        } => {
            assert_eq!((*sampled, *kept, *dropped, *changed), (41, 41, 0, 41));
            assert_eq!(examples.len(), 3);
            assert!(examples[0].after.contains("[Env=prod]"), "{:?}", examples[0]);
        }
        p => panic!("{p:?}"),
    }
    match &ok.iter().find(|p| p.id == "drop-booted").expect("valid").preview {
        ProposalPreview::Pipeline { dropped, examples, .. } => {
            assert_eq!(*dropped, 1);
            assert!(examples[0].before.contains("Booted") && examples[0].after.is_empty());
        }
        p => panic!("{p:?}"),
    }
    assert!(skipped_reason(&skipped, "noop").contains("changes none"));
    assert!(!skipped_reason(&skipped, "bad-step").is_empty());
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn run_rules_per_field_and_without_starts() {
    let mut lines = String::new();
    for (i, msg) in [
        "Application started worker-1",
        "Application started worker-2",
        "Application stopped worker-1",
    ]
    .iter()
    .enumerate()
    {
        lines.push_str(&format!("2026-10-14 09:00:{i:02}.000 +00:00 [INF] {msg}\n"));
    }
    let (ws, dir) = store_with(&lines);
    let span = |per: &str, start: &str| Fragment::Span {
        span: crate::spans::SpanCfg {
            kind: "run".into(),
            label: None,
            start: crate::spans::OneOrMany::One(start.into()),
            stop: crate::spans::OneOrMany::One("stop".into()),
            per: per.into(),
            color: None,
            max_duration: None,
            restart_on_start: false,
            enabled: true,
        },
    };
    let with_worker = |s: Fragment| vec![s, extractor_fragment("w", r"(?P<Worker>worker-\d+)")];
    let cands = vec![
        candidate(
            "per-field",
            ProposalGroup::Markers,
            with_worker(span("field:Worker", "start")),
            vec![],
        ),
        candidate("no-start", ProposalGroup::Markers, vec![span("instance", "crash")], vec![]),
    ];
    let (ok, skipped) = run_validate(&ws, cands, |_| {});
    let run = ok.iter().find(|p| p.id == "per-field").unwrap_or_else(|| panic!("{skipped:?}"));
    match &run.preview {
        ProposalPreview::Runs {
            runs, closed, open, lanes, ..
        } => assert_eq!((*runs, *closed, *open, *lanes), (2, 1, 1, 1)),
        p => panic!("{p:?}"),
    }
    assert_eq!(skipped_reason(&skipped, "no-start"), "no run starts in the sampled events");
    std::fs::remove_dir_all(dir).ok();
}

#[test]
fn formats_for_unrecognised_files_keep_the_best_reading_and_explain_the_rest() {
    let (ws, dir) = store_with(&log_lines());
    let lines: Vec<String> = (0..10)
        .map(|i| {
            let level = if i < 7 { "INFO" } else { "WARN" };
            format!("2026-10-14 09:00:{i:02} {level} widget {i} ready")
        })
        .collect();
    let group = |ext: &str, lines: Vec<String>| UnrecognisedGroup {
        extension: ext.into(),
        file_count: 1,
        files: vec![format!("x.{ext}")],
        reason: "no importer recognises this format".into(),
        sample_file: Some(format!("x.{ext}")),
        sample_lines: lines,
    };
    let format = |id: &str, regex: &str, file: &str, origin: ProposalOrigin| {
        let mut c = candidate(
            id,
            ProposalGroup::Formats,
            vec![Fragment::ParseRegex {
                regex: regex.into(),
                file_name: file.into(),
            }],
            vec![],
        );
        c.origin = origin;
        c
    };
    let all = r"^(?P<timestamp>\d{4}-\d\d-\d\d \d\d:\d\d:\d\d) (?P<level>\w+) (?P<message>.*)$";
    let info_only = r"^(?P<timestamp>\d{4}-\d\d-\d\d \d\d:\d\d:\d\d) (?P<level>INFO) (?P<message>.*)$";
    let few = r"^(?P<timestamp>\d{4}-\d\d-\d\d \d\d:\d\d:\d\d) (?P<level>WARN) (?P<message>widget 7 .*)$";
    let cands = vec![
        format("format:thing:1", info_only, "x.thing", ProposalOrigin::Heuristic),
        format("format:thing:2", all, "x.thing", ProposalOrigin::Heuristic),
        format("format:thing:3", info_only, "x.thing", ProposalOrigin::Ai),
        format("format:thing:4", few, "x.thing", ProposalOrigin::Heuristic),
        format("format:nolines:1", all, "x.nolines", ProposalOrigin::Heuristic),
        format("format:broken:1", "(", "x.broken", ProposalOrigin::Heuristic),
    ];
    let (ok, skipped) = run_validate(&ws, cands, |s| {
        s.unrecognised = vec![
            group("thing", lines.clone()),
            group("nolines", vec![]),
            group("broken", lines.clone()),
        ];
    });
    let ids: Vec<&str> = ok.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(
        ids,
        ["format:thing:2", "format:thing:3"],
        "best heuristic format plus the AI one: {skipped:?}"
    );
    match &ok[0].preview {
        ProposalPreview::Parse {
            parsed,
            lines,
            with_timestamp,
            examples,
        } => {
            assert_eq!((*lines, *with_timestamp), (10, 10));
            assert_eq!(*parsed, 10);
            assert!(examples.len() <= 3 && examples[0].contains("widget"));
        }
        p => panic!("{p:?}"),
    }
    assert_eq!(
        skipped_reason(&skipped, "format:thing:1"),
        "another format reads more of these files"
    );
    assert!(skipped_reason(&skipped, "format:thing:4").contains("sample lines with a timestamp"));
    assert!(skipped_reason(&skipped, "format:nolines:1").contains("no sample lines"));
    assert!(skipped_reason(&skipped, "format:broken:1").contains("not valid"), "{skipped:?}");
    std::fs::remove_dir_all(dir).ok();
}

// ---- heuristic proposals from a hand-built survey ----

fn lifecycle(role: LifecycleRole, template: &str, covered_by: Option<&str>) -> LifecycleCandidate {
    LifecycleCandidate {
        role,
        template: template.into(),
        template_kind: TemplateKind::Mined,
        count: 2,
        example: template.replace(crate::patterns::WILDCARD, "7"),
        covered_by: covered_by.map(String::from),
    }
}

fn number(name: &str, kind: NumberKind, unit: Option<&str>) -> NumberCandidate {
    NumberCandidate {
        name: name.into(),
        label: name.to_lowercase(),
        unit: unit.map(String::from),
        number_kind: kind,
        count: 10,
        min: 1.0,
        max: 9.0,
        example: format!("{name} 5"),
        regex: format!(r"{name} (?P<{name}>\d+)"),
    }
}

fn candidate_by<'a>(list: &'a [Candidate], id: &str) -> &'a Candidate {
    list.iter()
        .find(|c| c.id == id)
        .unwrap_or_else(|| panic!("no {id} in {:?}", list.iter().map(|c| &c.id).collect::<Vec<_>>()))
}

#[test]
fn lifecycle_findings_become_uniquely_named_markers_and_one_new_run_rule() {
    let mut profile = Profile::default_profile();
    profile.span.clear();
    let survey = FolderSurvey {
        markers: vec![MarkerCoverage {
            kind: "svc-start".into(),
            label: "x".into(),
            lifecycle: true,
            matches: 1,
        }],
        lifecycle: vec![
            lifecycle(LifecycleRole::Start, "Service booted", None),
            lifecycle(LifecycleRole::Start, "Service booted again", Some("start")),
            lifecycle(LifecycleRole::Stop, "Service halted", None),
            lifecycle(LifecycleRole::Stop, "Service halted gracefully", Some("stop")),
            lifecycle(LifecycleRole::Crash, "Fatal fault", None),
            lifecycle(LifecycleRole::Crash, "Fatal fault two", None),
            lifecycle(LifecycleRole::Crash, "Fatal fault three", Some("crash")),
        ],
        ..FolderSurvey::default()
    };
    let list = propose::propose(&survey, &profile);
    // "svc-start" is taken by a marker that exists: the new one gets a suffix; crash names are made unique too.
    let ids: Vec<&str> = list.iter().map(|c| c.id.as_str()).collect();
    for want in [
        "marker:svc-start-2",
        "marker:svc-stop",
        "marker:crash",
        "marker:crash-2",
        "span:run",
    ] {
        assert!(ids.contains(&want), "{want} in {ids:?}");
    }
    let run = candidate_by(&list, "span:run");
    assert_eq!(
        run.requires,
        ["marker:svc-start-2", "marker:svc-stop"],
        "a crash marker is not part of a run"
    );
    assert!(run.title.starts_with("Show runs from start to stop"));
    match &run.fragments[0] {
        Fragment::Span { span } => {
            assert_eq!(span.start.list(), ["svc-start-2", "start"]);
            assert_eq!(span.stop.list(), ["svc-stop", "stop"]);
        }
        f => panic!("{f:?}"),
    }
}

#[test]
fn an_existing_run_rule_is_extended_only_when_something_new_was_found() {
    let mut profile = Profile::default_profile();
    let existing = crate::spans::SpanCfg {
        kind: "run".into(),
        label: Some("Service".into()),
        start: crate::spans::OneOrMany::One("start".into()),
        stop: crate::spans::OneOrMany::One("stop".into()),
        per: "instance".into(),
        color: None,
        max_duration: None,
        restart_on_start: true,
        enabled: true,
    };
    profile.span = vec![existing.clone()];
    let nothing_new = FolderSurvey {
        lifecycle: vec![lifecycle(LifecycleRole::Start, "Already covered", Some("start"))],
        ..FolderSurvey::default()
    };
    assert!(propose::propose(&nothing_new, &profile)
        .iter()
        .all(|c| c.group != ProposalGroup::Markers));

    let new_start = FolderSurvey {
        lifecycle: vec![lifecycle(LifecycleRole::Start, "Service booted", None)],
        ..FolderSurvey::default()
    };
    let list = propose::propose(&new_start, &profile);
    let run = candidate_by(&list, "span:run");
    assert!(
        run.title.contains("“Service” runs start and stop on the new markers"),
        "{}",
        run.title
    );

    // A rule without stops stays unusable when only a start was found.
    profile.span = vec![crate::spans::SpanCfg {
        stop: crate::spans::OneOrMany::Many(vec![]),
        ..existing
    }];
    assert!(propose::propose(&new_start, &profile).iter().all(|c| c.id != "span:run"));

    // Without any run rule, a start alone is not enough either.
    profile.span.clear();
    assert!(propose::propose(&new_start, &profile).iter().all(|c| c.id != "span:run"));
}

#[test]
fn numbers_get_typed_extractors_and_the_chart_uses_the_first_metric() {
    let mut profile = Profile::default_profile();
    profile.chart.clear();
    let survey = FolderSurvey {
        numbers: vec![
            number("Waited", NumberKind::Duration, Some("sec")),
            number("Sent", NumberKind::Bytes, Some("bytes")),
            number("Cache", NumberKind::Bytes, Some("KB")),
            number("Retries", NumberKind::Number, None),
        ],
        ..FolderSurvey::default()
    };
    let list = propose::propose(&survey, &profile);
    let spec = |id: &str| match &candidate_by(&list, id).fragments[0] {
        Fragment::Extractor { extractor } => extractor.fields.values().next().cloned().unwrap(),
        f => panic!("{f:?}"),
    };
    let waited = spec("extract:Waited");
    assert_eq!((waited.unit.as_deref(), waited.to.as_deref()), (Some("s"), Some("ms")));
    let sent = spec("extract:Sent");
    assert_eq!((sent.unit.as_deref(), sent.to.as_deref()), (Some("B"), Some("MB")));
    assert_eq!(spec("extract:Cache").unit.as_deref(), Some("KB"));
    assert_eq!(spec("extract:Retries").to, None);
    // The chart of the first metric needs its extractor.
    let chart = list.iter().find(|c| c.id.starts_with("chart:waited")).expect("a metric chart");
    assert!(chart.requires.contains(&"extract:Waited".to_string()), "{:?}", chart.requires);
    // Without numbers or measurement-like fields there is only the error chart.
    let bare = propose::propose(&FolderSurvey::default(), &profile);
    assert!(
        bare.iter()
            .filter(|c| c.group == ProposalGroup::Charts)
            .all(|c| c.id.starts_with("chart:errors")),
        "{bare:?}"
    );
}

#[test]
fn an_instance_key_the_profile_already_uses_is_not_proposed_again() {
    let mut profile = Profile::default_profile();
    profile.instance.key = vec!["MachineName".into()];
    let cand = |field: &str| InstanceCandidate {
        field: field.into(),
        origin: KeyOrigin::Field,
        lanes: 3,
        coverage: 0.9,
        score: 1.0,
        examples: vec!["A".into()],
    };
    let survey = FolderSurvey {
        instance_candidates: vec![cand("MachineName"), cand("Worker")],
        ..FolderSurvey::default()
    };
    let list = propose::propose(&survey, &profile);
    assert!(list.iter().all(|c| c.id != "instance:MachineName"));
    let worker = candidate_by(&list, "instance:Worker");
    assert!(!worker.recommended, "only the best candidate is recommended");
    // The error chart splits by the key the profile already has.
    let chart = candidate_by(&list, "chart:errors-by-instance");
    assert!(chart.title.contains("MachineName"), "{}", chart.title);
}

#[test]
fn error_queries_use_the_longest_literal_run_and_skip_templates_without_one() {
    let err = |template: &str, kind: TemplateKind| TemplateSummary {
        template: template.into(),
        template_kind: kind,
        count: 3,
        level: Level::Error,
        share: 0.1,
        example: template.into(),
    };
    let w = crate::patterns::WILDCARD;
    let survey = FolderSurvey {
        errors: vec![
            err(&format!("failed {w} to write the cache entry {w} twice"), TemplateKind::Mined),
            err(&format!("{w} ab {w} cd {w}"), TemplateKind::Mined),
            err("Could not open {Path}", TemplateKind::Serilog),
        ],
        ..FolderSurvey::default()
    };
    let list = propose::propose(&survey, &Profile::default_profile());
    let queries: Vec<&Candidate> = list.iter().filter(|c| c.group == ProposalGroup::Queries).collect();
    assert_eq!(queries.len(), 2, "the template with only short literals is skipped");
    let ql = |c: &Candidate| match &c.fragments[0] {
        Fragment::SavedQuery { ql, .. } => ql.clone(),
        f => panic!("{f:?}"),
    };
    assert_eq!(
        ql(queries[0]),
        r#"level >= warning and @message contains "to write the cache entry""#
    );
    assert_eq!(ql(queries[1]), r#"@template = "Could not open {Path}""#);
    assert_eq!(propose::ql_quote(r#"a "b" \c"#), r#""a \"b\" \\c""#);
}
