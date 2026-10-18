//! Repeatable evaluation of the AI use cases with a real model, through the real worker over the real
//! transport. Cases live in `evals/cases.toml` (built from /samples) and are judged by the engine, not by
//! string comparison: queries by the events they match, regexes by what they match, profiles and pipelines by
//! parsing and dry-running the sample, answers by whether the record ids they cite exist and a key fact
//! appears.
//!
//!   cargo build --release -p imt-ai-worker --features metal      # or vulkan; no feature = CPU
//!   cargo run --release -p imt-ai --example eval -- --model models/Qwen3.5-4B-Q4_K_M.gguf --out report.json
//!
//! Options: `--cases <file>`, `--worker <binary>` (default: IMT_AI_WORKER or next to the example),
//! `--context <tokens>` (8192), `--only a,b` (use cases), `--limit N` (cases per use case),
//! `--samples <dir>` (default: ../../samples).

use imt_ai::assistant::{Assistant, AssistantConfig, Citation};
use imt_ai::chat::{ChatContext, ChatEnv, ChatEvent, ChatSession};
use imt_ai::gguf;
use imt_ai::hardware::{HardwareProbe, SystemProbe};
use imt_ai::llm::{ChatRequest, ChatResponse, LlmProvider};
use imt_ai::runtime::{GpuMode, LoadRequest, ModelRuntime};
use imt_ai::worker::{default_worker_path, GrpcLauncher, WorkerConfig, WorkerRuntime};
use imt_ai::AiError;
use imt_core::api::{PageRequest, QueryRequest};
use imt_core::ql::QlResult;
use imt_core::query::Query;
use imt_core::{LogStore, Profile, Workspace};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ---------------------------------------------------------------- cases

#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
struct Sample {
    /// A file under the samples folder.
    file: String,
    /// First line (0-based) and number of lines taken from the file.
    from: usize,
    lines: usize,
    /// Or the text itself.
    text: String,
}

