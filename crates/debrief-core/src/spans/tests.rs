use super::config::{self, check_toml, Per, SpanCfg, SpanField};
use super::draft;
use super::*;
use crate::profile::{CompiledProfile, Profile};

const S: Edge = Edge::Start;
const P: Edge = Edge::Stop;
const O: Edge = Edge::Other;

/// Feeds (lane, value, edge, ts) events; ids are their positions.
fn pair(restart: bool, events: &[(&str, Option<&str>, Edge, i64)]) -> Pairer {
    let mut p = Pairer::new(restart);
    for (i, (lane, value, edge, ts)) in events.iter().enumerate() {
        p.feed(lane, *value, *edge, i as u32, *ts, || format!("msg {i}"));
    }
    p
}

fn shape(p: &Pairer) -> Vec<(String, Option<String>, RunStatus, i64, i64)> {
    p.runs().into_iter().map(|r| (r.lane, r.group, r.status, r.from, r.to)).collect()
}

fn run(lane: &str, status: RunStatus, from: i64, to: i64) -> (String, Option<String>, RunStatus, i64, i64) {
    (lane.into(), None, status, from, to)
}

#[test]
fn interleaved_instances_in_one_file_pair_per_lane() {
    let p = pair(
        true,
        &[
            ("A", None, S, 1),
            ("B", None, S, 2),
            ("A", None, O, 3),
            ("B", None, O, 4),
            ("B", None, P, 5),
            ("A", None, P, 6),
            ("B", None, S, 7),
            ("B", None, O, 9),
        ],
    );
    assert_eq!(
        shape(&p),
        vec![
            run("A", RunStatus::Closed, 1, 6),
            run("B", RunStatus::Closed, 2, 5),
            run("B", RunStatus::Open, 7, 9)
        ]
    );
    let runs = p.runs();
    assert_eq!(runs[0].start.as_ref().map(|e| (e.id, e.message.as_str())), Some((0, "msg 0")));
    assert_eq!(runs[0].stop.as_ref().map(|e| e.id), Some(5));
    assert!(runs[2].stop.is_none());
}

#[test]
fn reused_pid_gives_separate_runs() {
    let pid = |v: &'static str, e: Edge, ts: i64| ("lane", Some(v), e, ts);
    let p = pair(
        true,
        &[
            pid("10", S, 1),
            pid("20", S, 2),
            pid("10", P, 3),
            pid("20", P, 4),
            pid("10", S, 10),
            pid("10", P, 12),
        ],
    );
    let got: Vec<_> = p.runs().into_iter().map(|r| (r.group, r.status, r.from, r.to)).collect();
    assert_eq!(
        got,
        vec![
            (Some("10".into()), RunStatus::Closed, 1, 3),
            (Some("20".into()), RunStatus::Closed, 2, 4),
            (Some("10".into()), RunStatus::Closed, 10, 12),
        ]
    );
}

#[test]
fn anomalies_no_start_and_no_stop() {
    // A stop first, then start, work, start again (crash/restart), work.
    let p = pair(
        true,
        &[
            ("A", None, P, 1),
            ("A", None, O, 2),
            ("A", None, S, 3),
            ("A", None, O, 5),
            ("A", None, S, 8),
            ("A", None, O, 9),
        ],
    );
    assert_eq!(
        shape(&p),
        vec![
            run("A", RunStatus::NoStart, 1, 1),
            run("A", RunStatus::NoStop, 3, 5),
            run("A", RunStatus::Open, 8, 9)
        ]
    );
    // Without restarts the second start belongs to the running run.
    let p = pair(false, &[("A", None, S, 3), ("A", None, O, 5), ("A", None, S, 8), ("A", None, O, 9)]);
    assert_eq!(shape(&p), vec![run("A", RunStatus::Open, 3, 9)]);
}

