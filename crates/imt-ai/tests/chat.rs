//! Chat sessions with a scripted model over the incident in /samples: tool calls, citations, proposals
//! that are never applied, context trimming, cancellation, persistence and starter prompts.

mod common;
use common::*;
use imt_ai::assistant::{Assistant, AssistantConfig, Citation, Proposal};
use imt_ai::chat::*;
use imt_ai::llm::{LlmProvider, Reply, Role, ScriptedProvider};
use imt_ai::AiError;
use serde_json::{json, Value};
use std::sync::Arc;

fn assistant(replies: Vec<Reply>) -> Assistant<ScriptedProvider> {
    Assistant::new(ScriptedProvider::new(replies), AssistantConfig::default())
}

fn text(s: &str) -> Reply {
    Reply::Text(s.to_string())
}

fn tools(name: &str, args: Value) -> Reply {
    Reply::Tools(String::new(), vec![(name.to_string(), args)])
}

fn send(session: &mut ChatSession, a: &Assistant<ScriptedProvider>, msg: &str, ctx: &ChatContext) -> (imt_ai::Result<()>, Vec<Envelope>) {
    let env = ChatEnv::new(a, store().into());
    let mut events = Vec::new();
    let r = session.send(&env, msg, ctx, &mut |e| events.push(e.clone()));
    (r, events)
}

fn kinds(events: &[Envelope]) -> Vec<&'static str> {
    events
        .iter()
        .map(|e| match &e.event {
            ChatEvent::Delta { .. } => "delta",
            ChatEvent::ToolStarted { .. } => "toolStarted",
            ChatEvent::ToolFinished { .. } => "toolFinished",
            ChatEvent::Proposal { .. } => "proposal",
            ChatEvent::LinkSuggestion { .. } => "link",
            ChatEvent::AgentApproval { .. } => "approval",
            ChatEvent::AgentStep { .. } => "step",
            ChatEvent::AgentPaused { .. } => "paused",
            ChatEvent::ResearchOffer { .. } => "researchOffer",
            ChatEvent::Done { .. } => "done",
            ChatEvent::Error { .. } => "error",
        })
        .collect()
}

#[test]
fn a_multi_turn_session_with_tool_calls_streams_numbered_events() {
    let fatal = first_id("level = fatal and MachineName = WEB02");
    let a = assistant(vec![
        tools("run_query", json!({"ql": "level = fatal and MachineName = WEB02"})),
        text(&format!(
            "The service crashed: see event {fatal} and `level = fatal and MachineName = WEB02`."
        )),
        text("It was restarted afterwards."),
    ]);
    let mut s = ChatSession::new("c1");
    let (r, ev1) = send(&mut s, &a, "What was the fatal error?", &ChatContext::default());
    r.unwrap();
    let k = kinds(&ev1);
    assert_eq!(k[0], "toolStarted");
    assert_eq!(k[1], "toolFinished");
    assert!(k[2..k.len() - 1].iter().all(|x| *x == "delta") && *k.last().unwrap() == "done");
    assert_eq!(
        ev1.iter().map(|e| e.seq).collect::<Vec<_>>(),
        (1..=ev1.len() as u64).collect::<Vec<_>>()
    );
    let ChatEvent::ToolFinished {
        name,
        summary,
        error,
        evidence,
    } = &ev1[1].event
    else {
        panic!()
    };
    assert_eq!((name.as_str(), summary.as_str(), *error), ("run_query", "1 events", false));
    assert!(evidence.record_ids.contains(&fatal) && evidence.queries == ["level = fatal and MachineName = WEB02"]);
    let ChatEvent::Done {
        text, citations, usage, ..
    } = &ev1.last().unwrap().event
    else {
        panic!()
    };
    assert!(text.contains("crashed"));
    assert!(citations.contains(&Citation::Record { id: fatal, verified: true }));
    assert!(citations.contains(&Citation::Query {
        ql: "level = fatal and MachineName = WEB02".into()
    }));
    assert!(usage.output_tokens > 0);
    assert_eq!(s.title(), "What was the fatal error?");

    // the second turn continues the numbering and sees the first turn
    let (r, ev2) = send(&mut s, &a, "And then?", &ChatContext::default());
    r.unwrap();
    assert_eq!(ev2[0].seq, ev1.last().unwrap().seq + 1);
    let req = a.provider().requests().last().unwrap().clone();
    let roles: Vec<Role> = req.messages.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant, Role::User]);
    assert!(req.messages[1].content.contains("crashed") && req.messages[2].content.contains("Question: And then?"));
    assert!(req.system.contains("untrusted_web_content") && req.system.contains("not found in the logs"));
    assert_eq!(s.turns().len(), 2);
}