impl Sample {
    fn load(&self, samples: &Path) -> String {
        if !self.text.is_empty() {
            return self.text.clone();
        }
        let all = std::fs::read_to_string(samples.join(&self.file)).unwrap_or_else(|e| panic!("{}: {e}", self.file));
        all.lines()
            .skip(self.from)
            .take(if self.lines == 0 { 40 } else { self.lines })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[derive(Debug, Deserialize)]
struct NlCase {
    id: String,
    question: String,
    /// A reference query; the proposal passes when both match the same events (or return the same rows).
    expect_ql: String,
}

#[derive(Debug, Deserialize)]
struct RegexCase {
    id: String,
    #[serde(default)]
    intent: String,
    positive: Vec<String>,
    #[serde(default)]
    negative: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ProfileCase {
    id: String,
    #[serde(flatten)]
    sample: Sample,
    /// How many entries the sample holds.
    records: usize,
}

#[derive(Debug, Deserialize)]
struct SampleGoalCase {
    id: String,
    goal: String,
    #[serde(flatten)]
    sample: Sample,
    #[serde(default)]
    dropped_min: Option<usize>,
    #[serde(default)]
    dropped_max: Option<usize>,
    #[serde(default)]
    changed_min: Option<usize>,
    #[serde(default)]
    require_step: Option<String>,
    #[serde(default)]
    matches_min: Option<usize>,
    #[serde(default)]
    matches_max: Option<usize>,
    /// Extractors: the extracted field names must contain one of these (case-insensitive).
    #[serde(default)]
    field_hints: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct NarrativeCase {
    id: String,
    /// Explain: a query whose first match is the event.
    #[serde(default)]
    query: String,
    /// Summarise: "HH:MM:SS" range on 2026-10-14 (UTC).
    #[serde(default)]
    from: String,
    #[serde(default)]
    to: String,
    /// Root causes: sample folders to open (default: all samples).
    #[serde(default)]
    paths: Vec<String>,
    /// One of these must appear in the answer (case-insensitive).
    facts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct Turn {
    user: String,
    /// The turn must call one of these tools.
    #[serde(default)]
    tools: Vec<String>,
    /// The answer must contain the number of events this query matches.
    #[serde(default)]
    count_of: String,
    #[serde(default)]
    facts: Vec<String>,
}

#[derive(Debug, Deserialize)]
struct ChatCase {
    id: String,
    turns: Vec<Turn>,
}

#[derive(Debug, Default, Deserialize)]
struct Cases {
    #[serde(default)]
    nl_to_query: Vec<NlCase>,
    #[serde(default)]
    propose_regex: Vec<RegexCase>,
    #[serde(default)]
    propose_profile: Vec<ProfileCase>,
    #[serde(default)]
    propose_pipeline: Vec<SampleGoalCase>,
    #[serde(default)]
    propose_markers: Vec<SampleGoalCase>,
    #[serde(default)]
    propose_extractors: Vec<SampleGoalCase>,
    #[serde(default)]
    explain_event: Vec<NarrativeCase>,
    #[serde(default)]
    summarize: Vec<NarrativeCase>,
    #[serde(default)]
    root_causes: Vec<NarrativeCase>,
    #[serde(default)]
    chat: Vec<ChatCase>,
}

// ---------------------------------------------------------------- measuring

#[derive(Default)]
struct Meter {
    calls: usize,
    output_tokens: usize,
    input_tokens: usize,
    gen_time: Duration,
}

/// Wraps the real provider and counts calls, tokens and time.
struct Measured {
    inner: Arc<dyn LlmProvider>,
    meter: Arc<Mutex<Meter>>,
}

impl LlmProvider for Measured {
    fn id(&self) -> &str {
        self.inner.id()
    }
    fn display_name(&self) -> String {
        self.inner.display_name()
    }
    fn context_length(&self) -> usize {
        self.inner.context_length()
    }
    fn chat(&self, req: &ChatRequest, on_delta: &mut dyn FnMut(&str)) -> imt_ai::Result<ChatResponse> {
        let t = Instant::now();
        let r = self.inner.chat(req, on_delta);
        let mut m = self.meter.lock().unwrap_or_else(|p| p.into_inner());
        m.calls += 1;
        m.gen_time += t.elapsed();
        if let Ok(resp) = &r {
            if let Some(u) = resp.usage {
                m.output_tokens += u.output_tokens;
                m.input_tokens += u.input_tokens;
            }
        }
        r
    }
}

#[derive(Debug, Clone, Serialize)]
struct CaseResult {
    use_case: String,
    id: String,
    pass: bool,
    /// The first answer was rejected and the model was asked again (proposal use cases).
    repaired: bool,
    /// The model never produced an acceptable answer.
    malformed: bool,
    latency_ms: u64,
    output_tokens: usize,
    gen_ms: u64,
    reason: String,
}

#[derive(Debug, Serialize)]
struct UseCaseReport {
    cases: usize,
    passed: usize,
    pass_rate: f64,
    repair_rate: f64,
    malformed_rate: f64,
    median_ms: u64,
    p95_ms: u64,
    tokens_per_s: f64,
    peak_rss_mb: u64,
}

#[derive(Debug, Serialize)]
struct Report {
    model: String,
    file: String,
    backend: String,
    context: usize,
    load_ms: u64,
    peak_rss_mb: u64,
    use_cases: BTreeMap<String, UseCaseReport>,
    failures: Vec<CaseResult>,
    results: Vec<CaseResult>,
}

struct Ctx {
    samples: PathBuf,
    store: Box<dyn LogStore>,
    assistant: Assistant<Measured>,
    meter: Arc<Mutex<Meter>>,
    rss_peak: Arc<AtomicU64>,
    profile: Profile,
}

fn ms_of(hms: &str) -> i64 {
    let p: Vec<i64> = hms.split(':').map(|x| x.parse().unwrap_or(0)).collect();
    // 2026-10-14 00:00:00 UTC
    const DAY: i64 = 1_791_936_000_000;
    DAY + (p.first().copied().unwrap_or(0) * 3600 + p.get(1).copied().unwrap_or(0) * 60 + p.get(2).copied().unwrap_or(0)) * 1000
}

fn open_store(samples: &Path, profile: &Profile, paths: &[String]) -> Box<dyn LogStore> {
    let mut ws: Box<dyn LogStore> = Box::new(Workspace::new(profile.clone()).expect("workspace"));
    let list: Vec<PathBuf> = if paths.is_empty() {
        vec![samples.to_path_buf()]
    } else {
        paths.iter().map(|p| samples.join(p)).collect()
    };
    ws.open(&list).expect("open samples");
    ws
}

fn event_ids(store: &dyn LogStore, ql: &str) -> Result<Vec<u32>, String> {
    match store.run_query(&QueryRequest {
        query: Query {
            ql: ql.into(),
            ..Query::default()
        },
    }) {
        Ok(QlResult::Events { mut ids, .. }) => {
            ids.sort_unstable();
            Ok(ids)
        }
        Ok(QlResult::Table { mut rows, .. }) => {
            // Tables are compared by their rows, not by column names or order.
            rows.sort_by_key(|r| serde_json::to_string(r).unwrap_or_default());
            Ok(rows
                .iter()
                .enumerate()
                .map(|(i, r)| (i as u32) ^ (serde_json::to_string(r).unwrap_or_default().len() as u32) << 16)
                .collect())
        }
        Err(e) => Err(e.to_string()),
    }
}

fn table_signature(store: &dyn LogStore, ql: &str) -> Option<Vec<String>> {
    match store.run_query(&QueryRequest {
        query: Query {
            ql: ql.into(),
            ..Query::default()
        },
    }) {
        Ok(QlResult::Table { rows, .. }) => {
            let mut v: Vec<String> = rows
                .iter()
                .map(|r| r.iter().map(|c| c.to_string()).collect::<Vec<_>>().join("|"))
                .collect();
            v.sort();
            Some(v)
        }
        _ => None,
    }
}

fn event_exists(store: &dyn LogStore, id: u32) -> bool {
    store
        .page(&PageRequest {
            query: Query::default(),
            offset: 0,
            limit: 1,
            descending: false,
            anchor: Some(id),
        })
        .map(|p| p.rows.first().is_some_and(|r| r.id == id))
        .unwrap_or(false)
}

fn first_match(store: &dyn LogStore, ql: &str) -> Option<u32> {
    event_ids(store, ql).ok().and_then(|v| v.first().copied())
}

fn has_fact(text: &str, facts: &[String]) -> bool {
    let t = text.to_lowercase();
    facts.is_empty() || facts.iter().any(|f| t.contains(&f.to_lowercase()))
}

fn grounded(store: &dyn LogStore, citations: &[Citation]) -> Result<usize, String> {
    let mut n = 0;
    for c in citations {
        if let Citation::Record { id, .. } = c {
            if !event_exists(store, *id) {
                return Err(format!("cites event {id}, which does not exist"));
            }
            n += 1;
        }
    }
    Ok(n)
}

// ---------------------------------------------------------------- the use cases

type Verdict = Result<(), String>;

fn nl(cx: &Ctx, c: &NlCase) -> imt_ai::Result<Verdict> {
    let p = cx.assistant.nl_to_query(cx.store.as_ref(), &c.question)?;
    // Questions that can be read two ways list alternative reference queries ("a || b"); matching any one passes.
    let alternatives: Vec<&str> = c.expect_ql.split(" || ").collect();
    let mut last = Err("no reference query".to_string());
    for reference in &alternatives {
        let verdict = compare_queries(cx, &p.ql, reference);
        if verdict.is_ok() {
            return Ok(Ok(()));
        }
        last = verdict;
    }
    Ok(last)
}

fn compare_queries(cx: &Ctx, got_ql: &str, expect_ql: &str) -> Verdict {
    let (got, want) = (event_ids(cx.store.as_ref(), got_ql), event_ids(cx.store.as_ref(), expect_ql));
    match (got, want) {
        (Ok(g), Ok(w)) => {
            let tables = table_signature(cx.store.as_ref(), expect_ql);
            let same = match tables {
                Some(t) => table_signature(cx.store.as_ref(), got_ql) == Some(t),
                None => g == w,
            };
            if same {
                Ok(())
            } else {
                Err(format!(
                    "`{}` matches {} events, the reference `{}` matches {}",
                    got_ql,
                    g.len(),
                    expect_ql,
                    w.len()
                ))
            }
        }
        (Err(e), _) => Err(format!("`{got_ql}` fails: {e}")),
        (_, Err(e)) => Err(format!("the reference query is broken: {e}")),
    }
}

fn regex_case(cx: &Ctx, c: &RegexCase) -> imt_ai::Result<Verdict> {
    let p = cx
        .assistant
        .propose_regex(&c.positive, if c.intent.is_empty() { None } else { Some(&c.intent) })?;
    let re = imt_core::user_regex::compile(&p.pattern, false).map_err(|e| AiError::Malformed(e.to_string()))?;
    if let Some(miss) = c.positive.iter().find(|l| !re.is_match(l)) {
        return Ok(Err(format!("does not match a positive line: {miss}")));
    }
    if let Some(hit) = c.negative.iter().find(|l| re.is_match(l)) {
        return Ok(Err(format!("matches a negative line: {hit}")));
    }
    Ok(if p.groups.is_empty() {
        Err("no named groups".into())
    } else {
        Ok(())
    })
}

fn profile_case(cx: &Ctx, c: &ProfileCase) -> imt_ai::Result<Verdict> {
    let sample = c.sample.load(&cx.samples);
    let name = if c.sample.file.is_empty() {
        "sample.log"
    } else {
        c.sample.file.rsplit('/').next().unwrap_or("sample.log")
    };
    let p = cx.assistant.propose_profile(cx.store.as_ref(), &sample, name)?;
    let parsed = p.parsed as f64 / c.records.max(1) as f64;
    let shown = p.parsed.clamp(1, 200) as f64;
    let ts = p.with_timestamp as f64 / shown;
    Ok(if parsed < 0.95 {
        Err(format!("parsed {} of {} entries", p.parsed, c.records))
    } else if ts < 0.95 {
        Err(format!("only {:.0}% of entries have a timestamp", ts * 100.0))
    } else {
        Ok(())
    })
}

fn in_range(n: usize, min: Option<usize>, max: Option<usize>) -> bool {
    min.is_none_or(|m| n >= m) && max.is_none_or(|m| n <= m)
}

fn pipeline_case(cx: &Ctx, c: &SampleGoalCase) -> imt_ai::Result<Verdict> {
    let sample = c.sample.load(&cx.samples);
    let p = cx.assistant.propose_pipeline(cx.store.as_ref(), &c.goal, &sample, None)?;
    if let Some(s) = &c.require_step {
        if !p.steps.iter().any(|x| &x.step == s) {
            return Ok(Err(format!(
                "no `{s}` step (steps: {})",
                p.steps.iter().map(|x| x.step.as_str()).collect::<Vec<_>>().join(", ")
            )));
        }
    }
    if !in_range(p.dropped, c.dropped_min, c.dropped_max) {
        return Ok(Err(format!(
            "dropped {} (wanted {:?}..{:?}); steps: {}",
            p.dropped,
            c.dropped_min,
            c.dropped_max,
            p.toml.replace('\n', " ")
        )));
    }
    if c.changed_min.is_some_and(|m| p.changed < m) {
        return Ok(Err(format!("changed {} (wanted at least {:?})", p.changed, c.changed_min)));
    }
    Ok(Ok(()))
}

fn markers_case(cx: &Ctx, c: &SampleGoalCase) -> imt_ai::Result<Verdict> {
    let sample = c.sample.load(&cx.samples);
    let p = cx.assistant.propose_markers(cx.store.as_ref(), &c.goal, &sample, None)?;
    Ok(if p.markers.iter().any(|m| in_range(m.matches, c.matches_min, c.matches_max)) {
        Ok(())
    } else {
        Err(format!(
            "marker matches {:?} (wanted {:?}..{:?}); {}",
            p.markers.iter().map(|m| m.matches).collect::<Vec<_>>(),
            c.matches_min,
            c.matches_max,
            p.toml.replace('\n', " ")
        ))
    })
}

fn extractors_case(cx: &Ctx, c: &SampleGoalCase) -> imt_ai::Result<Verdict> {
    let sample = c.sample.load(&cx.samples);
    let p = cx.assistant.propose_extractors(cx.store.as_ref(), &c.goal, &sample, None)?;
    let good = p.extractors.iter().any(|e| {
        in_range(e.matches, c.matches_min, c.matches_max)
            && (c.field_hints.is_empty()
                || e.fields
                    .iter()
                    .any(|f| c.field_hints.iter().any(|h| f.to_lowercase().contains(&h.to_lowercase()))))
    });
    Ok(if good {
        Ok(())
    } else {
        Err(format!(
            "extractors {:?} (wanted fields like {:?}, matches {:?}..{:?}); {}",
            p.extractors.iter().map(|e| (&e.fields, e.matches)).collect::<Vec<_>>(),
            c.field_hints,
            c.matches_min,
            c.matches_max,
            p.toml.replace('\n', " ")
        ))
    })
}

fn narrative_verdict(store: &dyn LogStore, n: imt_ai::assistant::Narrative, facts: &[String]) -> Verdict {
    grounded(store, &n.citations)?;
    if !has_fact(&n.text, facts) {
        return Err(format!(
            "none of {facts:?} in the answer: {}",
            n.text.chars().take(200).collect::<String>().replace('\n', " ")
        ));
    }
    Ok(())
}

fn explain_case(cx: &Ctx, c: &NarrativeCase) -> imt_ai::Result<Verdict> {
    let id = first_match(cx.store.as_ref(), &c.query).ok_or_else(|| AiError::Invalid(format!("no event for {}", c.query)))?;
    let n = cx.assistant.explain_event(cx.store.as_ref(), id)?;
    Ok(narrative_verdict(cx.store.as_ref(), n, &c.facts))
}

fn summarize_case(cx: &Ctx, c: &NarrativeCase) -> imt_ai::Result<Verdict> {
    let n = cx.assistant.summarize(cx.store.as_ref(), ms_of(&c.from), ms_of(&c.to))?;
    Ok(narrative_verdict(cx.store.as_ref(), n, &c.facts))
}

fn root_case(cx: &Ctx, c: &NarrativeCase) -> imt_ai::Result<Verdict> {
    let store = open_store(&cx.samples, &cx.profile, &c.paths);
    let n = cx.assistant.root_causes(store.as_ref())?;
    Ok(narrative_verdict(store.as_ref(), n, &c.facts))
}

fn number_forms(n: usize) -> Vec<String> {
    let plain = n.to_string();
    let mut with_sep = String::new();
    for (i, ch) in plain.chars().enumerate() {
        if i > 0 && (plain.len() - i).is_multiple_of(3) {
            with_sep.push(',');
        }
        with_sep.push(ch);
    }
    vec![plain, with_sep]
}

fn chat_case(cx: &Ctx, c: &ChatCase) -> imt_ai::Result<Verdict> {
    let mut session = ChatSession::new("eval");
    let env = ChatEnv::new(&cx.assistant, cx.store.as_ref().into());
    for (i, t) in c.turns.iter().enumerate() {
        let mut tools_called: Vec<String> = Vec::new();
        let mut answer = String::new();
        let mut proposals = 0;
        session.send(&env, &t.user, &ChatContext::default(), &mut |e| match &e.event {
            ChatEvent::ToolStarted { name, args } => {
                trace(&format!("  tool {name} {args}"));
                tools_called.push(name.clone())
            }
            ChatEvent::Done { text, .. } => answer = text.clone(),
            ChatEvent::Proposal { .. } => proposals += 1,
            _ => {}
        })?;
        let _ = proposals;
        if !t.tools.is_empty() && !t.tools.iter().any(|x| tools_called.contains(x)) {
            return Ok(Err(format!(
                "turn {}: expected a call to {:?}, got {:?}",
                i + 1,
                t.tools,
                tools_called
            )));
        }
        if !t.count_of.is_empty() {
            // Alternatives ("a || b") cover questions that can reasonably be read two ways.
            let mut wanted = Vec::new();
            for ql in t.count_of.split(" || ") {
                match event_ids(cx.store.as_ref(), ql) {
                    Ok(v) => wanted.push(v.len()),
                    Err(e) => return Ok(Err(format!("reference query broken: {e}"))),
                }
            }
            if !wanted.iter().any(|n| number_forms(*n).iter().any(|f| answer.contains(f.as_str()))) {
                return Ok(Err(format!(
                    "turn {}: the answer should say one of {wanted:?}: {}",
                    i + 1,
                    answer.chars().take(160).collect::<String>().replace('\n', " ")
                )));
            }
        }
        if !has_fact(&answer, &t.facts) {
            return Ok(Err(format!(
                "turn {}: none of {:?} in: {}",
                i + 1,
                t.facts,
                answer.chars().take(160).collect::<String>().replace('\n', " ")
            )));
        }
    }
    Ok(Ok(()))
}

/// Prints extra detail when `IMT_EVAL_TRACE=1`.
fn trace(line: &str) {
    if std::env::var_os("IMT_EVAL_TRACE").is_some() {
        println!("{line}");
    }
}

// ---------------------------------------------------------------- running

#[derive(Clone, Copy)]
struct RunOpts<'a> {
    only: &'a [String],
    limit: usize,
    proposal: bool,
}

fn run_all<T>(
    cx: &Ctx,
    name: &str,
    cases: &[T],
    opts: &RunOpts,
    id_of: impl Fn(&T) -> &str,
    f: impl Fn(&Ctx, &T) -> imt_ai::Result<Verdict>,
    out: &mut Vec<CaseResult>,
) {
    let RunOpts { only, limit, proposal } = *opts;
    if (!only.is_empty() && !only.iter().any(|o| o == name)) || cases.is_empty() {
        return;
    }
    cx.rss_peak.store(0, Ordering::Relaxed);
    for c in cases.iter().take(limit) {
        *cx.meter.lock().unwrap_or_else(|p| p.into_inner()) = Meter::default();
        let t = Instant::now();
        let r = f(cx, c);
        let latency = t.elapsed();
        let m = cx.meter.lock().unwrap_or_else(|p| p.into_inner());
        let (pass, malformed, reason) = match r {
            Ok(Ok(())) => (true, false, String::new()),
            Ok(Err(why)) => (false, false, why),
            Err(AiError::Malformed(why)) => (false, true, format!("malformed: {why}")),
            Err(e) => (false, false, format!("error: {e}")),
        };
        let res = CaseResult {
            use_case: name.into(),
            id: id_of(c).into(),
            pass,
            repaired: proposal && m.calls > 1,
            malformed,
            latency_ms: latency.as_millis() as u64,
            output_tokens: m.output_tokens,
            gen_ms: m.gen_time.as_millis() as u64,
            reason,
        };
        println!(
            "{} {name}/{} ({:.1}s, {} calls){}",
            if pass { "PASS" } else { "FAIL" },
            res.id,
            latency.as_secs_f64(),
            m.calls,
            if pass {
                String::new()
            } else {
                format!(" -- {}", res.reason.chars().take(420).collect::<String>())
            }
        );
        out.push(res);
    }
}

fn percentile(sorted: &[u64], p: f64) -> u64 {
    if sorted.is_empty() {
        return 0;
    }
    sorted[(((sorted.len() - 1) as f64) * p).round() as usize]
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1).cloned())
}

struct Sys;
impl imt_collect::CommandRunner for Sys {
    fn run(&self, p: &str, a: &[String], t: Duration) -> Result<imt_collect::CommandOutput, imt_collect::CommandError> {
        imt_collect::SystemRunner.run(p, a, t)
    }
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let model = PathBuf::from(arg(&args, "--model").expect("--model <file.gguf>"));
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let cases_path = arg(&args, "--cases")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("evals/cases.toml"));
    let samples = arg(&args, "--samples")
        .map(PathBuf::from)
        .unwrap_or_else(|| manifest.join("../../samples"));
    let context: usize = arg(&args, "--context").and_then(|v| v.parse().ok()).unwrap_or(8192);
    let limit: usize = arg(&args, "--limit").and_then(|v| v.parse().ok()).unwrap_or(usize::MAX);
    let only: Vec<String> = arg(&args, "--only")
        .map(|v| v.split(',').map(str::to_string).collect())
        .unwrap_or_default();
    let worker = arg(&args, "--worker").map(PathBuf::from).unwrap_or_else(default_worker_path);

    let cases: Cases = toml::from_str(&std::fs::read_to_string(&cases_path).unwrap_or_else(|e| panic!("{}: {e}", cases_path.display())))
        .expect("cases.toml");
    let profile_text = std::fs::read_to_string(samples.join("profiles/orderservice.imtprofile.toml")).expect("sample profile");
    let profile = Profile::from_toml(&profile_text).expect("profile");
    let store = open_store(&samples, &profile, &[]);

    // The real worker, over the real transport.
    let info = gguf::inspect(&model).unwrap_or_else(|e| panic!("{e}"));
    let cfg = WorkerConfig::new(Arc::new(GrpcLauncher {
        exe: worker,
        args: vec![],
        env: imt_ai::worker::fixed_env(vec![]),
    }));
    let rt = WorkerRuntime::new(cfg, Box::new(|| SystemProbe::new(Sys).probe()));
    let t0 = Instant::now();
    let loaded = rt
        .load(&LoadRequest {
            path: model.clone(),
            context,
            info: info.clone(),
            gpu: GpuMode::Auto,
            threads: 0,
        })
        .unwrap_or_else(|e| panic!("cannot load the model: {e}"));
    let load_ms = t0.elapsed().as_millis() as u64;
    println!(
        "loaded {} on {} in {:.1}s (context {})",
        loaded.provider.display_name(),
        loaded.backend,
        load_ms as f64 / 1000.0,
        loaded.provider.context_length()
    );

    // Peak memory of the worker process, sampled while the cases run.
    let rss_peak = Arc::new(AtomicU64::new(0));
    let overall_peak = Arc::new(AtomicU64::new(0));
    {
        let (peak, overall) = (rss_peak.clone(), overall_peak.clone());
        std::thread::spawn(move || {
            let me = sysinfo::get_current_pid().ok();
            let mut sys = sysinfo::System::new();
            loop {
                sys.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
                let rss = sys
                    .processes()
                    .values()
                    .filter(|p| p.parent() == me && p.name().to_string_lossy().contains("imt-ai-worker"))
                    .map(|p| p.memory())
                    .max()
                    .unwrap_or(0);
                peak.fetch_max(rss, Ordering::Relaxed);
                overall.fetch_max(rss, Ordering::Relaxed);
                std::thread::sleep(Duration::from_millis(250));
            }
        });
    }

    let meter = Arc::new(Mutex::new(Meter::default()));
    let provider = Measured {
        inner: loaded.provider.clone(),
        meter: meter.clone(),
    };
    let assistant = Assistant::new(
        provider,
        AssistantConfig {
            max_tokens: 1024,
            temperature: 0.2,
            max_tool_rounds: 6,
            ..AssistantConfig::default()
        },
    );
    let cx = Ctx {
        samples,
        store,
        assistant,
        meter,
        rss_peak: rss_peak.clone(),
        profile,
    };

    let mut results: Vec<CaseResult> = Vec::new();
    let mut peaks: BTreeMap<String, u64> = BTreeMap::new();
    macro_rules! group {
        ($name:literal, $cases:expr, $f:expr, $proposal:expr) => {{
            let before = results.len();
            run_all(
                &cx,
                $name,
                &$cases,
                &RunOpts {
                    only: &only,
                    limit,
                    proposal: $proposal,
                },
                |c| c.id.as_str(),
                $f,
                &mut results,
            );
            if results.len() > before {
                peaks.insert($name.to_string(), rss_peak.load(Ordering::Relaxed) / (1024 * 1024));
            }
        }};
    }
    group!("nl_to_query", cases.nl_to_query, nl, true);
    group!("propose_regex", cases.propose_regex, regex_case, true);
    group!("propose_profile", cases.propose_profile, profile_case, true);
    group!("propose_pipeline", cases.propose_pipeline, pipeline_case, true);
    group!("propose_markers", cases.propose_markers, markers_case, true);
    group!("propose_extractors", cases.propose_extractors, extractors_case, true);
    group!("explain_event", cases.explain_event, explain_case, false);
    group!("summarize", cases.summarize, summarize_case, false);
    group!("root_causes", cases.root_causes, root_case, false);
    group!("chat", cases.chat, chat_case, false);

    let mut use_cases = BTreeMap::new();
    for name in peaks.keys() {
        let rs: Vec<&CaseResult> = results.iter().filter(|r| &r.use_case == name).collect();
        let n = rs.len() as f64;
        let mut lat: Vec<u64> = rs.iter().map(|r| r.latency_ms).collect();
        lat.sort_unstable();
        let (tokens, gen_ms) = (
            rs.iter().map(|r| r.output_tokens).sum::<usize>() as f64,
            rs.iter().map(|r| r.gen_ms).sum::<u64>() as f64,
        );
        use_cases.insert(
            name.clone(),
            UseCaseReport {
                cases: rs.len(),
                passed: rs.iter().filter(|r| r.pass).count(),
                pass_rate: rs.iter().filter(|r| r.pass).count() as f64 / n,
                repair_rate: rs.iter().filter(|r| r.repaired).count() as f64 / n,
                malformed_rate: rs.iter().filter(|r| r.malformed).count() as f64 / n,
                median_ms: percentile(&lat, 0.5),
                p95_ms: percentile(&lat, 0.95),
                tokens_per_s: if gen_ms > 0.0 { tokens / (gen_ms / 1000.0) } else { 0.0 },
                peak_rss_mb: peaks[name],
            },
        );
    }
    let report = Report {
        model: loaded.provider.display_name(),
        file: model.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default(),
        backend: loaded.backend.clone(),
        context,
        load_ms,
        peak_rss_mb: overall_peak.load(Ordering::Relaxed) / (1024 * 1024),
        failures: results.iter().filter(|r| !r.pass).cloned().collect(),
        results: results.clone(),
        use_cases,
    };
    println!(
        "\n{:20} {:>6} {:>7} {:>7} {:>9} {:>9} {:>9} {:>7} {:>8}",
        "use case", "pass", "rate", "repair", "malformed", "median s", "p95 s", "tok/s", "rss MB"
    );
    for (name, u) in &report.use_cases {
        println!(
            "{:20} {:>3}/{:<2} {:>6.0}% {:>6.0}% {:>8.0}% {:>9.1} {:>9.1} {:>7.1} {:>8}",
            name,
            u.passed,
            u.cases,
            u.pass_rate * 100.0,
            u.repair_rate * 100.0,
            u.malformed_rate * 100.0,
            u.median_ms as f64 / 1000.0,
            u.p95_ms as f64 / 1000.0,
            u.tokens_per_s,
            u.peak_rss_mb
        );
    }
    if let Some(out) = arg(&args, "--out") {
        std::fs::write(&out, serde_json::to_string_pretty(&report).unwrap_or_default()).expect("write report");
        println!("report: {out}");
    }
}