#[test]
fn consecutive_starts_and_stops_are_one() {
    let p = pair(
        true,
        &[
            ("A", None, S, 1),
            ("A", None, S, 2),
            ("A", None, O, 3),
            ("A", None, P, 4),
            ("A", None, P, 5),
            ("A", None, O, 6),
            ("A", None, P, 7),
        ],
    );
    let runs = p.runs();
    assert_eq!(
        shape(&p),
        vec![run("A", RunStatus::Closed, 1, 5), run("A", RunStatus::NoStart, 7, 7)]
    );
    assert_eq!(runs[0].start.as_ref().map(|e| e.id), Some(0), "the first start is kept");
    assert_eq!(runs[0].stop.as_ref().map(|e| e.id), Some(4), "the last stop is kept");
}

#[test]
fn caps_drop_the_oldest_runs_and_ignore_extra_groups() {
    let mut p = Pairer::new(true);
    for i in 0..(MAX_RUNS_PER_LANE as i64 + 5) {
        p.feed("A", None, Edge::Start, 0, i * 10, String::new);
        p.feed("A", None, Edge::Stop, 0, i * 10 + 1, String::new);
    }
    let runs = p.runs();
    assert_eq!(runs.len(), MAX_RUNS_PER_LANE);
    assert_eq!(p.dropped_runs, 5);
    assert_eq!(runs[0].from, 50, "oldest dropped first");

    let mut p = Pairer::new(true);
    for i in 0..=MAX_GROUPS {
        p.feed("A", Some(&i.to_string()), Edge::Start, 0, 1, String::new);
    }
    p.feed("B", None, Edge::Other, 0, 2, String::new);
    assert_eq!(p.ignored_events, 2);
    assert_eq!(p.runs().len(), MAX_GROUPS);
}

#[test]
fn long_messages_are_cut() {
    let mut p = Pairer::new(true);
    p.feed("A", None, Edge::Start, 0, 1, || "x".repeat(1000));
    assert_eq!(p.runs()[0].start.as_ref().unwrap().message.chars().count(), pairing::MESSAGE_CHARS);
}

#[test]
fn feeding_in_batches_equals_feeding_at_once() {
    let events: Vec<(&str, Option<&str>, Edge, i64)> = (0..200)
        .map(|i| {
            let lane = if i % 3 == 0 { "A" } else { "B" };
            let edge = match i % 7 {
                0 => S,
                3 => P,
                5 if i % 2 == 0 => S,
                _ => O,
            };
            (lane, None, edge, i as i64)
        })
        .collect();
    let whole = pair(true, &events);
    for cut in [1, 50, 99, 150] {
        let mut p = Pairer::new(true);
        for (i, (lane, value, edge, ts)) in events.iter().enumerate() {
            p.feed(lane, *value, *edge, i as u32, *ts, || format!("msg {i}"));
            if i == cut {
                let _ = p.runs(); // reading in between changes nothing
            }
        }
        assert_eq!(p.runs(), whole.runs());
    }
}

fn rule(extra: &str) -> SpanRule {
    let cfg: SpanCfg = toml::from_str(&format!("kind='run'\nstart='start'\nstop='stop'\n{extra}")).unwrap();
    config::compile(&[cfg]).unwrap().remove(0)
}