#[test]
fn events_can_be_resumed_by_sequence_number() {
    let a = assistant(vec![text("First answer."), text("Second answer.")]);
    let mut s = ChatSession::new("c");
    let (_, ev) = send(&mut s, &a, "one", &ChatContext::default());
    let cut = ev[1].seq;
    let missed = s.events_since(cut).unwrap();
    assert_eq!(missed.len(), ev.len() - 2);
    assert_eq!(missed[0].seq, cut + 1);
    assert!(s.events_since(ev.last().unwrap().seq).unwrap().is_empty());
    assert_eq!(s.events_since(0).unwrap().len(), ev.len());
    // the serialised event carries its sequence number and type
    let j = serde_json::to_value(&ev[0]).unwrap();
    assert_eq!((j["seq"].clone(), j["type"].clone()), (json!(1), json!("delta")));
    // a very old position cannot be resumed
    for i in 0..(REPLAY_LIMIT / 2 + 10) {
        let a = assistant(vec![text("x")]);
        send(&mut s, &a, &format!("q{i}"), &ChatContext::default()).0.unwrap();
    }
    assert!(s.events_since(0).is_none());
}

#[test]
fn proposals_are_emitted_validated_and_never_applied() {
    let before = store().summary().total;
    let profile_before = store().profile().name.clone();
    // the chat model calls propose_query; the nested natural-language-to-query call is the second scripted reply
    let a = assistant(vec![
        tools("propose_query", json!({"question": "show errors"})),
        text(r#"{"ql":"level >= error","explanation":"All errors."}"#),
        text("I prepared a query for you; use Apply to run it."),
    ]);
    let mut s = ChatSession::new("p");
    let (r, ev) = send(&mut s, &a, "Give me a query for errors", &ChatContext::default());
    r.unwrap();
    let proposal = ev
        .iter()
        .find_map(|e| {
            if let ChatEvent::Proposal { proposal } = &e.event {
                Some(proposal.clone())
            } else {
                None
            }
        })
        .expect("a proposal event");
    let Proposal::Query(q) = proposal else { panic!() };
    assert_eq!(q.ql, "level >= error");
    assert!(q.estimated_matches.unwrap() > 0);
    let k = kinds(&ev);
    assert!(k.iter().position(|x| *x == "toolFinished") < k.iter().position(|x| *x == "proposal"));
    // the tool result told the model nothing was changed
    let tool_result = a.provider().requests()[2]
        .messages
        .iter()
        .find(|m| m.role == Role::Tool)
        .unwrap()
        .content
        .clone();
    assert!(tool_result.contains("nothing was changed") && tool_result.contains("must review and apply"));
    assert_eq!((store().summary().total, store().profile().name.clone()), (before, profile_before));
    // the proposal is stored with the turn for the project file
    assert_eq!(s.turns()[0].proposals[0]["kind"], "query");
}

#[test]
fn pipeline_proposals_use_a_sample_from_the_selected_event() {
    let id = first_id("Invoice");
    let steps =
        json!({"steps": [{"step": "drop", "when": [{"field": "@message", "op": "contains", "value": "written"}]}], "rationale": "noise"})
            .to_string();
    let a = assistant(vec![
        tools("propose_pipeline", json!({"goal": "drop written invoices"})),
        text(&steps),
        text("Done: review the pipeline proposal."),
    ]);
    let mut s = ChatSession::new("pl");
    let ctx = ChatContext {
        selected_ids: vec![id],
        ..Default::default()
    };
    let (r, ev) = send(&mut s, &a, "Hide the noisy invoice lines", &ctx);
    r.unwrap();
    let Some(Proposal::Pipeline(p)) = ev.iter().find_map(|e| {
        if let ChatEvent::Proposal { proposal } = &e.event {
            Some(proposal.clone())
        } else {
            None
        }
    }) else {
        panic!("{:?}", kinds(&ev))
    };
    assert!(p.dropped > 0 && p.toml.contains("[[pipeline]]"));
    // without a selected event the tool explains what is missing
    let a = assistant(vec![
        tools("propose_pipeline", json!({"goal": "x"})),
        text("Please select an event."),
    ]);
    let (_, ev) = send(&mut ChatSession::new("x"), &a, "hide noise", &ChatContext::default());
    assert!(ev
        .iter()
        .any(|e| matches!(&e.event, ChatEvent::ToolFinished { error: true, summary, .. } if summary.contains("select an event"))));
}

#[test]
fn every_propose_tool_is_reachable_from_chat() {
    let id = first_id("Invoice");
    let ctx = ChatContext {
        selected_ids: vec![id],
        ..Default::default()
    };
    let marker =
        json!({"markers": [{"kind": "inv", "label": "Invoice", "when": [{"field": "@message", "op": "contains", "value": "written"}]}], "rationale": "r"})
            .to_string();
    let extract = json!({"extractors": [{"name": "inv-no", "from": "@message", "regex": "Invoice (?P<invoice>INV-\\d+) written", "when": [], "fields": {}}], "rationale": "r"}).to_string();
    let profile = json!({"name": "Billing", "format": "text", "regex": r"^(?P<timestamp>\d{4}-\d{2}-\d{2} \d{2}:\d{2}:\d{2},\d{3})\s+\[(?P<Thread>[^\]]+)\]\s+(?P<level>[A-Za-z]+)\s+(?P<Logger>\S+)\s+-\s(?P<message>.*)$", "timestamp_formats": [], "instance_key": [], "rationale": "r"}).to_string();
    let regex = r#"{"pattern":"","renames":{},"rationale":"fine"}"#;
    let a = assistant(vec![
        Reply::Tools(
            String::new(),
            vec![
                ("propose_markers".into(), json!({"goal": "mark invoices"})),
                ("propose_extractors".into(), json!({"goal": "invoice numbers"})),
                ("propose_profile".into(), json!({})),
                (
                    "propose_regex".into(),
                    json!({"example_lines": ["Invoice INV-1 written", "Invoice INV-22 written", "Invoice INV-333 written"], "intent": "the number"}),
                ),
                ("propose_regex".into(), json!({"wrong": true})),
            ],
        ),
        text(&marker),
        text(&extract),
        text(&profile),
        text(regex),
        text("Four proposals are ready."),
    ]);
    let mut s = ChatSession::new("all");
    let (r, ev) = send(&mut s, &a, "set everything up", &ctx);
    r.unwrap();
    let kinds_seen: Vec<String> = ev
        .iter()
        .filter_map(|e| {
            if let ChatEvent::Proposal { proposal } = &e.event {
                serde_json::to_value(proposal).ok().map(|v| v["kind"].as_str().unwrap().to_string())
            } else {
                None
            }
        })
        .collect();
    assert_eq!(kinds_seen, ["markers", "extractors", "profile", "regex"]);
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(&e.event, ChatEvent::ToolFinished { error: true, .. }))
            .count(),
        1
    );
}

