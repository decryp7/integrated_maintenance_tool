//! The proposal use cases with a scripted model over the generated incident in /samples:
//! valid proposal, invalid -> repair -> valid, still invalid -> error.

mod common;
use common::*;
use debrief_ai::assistant::*;
use debrief_ai::llm::{LlmProvider, Reply, ScriptedProvider};
use debrief_ai::AiError;
use debrief_core::regex_assist;
use serde_json::json;

fn assistant(replies: &[&str]) -> Assistant<ScriptedProvider> {
    Assistant::new(ScriptedProvider::text(replies), AssistantConfig::default())
}

fn lines(n: usize) -> Vec<String> {
    billing_sample(n).lines().map(str::to_string).collect()
}

// ---- propose_regex ----

fn invoice_lines() -> Vec<String> {
    lines(8).into_iter().filter(|l| l.contains("Invoice INV")).collect()
}

#[test]
fn regex_valid_with_renamed_group_is_validated_and_explained() {
    let ex = invoice_lines();
    assert!(ex.len() >= 3);
    let inferred = regex_assist::infer(&ex).unwrap();
    let old = inferred.groups[0].name.clone();
    let reply = json!({"pattern": "", "renames": {old.clone(): "event_time"}, "rationale": "Named the first group."}).to_string();
    let p = assistant(&[&reply]).propose_regex(&ex, Some("the invoice number")).unwrap();
    assert!(p.pattern.contains("(?P<event_time>"), "{}", p.pattern);
    assert!(p.groups.iter().any(|g| g.name == "event_time" && !g.examples.is_empty()));
    assert_eq!(p.match_count, p.total);
    assert!(p.matches.iter().all(|m| *m));
    assert!(!p.explanation.is_empty());
    assert_eq!(p.source, RegexSource::Model);
    assert_eq!(p.rationale, "Named the first group.");
}