#[test]
fn annotations_carry_status_duration_and_evidence() {
    let r = rule("label='Service'\nmax_duration='10ms'\nper='field:ProcessId'\ncolor='#abc'");
    let style = RunStyle::of(&r);
    let mk = |status, from, to| Run {
        lane: "WEB01".into(),
        group: Some("42".into()),
        start: None,
        stop: None,
        from,
        to,
        status,
    };
    let by = vec!["MachineName".to_string()];
    let a = to_annotation(&style, &mk(RunStatus::Closed, 0, 5), &by);
    assert_eq!(
        (a.label.as_str(), a.severity, a.owner.as_str(), a.annotator.as_str()),
        ("Service", Severity::Info, "profile", ANNOTATOR)
    );
    assert_eq!(a.instance.as_deref(), Some("WEB01"));
    assert_eq!(a.evidence_query.groups, vec!["WEB01"]);
    assert_eq!(a.evidence_query.group_by, by);
    assert_eq!(a.evidence_query.filters[0].field, "ProcessId");
    let info = a.run.clone().unwrap();
    assert_eq!((info.duration_ms, info.too_long, info.color.as_deref()), (5, false, Some("#abc")));
    assert_eq!(info.group.unwrap().value, "42");

    let long = to_annotation(&style, &mk(RunStatus::Closed, 0, 50), &by);
    assert!(long.run.as_ref().unwrap().too_long);
    assert_eq!(long.severity, Severity::Warning);
    let open = to_annotation(&style, &mk(RunStatus::Open, 0, 5), &by);
    assert!(open.label.contains("still running"));
    let no_stop = to_annotation(&style, &mk(RunStatus::NoStop, 0, 5), &by);
    assert!(no_stop.label.contains("no stop") && no_stop.severity == Severity::Warning);
    let no_start = to_annotation(&style, &mk(RunStatus::NoStart, 7, 7), &by);
    assert_eq!(no_start.run.as_ref().unwrap().duration_ms, 0);
    assert!(no_start.label.contains("stop without start"));

    let s = summarize(&r, [&a, &long, &open, &no_stop, &no_start], 3);
    assert_eq!(
        (s.runs, s.closed, s.open, s.no_stop, s.no_start, s.too_long, s.lanes),
        (5, 2, 1, 1, 1, 1, 1)
    );
    assert_eq!((s.total_ms, s.max_ms, s.avg_ms, s.dropped, s.anomalies()), (65, 50, 16, 3, 3));
    let empty = summarize(&r, std::iter::empty(), 0);
    assert_eq!((empty.avg_ms, empty.runs), (0, 0));
}