#[test]
fn malformed_tool_arguments_are_reported_to_the_model_and_the_chat_recovers() {
    let a = assistant(vec![
        Reply::Tools(
            String::new(),
            vec![
                ("run_query".into(), json!({"ql": 5})),
                ("no_such_tool".into(), json!({})),
                ("get_events".into(), json!({"ids": "x"})),
            ],
        ),
        text("Sorry, let me answer without it: not found in the logs."),
    ]);
    let mut s = ChatSession::new("m");
    let (r, ev) = send(&mut s, &a, "anything", &ChatContext::default());
    r.unwrap();
    assert_eq!(
        ev.iter()
            .filter(|e| matches!(&e.event, ChatEvent::ToolFinished { error: true, .. }))
            .count(),
        3
    );
    assert!(s.turns()[0].assistant.contains("not found in the logs"));
}

#[test]
fn a_tool_loop_that_never_ends_is_a_clear_error_event() {
    let call = || tools("overview", json!({}));
    let a = Assistant::new(
        ScriptedProvider::new(vec![call(), call(), call(), call()]),
        AssistantConfig {
            max_tool_rounds: 2,
            ..AssistantConfig::default()
        },
    );
    let mut s = ChatSession::new("l");
    let (r, ev) = send(&mut s, &a, "loop", &ChatContext::default());
    assert!(matches!(r, Err(AiError::ToolLoopLimit(2))));
    assert!(matches!(&ev.last().unwrap().event, ChatEvent::Error { message, cancelled: false } if message.contains("tool rounds")));
    assert!(s.turns().is_empty());
    assert!(matches!(
        send(&mut s, &assistant(vec![]), "  ", &ChatContext::default()).0,
        Err(AiError::Invalid(_))
    ));
}