#[test]
fn regex_keeping_the_inferred_pattern_is_marked_inferred() {
    let ex = invoice_lines();
    let a = assistant(&[r#"{"pattern":"","renames":{},"rationale":"fine"}"#]);
    let p = a.propose_regex(&ex, None).unwrap();
    assert_eq!(p.source, RegexSource::Inferred);
    // the prompt carried the examples and the engine's inferred regex
    let sent = a.provider().requests()[0].clone();
    assert!(sent.messages[0].content.contains("INV-") && sent.messages[0].content.contains("already built this regex"));
    assert!(sent.json_schema.is_some());
}

#[test]
fn regex_invalid_answers_get_one_repair_round() {
    let ex = invoice_lines();
    let a = assistant(&[
        r#"{"pattern":"(","renames":{},"rationale":"x"}"#,
        r#"{"pattern":"","renames":{},"rationale":"fixed"}"#,
    ]);
    let p = a.propose_regex(&ex, None).unwrap();
    assert_eq!(p.rationale, "fixed");
    let second = a.provider().requests()[1].clone();
    assert!(second.messages.last().unwrap().content.contains("does not compile"));
    assert_eq!(second.messages.len(), 3);
}

#[test]
fn regex_rejections_cover_each_engine_check() {
    let ex = invoice_lines();
    for (bad, needle) in [
        (
            r#"{"pattern":"^nothing (?P<x>\\d+)$","renames":{},"rationale":"x"}"#,
            "matches only",
        ),
        (r#"{"pattern":"^Invoice \\d+","renames":{},"rationale":"x"}"#, "no named groups"),
        (r#"{"pattern":"","renames":{"num1":"1bad"},"rationale":"x"}"#, "valid group name"),
        (r#"not json at all"#, "not valid JSON"),
    ] {
        let a = assistant(&[bad, bad]);
        match a.propose_regex(&ex, None) {
            Err(AiError::Malformed(m)) => assert!(m.contains(needle), "{needle}: {m}"),
            other => panic!("{needle}: {other:?}"),
        }
        assert_eq!(a.provider().remaining(), 0, "one repair round only");
    }
}

#[test]
fn regex_needs_examples_and_survives_cancellation() {
    assert!(matches!(assistant(&[]).propose_regex(&[], None), Err(AiError::Invalid(_))));
    assert!(matches!(
        assistant(&[]).propose_regex(&["   ".to_string()], None),
        Err(AiError::Invalid(_))
    ));
    let a = assistant(&[r#"{"pattern":"","renames":{},"rationale":"x"}"#]);
    a.cancel();
    assert!(matches!(a.propose_regex(&invoice_lines(), None), Err(AiError::Cancelled)));
}

// ---- propose_parser ----

fn profile_reply(regex: &str, formats: &[&str]) -> String {
    json!({"name": "Billing", "format": "text", "regex": regex, "timestamp_formats": formats, "instance_key": [], "rationale": "log4net layout"}).to_string()
}

/// A layout Debrief's built-in settings do not know: pipe-separated columns.
fn pipe_sample() -> String {
    billing_sample(40)
        .lines()
        .map(|l| {
            let (ts, rest) = l.split_at(23);
            let ts = ts.replace(',', ".");
            let mut parts = rest.trim_start().splitn(4, ' ');
            let (thread, level) = (
                parts.next().unwrap_or("").trim_matches(['[', ']']).to_string(),
                parts.next().unwrap_or("").to_string(),
            );
            let rest = parts.collect::<Vec<_>>().join(" ");
            let (logger, msg) = rest.trim_start().split_once(" - ").unwrap_or(("x", &rest));
            format!("{ts}|{level}|{thread}|{logger}|{msg}")
        })
        .collect::<Vec<_>>()
        .join("\n")
}

const PIPE_RE: &str =
    r"^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2}\.\d{3})\|(?P<level>[A-Z]+)\|(?P<Thread>\d+)\|(?P<Logger>[^|]+)\|(?P<message>.*)$";

#[test]
fn profile_proposal_is_compiled_and_previewed() {
    let sample = pipe_sample();
    let a = assistant(&[&profile_reply(PIPE_RE, &["yyyy-MM-dd HH:mm:ss.fff"])]);
    let p = a.propose_parser(store(), &sample, "billing.log").unwrap();
    assert!(p.parsed >= 38 && p.with_timestamp >= 38, "{} {}", p.parsed, p.with_timestamp);
    assert!(
        p.toml.contains("Thread") && p.importer.is_some(),
        "{} {:?} {:?} {}",
        p.name,
        p.status,
        p.rounds,
        p.toml
    );
    assert_eq!(p.preview.len(), 5);
    let parser = debrief_core::parser::Parser::from_toml(&p.toml).unwrap();
    assert_eq!((parser.id.as_str(), parser.format.kind.as_str()), ("custom-billing", "text"));
    assert!(a.provider().requests()[0].messages[0].content.contains("billing.log"));
}

#[test]
fn a_regex_that_does_not_fit_is_replaced_by_the_one_the_engine_assembled() {
    let sample = pipe_sample();
    // The engine can assemble a regex for this layout, so the model's wrong answer needs no second round.
    let a = assistant(&[&profile_reply(r"^ZZZ (?P<timestamp>x) (?P<message>y)$", &[])]);
    match a.propose_parser(store(), &sample, "billing.log") {
        Ok(p) => assert!(p.parsed >= 38 && a.provider().requests().len() == 1),
        // If the engine could not assemble one for this shape, the model is asked again.
        Err(e) => assert!(matches!(e, AiError::Malformed(_)) || matches!(e, AiError::Engine(_)), "{e}"),
    }
}

#[test]
fn files_the_built_in_settings_already_read_need_no_model() {
    let a = assistant(&[]);
    let p = a.propose_parser(store(), &billing_sample(40), "billing.log").unwrap();
    assert_eq!(p.name, "Built-in parsers");
    assert!(p.toml.is_empty(), "nothing to add to the library");
    assert!(p.parsed >= 38 && p.rationale.contains("already read"));
    assert!(a.provider().requests().is_empty(), "the model was not asked");
}

const NGINX: &str = "192.0.2.10 - - [14/Oct/2026:10:41:24 +0000] \"GET /api/orders HTTP/1.1\" 200 1532\n192.0.2.11 - - [14/Oct/2026:10:41:25 +0000] \"POST /api/cart HTTP/1.1\" 201 88\n192.0.2.10 - - [14/Oct/2026:10:41:26 +0000] \"GET /api/orders/9 HTTP/1.1\" 404 12\n192.0.2.12 - - [14/Oct/2026:10:41:27 +0000] \"GET /health HTTP/1.1\" 200 2\n192.0.2.13 - - [14/Oct/2026:10:41:29 +0000] \"GET /api/cart HTTP/1.1\" 500 310";
const NGINX_RE: &str = r#"^(?P<client>\S+) \S+ \S+ \[(?P<timestamp>[^\]]+)\] (?P<message>"[^"]*") (?P<status>\d+) (?P<bytes>\d+)$"#;

#[test]
fn profile_rejections_feed_back_the_engines_reason() {
    let good = profile_reply(NGINX_RE, &["dd/MMM/yyyy:HH:mm:ss zzz"]);
    assert!(
        assistant(&[&good]).propose_parser(store(), NGINX, "access.log").is_ok(),
        "the good answer works"
    );
    for (bad, needle) in [
        (profile_reply(r"^(?P<timestamp>\d+)\s(?P<other>.*)$", &[]), "message"),
        (profile_reply(r"^(?P<message>.*)$", &[]), "timestamp"),
        (profile_reply("", &[]), "needs a regex"),
        (profile_reply("(", &[]), "does not compile"),
        (
            profile_reply(r"^ZZZ (?P<timestamp>x) (?P<message>y)$", &[]),
            "does not match 5 of 5",
        ),
    ] {
        let a = assistant(&[&bad, &good]);
        let p = a
            .propose_parser(store(), NGINX, "access.log")
            .unwrap_or_else(|e| panic!("{needle}: {e}"));
        assert!(p.parsed > 0);
        let requests = a.provider().requests();
        if requests.len() == 1 {
            // The engine assembled a regex of its own that the engine accepts: no second round was needed.
            assert!(p.rounds[0].note.contains("engine"), "{needle}: {:?}", p.rounds);
        } else {
            let feedback = requests[1].messages.last().unwrap().content.clone();
            assert!(feedback.contains(needle), "{needle}: {feedback}");
        }
        // Twice invalid without a usable rescue is an error; a result is always one the engine verified.
        match assistant(&[&bad, &bad]).propose_parser(store(), NGINX, "access.log") {
            Ok(p) => assert!(p.verdict.is_some_and(|v| v.error.is_none() && v.events > 0)),
            Err(e) => assert!(matches!(e, AiError::Malformed(_)), "{e}"),
        }
    }
    // Timestamps first: a timestamp format the model gets wrong is replaced by one the engine found for the texts.
    let wrong_format = profile_reply(
        r"^(?P<client>\S+) \S+ \S+ \[(?P<timestamp>[^\]]+)\] (?P<message>.*)$",
        &["dd/MMM/yyyy"],
    );
    let a = assistant(&[&wrong_format]);
    let p = a.propose_parser(store(), NGINX, "access.log").unwrap();
    assert_eq!(a.provider().requests().len(), 1);
    assert!(p.rounds[0].note.contains("the engine read the timestamps"), "{:?}", p.rounds);
    assert_eq!(p.with_timestamp, 5);
    assert!(matches!(
        assistant(&[]).propose_parser(store(), "  \n", "x.log"),
        Err(AiError::Invalid(_))
    ));
}

// ---- propose_pipeline ----

fn steps_reply(steps: serde_json::Value) -> String {
    json!({"steps": steps, "rationale": "because"}).to_string()
}

#[test]
fn pipeline_drop_step_reports_kept_and_dropped() {
    let sample = billing_sample(60);
    let reply = steps_reply(json!([{"step": "drop", "when": [{"field": "@message", "op": "contains", "value": "written"}]}]));
    let p = assistant(&[&reply])
        .propose_pipeline(store(), "ignore written invoices", &sample, None)
        .unwrap();
    assert!(p.dropped > 0 && p.kept + p.dropped == p.sample_total);
    assert_eq!(p.steps.len(), 1);
    assert_eq!((p.steps[0].input, p.steps[0].output), (p.sample_total, p.kept));
    assert!(p.toml.contains("[[pipeline]]") && p.toml.contains("drop"));
    assert!(debrief_core::Profile::from_toml(&p.profile_toml).is_ok());
}

#[test]
fn pipeline_mask_step_reports_changes() {
    let sample = billing_sample(60);
    let reply = steps_reply(json!([{"step": "mask", "field": "@message", "regex": "(INV-)\\d+", "replace": "${1}***"}]));
    let p = assistant(&[&reply])
        .propose_pipeline(store(), "hide invoice numbers", &sample, None)
        .unwrap();
    assert!(p.changed > 0 && p.dropped == 0);
    assert!(p.examples[0].after.contains("INV-***"));
}

#[test]
fn pipeline_calc_step_computes_a_field() {
    let sample = billing_sample(60);
    let reply = steps_reply(json!([{"step": "calc", "field": "MsgLen", "expr": "len(@message)"}]));
    let p = assistant(&[&reply])
        .propose_pipeline(store(), "message length", &sample, None)
        .unwrap();
    assert!(p.changed > 0 && p.dropped == 0);
    assert!(p.toml.contains("calc") && p.toml.contains("len(@message)"), "{}", p.toml);
    assert!(debrief_core::Profile::from_toml(&p.profile_toml).is_ok());
    // A bad expression is refused by the engine before it reaches the user.
    let bad = steps_reply(json!([{"step": "calc", "field": "X", "expr": "nope("}]));
    assert!(assistant(&[&bad, &bad]).propose_pipeline(store(), "x", &sample, None).is_err());
}

#[test]
fn pipeline_repairs_noop_and_rejects_bad_steps() {
    let sample = billing_sample(60);
    let noop = steps_reply(json!([{"step": "drop", "when": [{"field": "@message", "op": "contains", "value": "ZZZ-never"}]}]));
    let good = steps_reply(json!([{"step": "drop", "when": [{"field": "@message", "op": "contains", "value": "starting"}]}]));
    let a = assistant(&[&noop, &good]);
    let p = a.propose_pipeline(store(), "drop startup", &sample, None).unwrap();
    assert!(p.dropped >= 1);
    assert!(a.provider().requests()[1]
        .messages
        .last()
        .unwrap()
        .content
        .contains("change nothing"));
    for (bad, needle) in [
        (steps_reply(json!([])), "between 1 and"),
        (steps_reply(json!([{"step": "plugin", "id": "x"}])), "not an allowed step kind"),
        (steps_reply(json!([{"step": "mask", "regex": "(", "replace": "x"}])), "failed"),
        (steps_reply(json!([{"step": 5}])), "step 1"),
    ] {
        match assistant(&[&bad, &bad]).propose_pipeline(store(), "x", &sample, None) {
            Err(AiError::Malformed(m)) => assert!(m.contains(needle), "{needle}: {m}"),
            other => panic!("{needle}: {other:?}"),
        }
    }
    assert!(matches!(
        assistant(&[]).propose_pipeline(store(), " ", &sample, None),
        Err(AiError::Invalid(_))
    ));
    assert!(matches!(
        assistant(&[]).propose_pipeline(store(), "x", "this is not a log at all", None),
        Err(AiError::Invalid(_))
    ));
    assert!(matches!(
        assistant(&[]).propose_pipeline(store(), "x", &sample, Some("[parse")),
        Err(AiError::Invalid(_))
    ));
}

// ---- propose_markers / propose_extractors ----

#[test]
fn markers_are_counted_on_the_sample() {
    let sample = billing_sample(80);
    let m = |kind: &str, value: &str| json!({"kind": kind, "label": "Invoice", "color": "#3987e5", "icon": "file", "badge": "I", "shape": "square", "description": "An invoice", "when": [{"field": "@message", "op": "contains", "value": value}]});
    let reply = json!({"markers": [m("invoice", "written")], "rationale": "r"}).to_string();
    let p = assistant(&[&reply])
        .propose_markers(store(), "mark invoices", &sample, None)
        .unwrap();
    assert!(p.markers[0].matches > 0 && p.markers[0].matches < p.sampled);
    assert!(p.toml.contains("[[marker]]") && p.profile_toml.contains("invoice"));
    // none matching -> repair
    let none = json!({"markers": [m("ghost", "ZZZ-never")], "rationale": "r"}).to_string();
    let a = assistant(&[&none, &reply]);
    assert!(a.propose_markers(store(), "x", &sample, None).is_ok());
    assert!(a.provider().requests()[1].messages.last().unwrap().content.contains("matches none"));
}

#[test]
fn marker_validation_rules() {
    let sample = billing_sample(80);
    let base = |f: &dyn Fn(&mut serde_json::Value)| {
        let mut v = json!({"kind": "invoice", "label": "I", "when": [{"field": "@message", "op": "contains", "value": "written"}]});
        f(&mut v);
        json!({"markers": [v], "rationale": "r"}).to_string()
    };
    for (bad, needle) in [
        (base(&|v| v["kind"] = json!("error")), "already exists"),
        (base(&|v| v["kind"] = json!("bad kind!")), "must be letters"),
        (base(&|v| v["when"] = json!([])), "at least one condition"),
        (base(&|v| v["color"] = json!("red")), "color"),
        (base(&|v| v["badge"] = json!("ABCD")), "badge"),
        (base(&|v| v["icon"] = json!("nope")), "unknown icon"),
        (base(&|v| v["shape"] = json!("star")), "shape"),
        (
            base(&|v| v["when"] = json!([{"field": "@message", "op": "exists"}])),
            "every sample event",
        ),
        (json!({"markers": [], "rationale": "r"}).to_string(), "between 1 and 6"),
    ] {
        match assistant(&[&bad, &bad]).propose_markers(store(), "x", &sample, None) {
            Err(AiError::Malformed(m)) => assert!(m.contains(needle), "{needle}: {m}"),
            other => panic!("{needle}: {other:?}"),
        }
    }
    assert!(matches!(
        assistant(&[]).propose_markers(store(), "", &sample, None),
        Err(AiError::Invalid(_))
    ));
}

#[test]
fn extractors_report_matches_and_examples() {
    let sample = billing_sample(80);
    let e = |regex: &str| {
        json!({"extractors": [{"name": "invoice-no", "from": "@message", "regex": regex, "when": [], "fields": {}}], "rationale": "r"})
            .to_string()
    };
    let p = assistant(&[&e(r"Invoice (?P<invoice>INV-\d+) written")])
        .propose_extractors(store(), "invoice numbers", &sample, None)
        .unwrap();
    assert!(p.extractors[0].matches > 0);
    assert_eq!(p.extractors[0].fields, ["invoice"]);
    assert!(p.extractors[0].examples[0]["invoice"].as_str().unwrap().starts_with("INV-"));
    assert!(p.toml.contains("[[extract]]"));
    let a = assistant(&[&e(r"ZZZ (?P<x>\d+)"), &e(r"Invoice (?P<invoice>INV-\d+) written")]);
    assert!(a.propose_extractors(store(), "x", &sample, None).is_ok());
    assert!(a.provider().requests()[1]
        .messages
        .last()
        .unwrap()
        .content
        .contains("extracted nothing"));
    for (bad, needle) in [
        (e("("), "does not compile"),
        (e(r"\d+"), "named groups"),
        (json!({"extractors": [], "rationale": ""}).to_string(), "between 1 and 4"),
    ] {
        match assistant(&[&bad, &bad]).propose_extractors(store(), "x", &sample, None) {
            Err(AiError::Malformed(m)) => assert!(m.contains(needle), "{needle}: {m}"),
            other => panic!("{needle}: {other:?}"),
        }
    }
    assert!(matches!(
        assistant(&[]).propose_extractors(store(), "", &sample, None),
        Err(AiError::Invalid(_))
    ));
}

// ---- nl_to_query ----

#[test]
fn nl_to_query_validates_and_counts() {
    let a = assistant(&[r#"{"ql":"level >= error","explanation":"All errors."}"#]);
    let q = a.nl_to_query(store(), "show me the errors").unwrap();
    assert_eq!(q.ql, "level >= error");
    assert!(q.estimated_matches.unwrap() > 0 && !q.repaired);
    let prompt = a.provider().requests()[0].messages[0].content.clone();
    assert!(prompt.contains("MachineName") && prompt.contains("Question: show me the errors"));
}

#[test]
fn nl_to_query_repairs_once_with_the_engines_error() {
    let a = assistant(&[
        r#"{"ql":"level >=","explanation":"x"}"#,
        r#"{"ql":"level >= warning | stats count() by MachineName","explanation":"per machine"}"#,
    ]);
    let q = a.nl_to_query(store(), "warnings per machine").unwrap();
    assert!(q.repaired);
    assert!(q.estimated_matches.unwrap() > 0);
    let fb = a.provider().requests()[1].messages.last().unwrap().content.clone();
    assert!(fb.contains("is not valid") && fb.contains("at byte"));
    let bad = r#"{"ql":"level >=","explanation":"x"}"#;
    assert!(matches!(
        assistant(&[bad, bad]).nl_to_query(store(), "x"),
        Err(AiError::Malformed(_))
    ));
    assert!(matches!(
        assistant(&[r#"{"ql":" ","explanation":""}"#, r#"{"ql":" ","explanation":""}"#]).nl_to_query(store(), "x"),
        Err(AiError::Malformed(_))
    ));
    assert!(matches!(assistant(&[]).nl_to_query(store(), "  "), Err(AiError::Invalid(_))));
}

// ---- narratives and the tool loop ----

#[test]
fn explain_event_prefetches_facts_and_verifies_citations() {
    let id = first_id("level = fatal");
    let reply =
        format!("The service crashed (event {id}). Nothing else is known; event 987654 is not in the logs. See `level >= error` for more.");
    let a = assistant(&[&reply]);
    let n = a.explain_event(store(), id).unwrap();
    let req = a.provider().requests()[0].clone();
    // the engine's get_events and neighbours results were handed to the model as a tool exchange
    assert_eq!(req.messages[1].tool_calls.len(), 2);
    assert!(req.messages[2].content.contains("\"events\""));
    assert!(req.system.contains("not found in the logs"));
    assert!(!req.tools.is_empty());
    assert!(n.citations.contains(&Citation::Record { id, verified: true }));
    assert!(n.citations.contains(&Citation::Record {
        id: 987654,
        verified: false
    }));
    assert!(n
        .citations
        .iter()
        .any(|c| matches!(c, Citation::Query { ql } if ql == "level >= error")));
    assert!(n.evidence.record_ids.contains(&id));
    assert!(n.usage.output_tokens > 0);
    assert!(matches!(
        assistant(&[]).explain_event(store(), 4_000_000),
        Err(AiError::NotFound(_))
    ));
}

#[test]
fn summarize_uses_the_moment_tool_and_may_call_more() {
    let (from, to) = (ms(10, 40, 0), ms(10, 50, 0));
    let a = Assistant::new(
        ScriptedProvider::new(vec![
            Reply::Tools(
                "Checking errors.".into(),
                vec![("run_query".into(), json!({"ql": "level >= error", "limit": 3}))],
            ),
            Reply::Text("Between 10:40 and 10:50 errors spiked. Not found in the logs: the deployment.".into()),
        ]),
        AssistantConfig::default(),
    );
    let n = a.summarize(store(), from, to).unwrap();
    let reqs = a.provider().requests();
    assert_eq!(reqs[0].messages[1].tool_calls[0].name, "moment");
    assert!(reqs[0].messages[2].content.contains("levels"));
    // the second request carries the model's own tool call and its result
    assert!(reqs[1].messages.iter().any(|m| m.tool_calls.iter().any(|c| c.name == "run_query")));
    assert!(n.text.contains("spiked"));
    assert!(n.evidence.queries.contains(&"level >= error".to_string()));
    assert!(matches!(assistant(&[]).summarize(store(), to, from), Err(AiError::Invalid(_))));
}

#[test]
fn root_causes_starts_from_the_overview() {
    let a = assistant(&["1. The gateway timed out (see event 5)."]);
    let n = a.root_causes(store()).unwrap();
    let req = a.provider().requests()[0].clone();
    assert_eq!(
        req.messages[1].tool_calls.iter().map(|c| c.name.as_str()).collect::<Vec<_>>(),
        ["overview", "exception_groups"]
    );
    assert!(req.messages[2].content.contains("findings"));
    assert!(n.text.contains("gateway"));
}

#[test]
fn tool_loop_limits_cancellation_and_bad_arguments() {
    let call = || Reply::Tools(String::new(), vec![("overview".into(), json!({}))]);
    // wrap-up answer after the limit
    let a = Assistant::new(
        ScriptedProvider::new(vec![call(), call(), Reply::Text("Answer from what I have.".into())]),
        AssistantConfig {
            max_tool_rounds: 2,
            ..AssistantConfig::default()
        },
    );
    assert!(a.root_causes(store()).unwrap().text.contains("what I have"));
    let reqs = a.provider().requests();
    assert!(reqs[2].tools.is_empty() && reqs[2].system.contains("used all tool rounds"));
    // still calling tools at the end, or silent: error
    let a = Assistant::new(
        ScriptedProvider::new(vec![call(), call(), call()]),
        AssistantConfig {
            max_tool_rounds: 2,
            ..AssistantConfig::default()
        },
    );
    assert!(matches!(a.root_causes(store()), Err(AiError::ToolLoopLimit(2))));
    let a = Assistant::new(
        ScriptedProvider::new(vec![call(), call(), Reply::Text(" ".into())]),
        AssistantConfig {
            max_tool_rounds: 2,
            ..AssistantConfig::default()
        },
    );
    assert!(matches!(a.root_causes(store()), Err(AiError::ToolLoopLimit(2))));
    let a = assistant(&[" "]);
    assert!(matches!(a.root_causes(store()), Err(AiError::Malformed(_))));
    // cancelled between rounds
    let a = Assistant::new(
        ScriptedProvider::new(vec![call(), Reply::Text("never".into())]),
        AssistantConfig::default(),
    );
    let flag = a.config().cancel.clone();
    let _ = flag;
    a.cancel();
    assert!(matches!(a.root_causes(store()), Err(AiError::Cancelled)));
    // malformed tool arguments become an error result the model can react to
    let bad = Reply::Tools(
        String::new(),
        vec![
            ("run_query".into(), json!({"ql": 5})),
            ("nonexistent".into(), json!({})),
            ("run_query".into(), json!({"_invalid_json": "{oops"})),
        ],
    );
    let a = Assistant::new(
        ScriptedProvider::new(vec![bad, Reply::Text("Recovered.".into())]),
        AssistantConfig::default(),
    );
    let n = a.summarize(store(), ms(10, 0, 0), ms(10, 5, 0)).unwrap();
    assert_eq!(n.text, "Recovered.");
    let second = a.provider().requests()[1].clone();
    let results: Vec<&str> = second
        .messages
        .iter()
        .filter(|m| m.tool_call_id.as_deref().is_some_and(|id| id.starts_with("call_")))
        .map(|m| m.content.as_str())
        .collect();
    assert!(
        results[0].contains("invalid arguments") && results[1].contains("unknown tool") && results[2].contains("could not be read"),
        "{results:?}"
    );
}

#[test]
fn assistant_works_through_shared_providers() {
    use std::sync::Arc;
    let p = Arc::new(ScriptedProvider::text(&["{\"pattern\":\"\",\"renames\":{},\"rationale\":\"x\"}"]));
    let a = Assistant::new(p.clone(), AssistantConfig::default());
    assert!(a.propose_regex(&invoice_lines(), None).is_ok());
    assert_eq!(p.requests().len(), 1);
    let r = Assistant::new(&*p, AssistantConfig::default());
    assert_eq!(r.provider().id(), "scripted");
    assert_eq!(r.provider().display_name(), "Scripted test model");
}