#[test]
fn per_parses_instance_and_fields() {
    assert_eq!(Per::parse("instance").unwrap(), Per::Instance);
    assert_eq!(Per::parse(" Lane ").unwrap(), Per::Instance);
    assert_eq!(Per::parse("field:ProcessId").unwrap(), Per::Field("ProcessId".into()));
    assert_eq!(Per::parse("field: Worker ").unwrap(), Per::Field("Worker".into()));
    for bad in ["", "field:", "machine", "field:   "] {
        assert!(Per::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn rules_are_validated() {
    let check = |extra: &str| {
        let cfg: SpanCfg = toml::from_str(&format!("start='start'\nstop='stop'\n{extra}")).unwrap();
        config::check(&cfg).into_iter().map(|i| i.field).collect::<Vec<_>>()
    };
    assert!(check("kind='run'").is_empty());
    assert_eq!(check("kind='a/b'"), vec![SpanField::Kind]);
    assert_eq!(check("kind=''"), vec![SpanField::Kind]);
    assert_eq!(check(&format!("kind='run'\nlabel='{}'", "x".repeat(201))), vec![SpanField::Label]);
    assert_eq!(check("kind='run'\nper='machine'"), vec![SpanField::Per]);
    assert_eq!(check("kind='run'\ncolor='green'"), vec![SpanField::Color]);
    assert_eq!(check("kind='run'\nmax_duration='soon'"), vec![SpanField::MaxDuration]);
    let cfg: SpanCfg = toml::from_str("kind='run'\nstart=[]\nstop=['']").unwrap();
    let fields: Vec<_> = config::check(&cfg).into_iter().map(|i| i.field).collect();
    assert_eq!(fields, vec![SpanField::Start, SpanField::Stop]);
    let many: Vec<String> = (0..17).map(|i| format!("k{i}")).collect();
    let cfg = SpanCfg {
        start: OneOrMany::Many(many),
        ..toml::from_str("kind='run'\nstart='a'\nstop='b'").unwrap()
    };
    assert!(config::check(&cfg)[0].message.contains("at most"));

    let a: SpanCfg = toml::from_str("kind='run'\nstart='a'\nstop='b'").unwrap();
    assert!(config::compile(&[a.clone(), a.clone()]).unwrap_err().to_string().contains("twice"));
    assert!(config::compile(&vec![a.clone(); MAX_SPAN_RULES + 1])
        .unwrap_err()
        .to_string()
        .contains("at most"));
    let bad = SpanCfg {
        per: "x".into(),
        ..a.clone()
    };
    assert!(config::compile(&[bad]).unwrap_err().to_string().starts_with("span \"run\": per"));
    let r = config::compile(&[a]).unwrap().remove(0);
    assert_eq!((r.label.as_str(), r.restart_on_start, r.enabled), ("run", true, true));
}

fn compiled(toml: &str) -> CompiledProfile {
    CompiledProfile::compile(Profile::from_toml(toml).unwrap()).unwrap()
}

#[test]
fn default_profile_ships_one_run_rule_on_the_builtin_markers() {
    let c = compiled("");
    assert_eq!(c.spans.len(), 1);
    let r = &c.spans[0];
    assert_eq!((r.kind.as_str(), r.label.as_str(), &r.per), ("run", "Running", &Per::Instance));
    let bit = |k: &str| 1u64 << c.markers.iter().position(|m| m.kind == k).unwrap();
    assert_eq!((r.start_mask, r.stop_mask), (bit("start"), bit("stop")));
    assert!(r.missing.is_empty());
    // A list of markers, and a marker that is not defined.
    let c = compiled("[[span]]\nkind='svc'\nstart=['start','fatal']\nstop='nope'");
    assert_eq!(c.spans[0].start_mask, bit("start") | bit("fatal"));
    assert_eq!((c.spans[0].stop_mask, c.spans[0].missing.clone()), (0, vec!["nope".to_string()]));
    // Turning runs off.
    assert!(compiled("span = []").spans.is_empty());
}

#[test]
fn span_errors_point_at_their_line() {
    let line = |text: &str| Profile::from_toml(text).unwrap_err();
    let e = line("name='x'\n\n[[span]]\nkind='run'\nstart='start'\nstop='stop'\nper='machine'\n");
    assert_eq!(e.line, Some(7), "{e}");
    assert!(e.message.contains("span \"run\"") && e.message.contains("per"), "{e}");
    // The second rule, missing a required key: its header.
    let e = line("[[span]]\nkind='a'\nstart='s'\nstop='t'\n\n[[span]]\nkind='b'\nstart='s'\n");
    assert_eq!(e.line, Some(6), "{e}");
    assert!(e.message.contains("[[span]] #2") && e.message.contains("stop"), "{e}");
    // A wrong type.
    let e = line("[[span]]\nkind='a'\nstart=5\nstop='t'\n");
    assert_eq!(e.line, Some(1), "{e}");
    // Duplicates point at the second kind.
    let e = line("[[span]]\nkind='a'\nstart='s'\nstop='t'\n[[span]]\nkind='a'\nstart='s'\nstop='t'\n");
    assert_eq!(e.line, Some(6), "{e}");
    assert!(e.message.contains("twice"));
    // A key the rule does not have falls back to its header; `span` that is not a list of tables.
    let e = line("[[span]]\nstart='s'\nstop='t'\nkind='a/b'\n");
    assert_eq!(e.line, Some(4), "{e}");
    let e = line("x = 1\nspan = 'run'\n");
    assert_eq!(e.line, Some(2), "{e}");
    let many: String = (0..17).map(|i| format!("[[span]]\nkind='k{i}'\nstart='s'\nstop='t'\n")).collect();
    let e = line(&many);
    assert!(e.message.contains("at most") && e.line == Some(65), "{e}");
    // Fine profiles pass.
    let user: toml::Table = toml::from_str("name='x'").unwrap();
    assert!(check_toml(&user, "name='x'").is_ok());
}

#[test]
fn rule_line_without_the_key_is_the_header() {
    let e = Profile::from_toml("[[span]]\nkind='run'\nstart='s'\nstop='t'\nmax_duration = 'x'\n[[marker]]\nkind='m'\n").unwrap_err();
    assert_eq!(e.line, Some(5));
    // `label` too long is reported on the header when the key is written inline elsewhere.
    let text = format!("[[span]]\nkind='run'\nstart='s'\nstop='t'\n\"label\" = '{}'\n", "x".repeat(300));
    assert_eq!(Profile::from_toml(&text).unwrap_err().line, Some(1));
}

fn draft(start: Trigger, stop: Trigger, per: &str) -> SpanDraft {
    SpanDraft {
        kind: "svc".into(),
        label: "Service".into(),
        start,
        stop,
        per: per.into(),
        max_duration: Some("8h".into()),
        restart_on_start: Some(false),
        color: Some("#3987e5".into()),
    }
}

fn markers(kinds: &[&str]) -> Trigger {
    Trigger::Markers {
        kinds: kinds.iter().map(|k| k.to_string()).collect(),
    }
}

fn pattern(p: &str) -> Trigger {
    Trigger::Pattern {
        pattern: p.into(),
        case_sensitive: false,
    }
}

#[test]
fn drafts_on_markers_plan_only_the_span() {
    let c = compiled("");
    let (errors, warnings, checked) = draft::check(
        &draft(markers(&["start"]), markers(&["stop", "fatal"]), "field:ProcessId"),
        &c.profile,
        &c.markers,
    );
    assert!(errors.is_empty() && warnings.is_empty(), "{errors:?}");
    let checked = checked.unwrap();
    assert!(checked.plan.markers.is_empty());
    assert_eq!(
        checked.plan.span,
        serde_json::json!({ "kind": "svc", "label": "Service", "start": "start", "stop": ["stop", "fatal"], "per": "field:ProcessId", "color": "#3987e5", "max_duration": "8h", "restart_on_start": false })
    );
    assert_eq!(checked.rule.max_duration_ms, Some(8 * 3_600_000));
    // The whole new list: the shipped rule kept, the draft added last; or replaced in place.
    assert_eq!(checked.plan.spans.len(), 2);
    assert_eq!(
        (checked.plan.spans[0]["kind"].as_str(), checked.plan.spans[0]["enabled"].as_bool()),
        (Some("run"), Some(true))
    );
    assert_eq!(checked.plan.spans[1], checked.plan.span);
    // Replacing the shipped rule warns.
    let (_, warnings, replaced) = draft::check(
        &SpanDraft {
            kind: "run".into(),
            ..draft(markers(&["start"]), markers(&["stop"]), "instance")
        },
        &c.profile,
        &c.markers,
    );
    assert!(warnings[0].contains("replaces"), "{warnings:?}");
    let plan = replaced.unwrap().plan;
    assert_eq!(plan.spans.len(), 1);
    assert_eq!(plan.spans[0]["label"], "Service");
}

#[test]
fn drafts_on_patterns_plan_marker_rules() {
    let c = compiled("");
    let (errors, _, checked) = draft::check(
        &draft(pattern("worker started"), pattern("worker stopped"), "instance"),
        &c.profile,
        &c.markers,
    );
    assert!(errors.is_empty(), "{errors:?}");
    let plan = checked.unwrap().plan;
    assert_eq!(plan.span["start"], "svc-start");
    assert_eq!(plan.span["stop"], "svc-stop");
    assert_eq!(plan.markers.len(), 2);
    assert_eq!(plan.markers[0]["kind"], "svc-start");
    assert_eq!(plan.markers[0]["lifecycle"], true);
    assert_eq!(plan.markers[0]["when"][0]["value"], "(?i)worker started");
    // The planned markers compile in a profile.
    let mut p = c.profile.clone();
    p.marker.push(serde_json::from_value(plan.markers[0].clone()).unwrap());
    assert!(CompiledProfile::compile(p).is_ok());
    // Case-sensitive patterns are written as typed; an existing marker of that kind is updated.
    let c = compiled("[[marker]]\nkind='svc-start'\nwhen=[{field='@message',op='contains',value='x'}]");
    let start = Trigger::Pattern {
        pattern: "Up".into(),
        case_sensitive: true,
    };
    let (_, warnings, checked) = draft::check(&draft(start, pattern("down"), "instance"), &c.profile, &c.markers);
    assert_eq!(checked.unwrap().plan.markers[0]["when"][0]["value"], "Up");
    assert!(warnings[0].contains("updates the marker rule"), "{warnings:?}");
}

#[test]
fn draft_errors_are_per_field() {
    let c = compiled("");
    let fields = |d: SpanDraft| {
        let (errors, _, checked) = draft::check(&d, &c.profile, &c.markers);
        assert!(checked.is_none());
        errors.into_iter().map(|i| i.field).collect::<Vec<_>>()
    };
    assert_eq!(
        fields(draft(markers(&["nope"]), markers(&["stop"]), "instance")),
        vec![SpanField::Start]
    );
    assert_eq!(
        fields(draft(markers(&[]), pattern("("), "instance")),
        vec![SpanField::Start, SpanField::Stop]
    );
    assert_eq!(
        fields(draft(pattern("  "), markers(&["stop"]), "x")),
        vec![SpanField::Start, SpanField::Per]
    );
    assert_eq!(
        fields(SpanDraft {
            kind: "bad kind".into(),
            max_duration: Some("eventually".into()),
            color: Some("red".into()),
            ..draft(markers(&["start"]), markers(&["stop"]), "instance")
        }),
        vec![SpanField::Kind, SpanField::Color, SpanField::MaxDuration]
    );
    // Too many marker rules for the patterns; too many span rules.
    let full: String = (0..31).map(|i| format!("[[marker]]\nkind='m{i}'\n")).collect();
    let c = compiled(&full);
    let (errors, _, _) = draft::check(&draft(pattern("a"), pattern("b"), "instance"), &c.profile, &c.markers);
    assert_eq!(errors[0].field, SpanField::General);
    assert!(errors[0].message.contains("32"));
    let spans: String = (0..16)
        .map(|i| format!("[[span]]\nkind='k{i}'\nstart='start'\nstop='stop'\n"))
        .collect();
    let c = compiled(&spans);
    let (errors, _, _) = draft::check(&draft(markers(&["start"]), markers(&["stop"]), "instance"), &c.profile, &c.markers);
    assert!(errors[0].message.contains("at most 16"), "{errors:?}");
}

#[test]
fn a_record_matching_start_and_stop_is_a_start() {
    let c = compiled(
        "[[marker]]\nkind='both'\nwhen=[{field='@message',op='contains',value='restart'}]\n[[span]]\nkind='r'\nstart='both'\nstop='both'",
    );
    let classify = draft::Classifier::of_rule(&c.spans[0]);
    let mut r = crate::model::Record::new(0);
    r.markers = c.compute_markers(
        &{
            let mut x = crate::model::Record::new(0);
            x.message = "restart".into();
            x
        },
        "s",
    );
    assert_eq!(classify.edge(&r), Edge::Start);
    r.markers = 0;
    assert_eq!(classify.edge(&r), Edge::Other);
}

#[test]
fn edge_cases_of_validation_and_ordering() {
    // A colour error points at its key; overlong marker kinds are refused; General has no key.
    let e = Profile::from_toml("[[span]]\nkind='run'\nstart='s'\nstop='t'\ncolor='red'\n").unwrap_err();
    assert_eq!(e.line, Some(5));
    let cfg = SpanCfg {
        start: OneOrMany::One("x".repeat(201)),
        ..toml::from_str("kind='run'\nstart='a'\nstop='b'").unwrap()
    };
    assert!(config::check(&cfg)[0].message.contains("overlong"));
    assert_eq!(SpanField::General.key(), None);
    // Runs starting at the same time on one lane are ordered by their field value.
    let p = pair(
        true,
        &[
            ("A", Some("2"), S, 1),
            ("A", Some("1"), S, 1),
            ("A", Some("2"), P, 3),
            ("A", Some("1"), P, 4),
        ],
    );
    let groups: Vec<Option<String>> = p.runs().into_iter().map(|r| r.group).collect();
    assert_eq!(groups, vec![Some("1".into()), Some("2".into())]);
}