#[test]
fn cancel_mid_stream_keeps_the_partial_answer() {
    let long = "This answer is long enough to be interrupted while it is still being written out. ".repeat(4);
    let a = assistant(vec![text(&long)]);
    let mut s = ChatSession::new("x");
    let env = ChatEnv::new(&a, store().into());
    let mut events = Vec::new();
    let r = s.send(&env, "tell me everything", &ChatContext::default(), &mut |e| {
        events.push(e.clone());
        if matches!(e.event, ChatEvent::Delta { .. }) && events.len() == 3 {
            a.cancel();
        }
    });
    assert!(matches!(r, Err(AiError::Cancelled)));
    assert!(matches!(&events.last().unwrap().event, ChatEvent::Error { cancelled: true, .. }));
    let t = &s.turns()[0];
    assert!(t.cancelled && !t.assistant.is_empty() && t.assistant.len() < long.len());
    // the next question works: the flag was reset
    let a2 = assistant(vec![text("Fine.")]);
    send(&mut s, &a2, "again", &ChatContext::default()).0.unwrap();
    assert_eq!(s.turns().len(), 2);
}

#[test]
fn history_is_trimmed_to_the_context_window() {
    let turns: Vec<TurnRecord> = (0..30)
        .map(|i| TurnRecord {
            user: format!("question number {i} about the payment gateway timeouts"),
            assistant: format!("answer number {i}: the gateway timed out after thirty seconds, see event {i}"),
            citations: vec![],
            proposals: vec![],
            web: vec![],
            cancelled: false,
            usage: Default::default(),
        })
        .collect();
    let mut s = ChatSession::from_record(ChatRecord {
        id: "big".into(),
        title: "t".into(),
        turns,
    });
    let a = Assistant::new(
        ScriptedProvider::new(vec![text("ok")]).with_context(2048),
        AssistantConfig {
            max_tokens: 256,
            ..AssistantConfig::default()
        },
    );
    send(&mut s, &a, "latest question", &ChatContext::default()).0.unwrap();
    let req = a.provider().requests()[0].clone();
    assert!(
        req.messages.len() < 20,
        "only the newest turns are kept, got {}",
        req.messages.len()
    );
    assert!(req.messages.len() >= 3);
    assert!(req.system.contains("Earlier in this conversation") && req.system.contains("question number 28"));
    let total: usize = req.messages.iter().map(|m| m.content.len()).sum::<usize>() + req.system.len();
    assert!(total < (2048 - 256) * 4, "prompt {total} chars");
    assert!(req.messages.last().unwrap().content.ends_with("Question: latest question"));
    assert_eq!(s.turns().len(), 31, "the full history is kept for export");
    // tools shrink on small windows
    assert!(req.tools.iter().all(|t| t.name != "neighbours" && t.name != "field_stats"));
    assert!(req.tools.iter().any(|t| t.name == "propose_regex"));
    let big = Assistant::new(
        ScriptedProvider::new(vec![text("ok")]).with_context(16384),
        AssistantConfig::default(),
    );
    send(&mut ChatSession::new("b"), &big, "q", &ChatContext::default()).0.unwrap();
    assert!(big.provider().requests()[0].tools.iter().any(|t| t.name == "neighbours"));
}

#[test]
fn a_context_overflow_is_retried_with_a_tighter_budget() {
    let turns: Vec<TurnRecord> = (0..4)
        .map(|i| TurnRecord {
            user: format!("old question {i}"),
            assistant: format!("old answer {i}"),
            citations: vec![],
            proposals: vec![],
            web: vec![],
            cancelled: false,
            usage: Default::default(),
        })
        .collect();
    let mut s = ChatSession::from_record(ChatRecord {
        id: "o".into(),
        title: "t".into(),
        turns,
    });
    let a = assistant(vec![
        Reply::Fail(AiError::ContextOverflow { prompt: 9000, limit: 8192 }),
        text("Short answer."),
    ]);
    send(&mut s, &a, "new question", &ChatContext::default()).0.unwrap();
    let reqs = a.provider().requests();
    assert!(
        reqs[0].messages.len() > 1 && reqs[1].messages.len() == 1,
        "the retry dropped the history"
    );
    let a = assistant(vec![
        Reply::Fail(AiError::ContextOverflow { prompt: 9000, limit: 8192 }),
        Reply::Fail(AiError::ContextOverflow { prompt: 9000, limit: 8192 }),
    ]);
    let (r, ev) = send(&mut s, &a, "again", &ChatContext::default());
    assert!(matches!(r, Err(AiError::ContextOverflow { .. })));
    assert!(matches!(&ev.last().unwrap().event, ChatEvent::Error { .. }));
}

#[test]
fn the_context_is_validated_and_the_engine_fetches_the_details() {
    let id = first_id("level = fatal");
    let ctx = ChatContext {
        query: Some("level >= error".into()),
        time_range: Some((ms(10, 40, 0), ms(10, 50, 0))),
        selected_ids: vec![id, 4_000_000, id],
        focused_instance: Some("WEB02\u{7}".into()),
        open_view: Some("timeline".into()),
    };
    let a = assistant(vec![text("ok")]);
    send(&mut ChatSession::new("c"), &a, "why?", &ctx).0.unwrap();
    let q = a.provider().requests()[0].messages.last().unwrap().content.clone();
    assert!(
        q.contains("timeline view")
            && q.contains("Active filter: `level >= error`")
            && q.contains("2026-10-14 10:40:00 to 2026-10-14 10:50:00")
    );
    assert!(q.contains("Focused instance: WEB02") && !q.contains('\u{7}'));
    assert!(q.contains(&format!("#{id} [Fatal]")), "{q}");
    assert!(q.contains("do not exist"), "{q}");

    // bad values from the UI are dropped with a note, never passed on as data
    let bad = ChatContext {
        query: Some("level >=".into()),
        time_range: Some((5, 5)),
        selected_ids: (0..40).collect(),
        ..Default::default()
    };
    let a = assistant(vec![text("ok")]);
    send(&mut ChatSession::new("c"), &a, "why?", &bad).0.unwrap();
    let q = a.provider().requests()[0].messages.last().unwrap().content.clone();
    assert!(
        q.contains("filter was ignored") && q.contains("time range was ignored") && q.contains("first 20 selected"),
        "{q}"
    );
    assert!(!q.contains("Active filter"));
    assert!(q.contains("and 15 more"));
    let r = resolve(
        &ChatContext::default(),
        store(),
        &imt_ai::assistant::EngineTools::new(store(), imt_ai::assistant::ToolLimits::for_context(4096)),
        500,
    );
    assert!(r.text.is_empty() && r.notes.is_empty());
}

#[test]
fn chats_survive_the_project_file_and_damaged_records_give_none() {
    let a = assistant(vec![text("Answer with event 3."), text("Second.")]);
    let mut sessions = ChatSessions::new();
    let id = sessions.create().id().to_string();
    {
        let s = sessions.get_mut(&id).unwrap();
        send(s, &a, "first", &ChatContext::default()).0.unwrap();
    }
    let other = sessions.create().id().to_string();
    assert_ne!(id, other);
    let value = sessions.to_record().to_value();
    assert_eq!(value["version"], 1);
    let (rec, warn) = ChatHistoryRecord::from_value(Some(&value));
    assert!(warn.is_none());
    let mut restored = ChatSessions::from_record(rec);
    assert_eq!(restored.list().len(), 2);
    let s = restored.get_mut(&id).unwrap();
    assert_eq!(s.turns()[0].assistant, "Answer with event 3.");
    // numbering of new chats continues
    let third = restored.create().id().to_string();
    assert!(third != id && third != other);
    assert!(restored.remove(&third) && !restored.remove(&third));
    // damaged or newer records become empty histories with a warning
    assert!(ChatHistoryRecord::from_value(Some(&json!({"version": "x"})))
        .1
        .unwrap()
        .contains("damaged"));
    assert!(ChatHistoryRecord::from_value(Some(&json!({"version": 9, "sessions": []})))
        .1
        .unwrap()
        .contains("newer"));
    // unknown extra fields from a future version of the same format are ignored
    let (rec, w) = ChatHistoryRecord::from_value(Some(
        &json!({"version": 1, "sessions": [{"id": "a", "title": "t", "turns": [{"user": "u", "assistant": "a", "future": 1}]}], "extra": true}),
    ));
    assert!(w.is_none() && rec.sessions[0].turns[0].citations.is_empty());
    sessions.get_mut(&id).unwrap().rename("  A better title  ");
    assert_eq!(sessions.list()[0].1, "A better title");
}

#[test]
fn a_chat_exports_as_markdown() {
    let a = assistant(vec![text("The crash is event 5. Try `level = fatal`.")]);
    let mut s = ChatSession::new("e");
    send(&mut s, &a, "What crashed?", &ChatContext::default()).0.unwrap();
    let mut rec = s.to_record();
    rec.turns[0].web.push(imt_ai::assistant::WebSource {
        title: "Docs [x]".into(),
        url: "https://docs.example.org/a".into(),
    });
    rec.turns[0].proposals.push(json!({"kind": "pipeline"}));
    let mut cancelled = rec.turns[0].clone();
    cancelled.cancelled = true;
    rec.turns.push(cancelled);
    let md = export_markdown(&rec);
    assert!(md.starts_with("# What crashed?"));
    assert!(md.contains("**You:** What crashed?") && md.contains("**Assistant:** The crash is event 5."));
    assert!(md.contains("Evidence: events #5 (unverified)") && md.contains("Query: `level = fatal`"));
    assert!(md.contains("[Docs x](https://docs.example.org/a)") && md.contains("Proposal shown: pipeline") && md.contains("*(stopped)*"));
    assert!(export_markdown(&ChatRecord {
        id: "i".into(),
        title: " ".into(),
        turns: vec![],
    })
    .starts_with("# Chat"));
}

#[test]
fn starter_prompts_come_from_the_overview() {
    let p = suggested_prompts(store());
    assert!((3..=5).contains(&p.len()), "{p:?}");
    assert!(p.iter().all(|s| s.len() > 10) && p.iter().collect::<std::collections::HashSet<_>>().len() == p.len());
    let findings = store()
        .overview(&imt_core::api::QueryRequest { query: Default::default() })
        .unwrap();
    assert!(!findings.is_empty());
    assert!(p.iter().any(|s| s.contains(&findings[0].title)), "{p:?} vs {}", findings[0].title);
    assert!(p.iter().any(|s| s.contains("What happened around")), "{p:?}");
    assert_eq!(format_hm(ms(10, 42, 7)), "10:42");
}

#[test]
fn provider_trait_objects_work_with_chat() {
    let p: Arc<dyn LlmProvider> = Arc::new(ScriptedProvider::text(&["fine"]));
    let a = Assistant::new(p, AssistantConfig::default());
    let env = ChatEnv::new(&a, store().into());
    let mut s = ChatSession::new("d");
    s.send(&env, "hi", &ChatContext::default(), &mut |_| {}).unwrap();
    assert_eq!(s.turns()[0].assistant, "fine");
}

fn screenshot() -> imt_ai::llm::ImageInput {
    imt_ai::vision::image::sanitize(
        &imt_ai::vision::image::testing::screenshot_png(200, 120),
        1 << 20,
        &imt_ai::vision::image::ImageLimits::default(),
    )
    .unwrap()
    .image
}

#[test]
fn an_image_goes_with_its_question_only_and_the_history_keeps_a_note() {
    let a = Assistant::new(
        ScriptedProvider::new(vec![text("An error dialog."), text("Still the dialog.")]).with_vision(),
        AssistantConfig::default(),
    );
    let mut s = ChatSession::new("c");
    let env = ChatEnv::new(&a, store().into()).with_images(vec![screenshot()]);
    s.send(&env, "what is this?", &ChatContext::default(), &mut |_| {}).unwrap();
    let first = a.provider().requests()[0].clone();
    assert_eq!(first.messages.last().unwrap().images.len(), 1);
    assert_eq!(s.turns()[0].user, "what is this? [image]");
    // The next question without an image: the earlier image is not sent again.
    send(&mut s, &a, "and now?", &ChatContext::default()).0.unwrap();
    let second = a.provider().requests()[1].clone();
    assert!(second.messages.iter().all(|m| m.images.is_empty()));
    // A model that cannot read images fails the turn with the typed error.
    let blind = assistant(vec![text("unused")]);
    let mut s = ChatSession::new("d");
    let env = ChatEnv::new(&blind, store().into()).with_images(vec![screenshot()]);
    let mut events = vec![];
    assert_eq!(
        s.send(&env, "what is this?", &ChatContext::default(), &mut |e| events.push(e.clone())),
        Err(AiError::NoVision)
    );
    assert_eq!(kinds(&events).last(), Some(&"error"));
}

#[test]
fn a_related_logs_turn_lists_the_links_with_reasons_and_cites_them() {
    use imt_core::related::{Reason, RelatedEvent, RelatedLink, RelatedNote, RelatedResult};
    let link = RelatedLink {
        event: RelatedEvent {
            id: 42,
            ts: Some(0),
            level: imt_core::model::Level::Error,
            message: "Payment failed".into(),
            exception: None,
            source: 1,
            source_name: "orders.clef".into(),
            group: String::new(),
            fields: Default::default(),
        },
        score: 6.0,
        reasons: vec![Reason::Time { delta_ms: 1400 }],
    };
    let outcome = imt_ai::vision::RelatedOutcome {
        facts: Default::default(),
        result: RelatedResult {
            time: None,
            links: vec![link],
            notes: vec![RelatedNote::NoTime],
            examined: 3,
        },
        explanation: imt_ai::assistant::Narrative {
            text: "Event #42 is the failure.".into(),
            citations: vec![],
            evidence: imt_ai::assistant::Evidence {
                record_ids: vec![42],
                queries: vec![],
            },
            usage: Default::default(),
        },
    };
    let mut s = ChatSession::new("r");
    let mut events = vec![];
    s.send_related("find the logs related to this screenshot", 1, || Ok(outcome), &mut |e| {
        events.push(e.clone())
    })
    .unwrap();
    assert_eq!(kinds(&events), ["toolStarted", "toolFinished", "done"]);
    let ChatEvent::Done { text, citations, .. } = &events[2].event else {
        panic!()
    };
    assert!(text.contains("- #42 orders.clef: Payment failed (time +1.4 s)"), "{text}");
    assert!(text.contains("matched by text and title only"));
    assert!(citations.contains(&Citation::Record { id: 42, verified: true }));
    assert_eq!(s.turns()[0].user, "find the logs related to this screenshot [image]");
    // A failure is the turn's error, attributed to the search.
    let mut events = vec![];
    let e = s
        .send_related("/related", 1, || Err(AiError::NoVision), &mut |e| events.push(e.clone()))
        .unwrap_err();
    assert_eq!(e, AiError::NoVision);
    assert_eq!(kinds(&events), ["toolStarted", "toolFinished", "error"]);
}
