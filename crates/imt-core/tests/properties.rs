//! Property-based tests: parsers never panic or hang on arbitrary or mutated input,
//! plus a few semantic invariants (paging, timeline totals, export round-trip).
//!
//! Run more cases with `PROPTEST_CASES=5000 cargo test -p imt-core --test properties`.

use imt_core::archive;
use imt_core::condition::{CondValue, Condition, ConditionCfg};
use imt_core::export::{clef_value, ExportRow, Exporter};
use imt_core::importers::{self, CsvImporter, ImportCtx, Importer, Importers, JsonImporter, TextImporter};
use imt_core::model::{Level, Record};
use imt_core::patterns::{self, MinerConfig};
use imt_core::pipeline::{Pipeline, StepCfg};
use imt_core::profile::CompiledProfile;
use imt_core::query::{FieldFilter, Query};
use imt_core::template;
use imt_core::time::{self, TimestampParser};
use imt_core::workspace::{PageRequest, TimelineRequest};
use imt_core::{exceptions, ql, user_regex, Profile, Workspace};
use proptest::prelude::*;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::sync::OnceLock;
use std::time::Duration;

const CASES: u32 = 256;
const HANG_LIMIT: Duration = Duration::from_secs(15);

/// Runs `f` on a helper thread and fails if it panics or takes longer than [`HANG_LIMIT`].
fn guard<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> T {
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)));
    });
    match rx.recv_timeout(HANG_LIMIT) {
        Ok(Ok(v)) => v,
        Ok(Err(p)) => std::panic::resume_unwind(p),
        Err(_) => panic!("hang: no result within {HANG_LIMIT:?}"),
    }
}

// ---------------------------------------------------------------------------
// Input generators
// ---------------------------------------------------------------------------

/// Byte-level mutations applied to a seed: flips, insertions, deletions, truncation, splices.
fn mutate(seed: &[u8], ops: &[(u8, usize, u8)]) -> Vec<u8> {
    let mut v = seed.to_vec();
    for &(kind, pos, byte) in ops {
        if v.is_empty() {
            v.push(byte);
            continue;
        }
        let i = pos % v.len();
        match kind % 6 {
            0 => v[i] ^= 1 << (byte % 8),
            1 => v[i] = byte,
            2 => v.insert(i, byte),
            3 => {
                v.remove(i);
            }
            4 => v.truncate(i),
            _ => {
                let j = (pos / 7 + byte as usize) % v.len();
                let chunk: Vec<u8> = v[j..(j + 16).min(v.len())].to_vec();
                for (k, b) in chunk.into_iter().enumerate() {
                    v.insert((i + k).min(v.len()), b);
                }
            }
        }
    }
    v
}

fn mutate_str(seed: &str, ops: &[(u8, usize, u8)]) -> String {
    String::from_utf8_lossy(&mutate(seed.as_bytes(), ops)).into_owned()
}

fn ops() -> impl Strategy<Value = Vec<(u8, usize, u8)>> {
    prop::collection::vec((any::<u8>(), any::<usize>(), any::<u8>()), 0..12)
}

fn root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

/// First `max` bytes of every sample file (the /samples folder plus the sample profile).
fn sample_prefixes() -> &'static Vec<Vec<u8>> {
    static S: OnceLock<Vec<Vec<u8>>> = OnceLock::new();
    S.get_or_init(|| {
        fn walk(dir: &Path, out: &mut Vec<Vec<u8>>) {
            let Ok(rd) = std::fs::read_dir(dir) else { return };
            let mut entries: Vec<_> = rd.flatten().map(|e| e.path()).collect();
            entries.sort();
            for p in entries {
                if p.is_dir() {
                    walk(&p, out);
                } else if p.extension().is_none_or(|e| e != "py") {
                    use std::io::Read;
                    let mut buf = Vec::new();
                    if let Ok(f) = std::fs::File::open(&p) {
                        let _ = f.take(6000).read_to_end(&mut buf);
                    }
                    if !buf.is_empty() {
                        out.push(buf);
                    }
                }
            }
        }
        let mut out = Vec::new();
        walk(&root().join("samples"), &mut out);
        assert!(!out.is_empty(), "no samples found");
        out
    })
}

fn sample_mutation() -> impl Strategy<Value = Vec<u8>> {
    (any::<prop::sample::Index>(), ops()).prop_map(|(i, ops)| {
        let seeds = sample_prefixes();
        mutate(&seeds[i.index(seeds.len())], &ops)
    })
}

/// Arbitrary bytes, mutated sample prefixes, or arbitrary UTF-16 text.
fn file_bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..2048),
        sample_mutation(),
        ".{0,400}".prop_map(|s| {
            let mut v = vec![0xFF, 0xFE];
            v.extend(s.encode_utf16().flat_map(|u| u.to_le_bytes()));
            v
        }),
    ]
}

const TS_SHAPES: &[&str] = &[
    "2026-10-14T10:00:00.123Z",
    "2026-10-14 10:00:00,123",
    "14/Oct/2026:10:00:00 +0800",
    "Oct 14 10:00:00",
    "10/14/2026 10:00:00 PM",
    "1791972000123",
    "1791972000.123456",
    "Wed, 14 Oct 2026 10:00:00 GMT",
    "20261014T100000+0200",
    "10:00:00.5",
    "2026-10-14T10:00:00-05:00",
    "14.10.2026 10:00:00",
    "2025/11/08,00:04:37.759+01:00",
    "Sat Nov 08 00:04:37 2025",
];

fn timestamp_like() -> impl Strategy<Value = String> {
    prop_oneof![
        "[0-9]{4}[-/][0-9]{2}[-/][0-9]{2}[T ,][0-9]{2}:[0-9]{2}:[0-9]{2}([.,][0-9]{1,12})?(Z|[+-][0-9]{2}:?[0-9]{2})?",
        "[0-9]{1,4}[/.-][0-9]{1,2}[/.-][0-9]{1,4} [0-9]{1,2}:[0-9]{2}(:[0-9]{2})? ?([AaPp][Mm])?",
        "[0-9.eE+-]{1,30}",
        "[A-Za-z]{3} +[0-9]{1,2} [0-9:]{4,8}",
        (any::<prop::sample::Index>(), ops()).prop_map(|(i, o)| mutate_str(TS_SHAPES[i.index(TS_SHAPES.len())], &o)),
    ]
}

fn default_ctx_profile() -> &'static CompiledProfile {
    static P: OnceLock<CompiledProfile> = OnceLock::new();
    P.get_or_init(|| CompiledProfile::compile(Profile::default_profile()).unwrap())
}

fn sample_ctx_profile() -> &'static CompiledProfile {
    static P: OnceLock<CompiledProfile> = OnceLock::new();
    P.get_or_init(|| {
        let toml = std::fs::read_to_string(root().join("samples/profiles/orderservice.imtprofile.toml")).unwrap();
        CompiledProfile::compile(Profile::from_toml(&toml).unwrap()).unwrap()
    })
}

fn ctx(p: &CompiledProfile) -> ImportCtx<'_> {
    ImportCtx {
        profile: p,
        path: Path::new("fuzz.log"),
        file_date: chrono::NaiveDate::from_ymd_opt(2026, 10, 14),
    }
}

// ---------------------------------------------------------------------------
// Time
// ---------------------------------------------------------------------------

fn parsers() -> Vec<TimestampParser> {
    let fmts = vec!["yyyy-MM-dd HH:mm:ss,fff".to_string(), "dd/MMM/yyyy:HH:mm:ss zzz".to_string()];
    vec![
        TimestampParser::new(&[], "UTC", None).unwrap(),
        TimestampParser::new(&fmts, "+08:00", Some(true)).unwrap(),
        TimestampParser::new(&fmts, "local", Some(false)).unwrap(),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn timestamp_parse_arbitrary(s in any::<String>()) {
        guard(move || for p in parsers() { let _ = p.parse(&s); let _ = p.parse_time_on(&s, chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()); });
    }

    #[test]
    fn timestamp_parse_timestamp_like(s in timestamp_like()) {
        guard(move || for p in parsers() { let _ = p.parse(&s); let _ = p.parse_time_on(&s, chrono::NaiveDate::from_ymd_opt(2026, 1, 1).unwrap()); });
    }

    #[test]
    fn timestamp_parser_new_with_arbitrary_formats(fmt in any::<String>(), zone in any::<String>(), s in timestamp_like()) {
        guard(move || {
            if let Ok(p) = TimestampParser::new(&[fmt], &zone, None) {
                let _ = p.parse(&s);
            }
        });
    }

    #[test]
    fn timestamp_helpers_never_panic(n in any::<f64>(), ms in any::<i64>(), f in ".{0,40}", samples in prop::collection::vec(timestamp_like(), 0..8)) {
        guard(move || {
            let _ = time::epoch_from_number(n);
            let _ = time::utc_date_of(ms);
            let _ = time::to_strftime(&f);
            let _ = time::detect_day_first(samples.iter().map(String::as_str));
        });
    }

    /// The timestamp diagnosis (format inference and its verification) never panics on any lines.
    #[test]
    fn timestamp_suggestion_never_panics(lines in prop::collection::vec(prop_oneof![timestamp_like(), any::<String>()], 0..8)) {
        guard(move || {
            let refs: Vec<&str> = lines.iter().map(String::as_str).collect();
            for p in parsers() {
                let _ = imt_core::timestamp_issue::suggest::propose(&refs, &p);
            }
            let records: Vec<Record> = (1..=lines.len() as u32).map(Record::new).collect();
            let _ = imt_core::timestamp_issue::assess("text", &records, &lines.join("\n"), &parsers()[0]);
        });
    }

    /// Milliseconds formatted as RFC3339 parse back to the same instant.
    #[test]
    fn rfc3339_roundtrip(ms in -2_000_000_000_000i64..4_000_000_000_000i64) {
        use chrono::{SecondsFormat, TimeZone, Utc};
        let text = Utc.timestamp_millis_opt(ms).single().unwrap().to_rfc3339_opts(SecondsFormat::Millis, true);
        let p = TimestampParser::new(&[], "UTC", None).unwrap();
        prop_assert_eq!(p.parse(&text), Some(ms), "text {}", text);
    }
}

// ---------------------------------------------------------------------------
// Templates, regexes, conditions, profile
// ---------------------------------------------------------------------------

fn template_text() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[{}:,@$0-9a-zA-Z_ .<>\\[\\]-]{0,60}",
        (any::<prop::sample::Index>(), ops()).prop_map(|(i, o)| {
            const T: &[&str] = &[
                "{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}",
                "{Timestamp:HH:mm:ss} {SourceContext,-30} {Message}",
                "Order {OrderId} took {Elapsed:0.0} ms {{literal}}",
            ];
            mutate_str(T[i.index(T.len())], &o)
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn template_render_never_panics(t in template_text(), keys in prop::collection::vec("[A-Za-z]{1,6}", 0..4)) {
        guard(move || {
            let mut fields = serde_json::Map::new();
            for (i, k) in keys.iter().enumerate() {
                fields.insert(k.clone(), json!(i));
            }
            let _ = template::render(&t, &fields);
        });
    }

    #[test]
    fn output_template_regex_compiles(t in template_text()) {
        let pattern = guard(move || template::output_template_to_regex(&t));
        prop_assert!(regex::Regex::new(&pattern).is_ok(), "does not compile: {}", pattern);
    }

    #[test]
    fn user_regex_never_panics(pattern in any::<String>(), ci in any::<bool>(), hay in ".{0,200}") {
        guard(move || {
            if let Ok(re) = user_regex::compile(&pattern, ci) {
                let _ = re.is_match(&hay);
            }
        });
    }

    #[test]
    fn condition_compile_never_panics(field in ".{0,10}", op in prop_oneof![
        Just("matches".to_string()), Just("contains".to_string()), Just("in".to_string()), Just("gt".to_string()), Just(">".to_string()),
        Just("eq".to_string()), Just("=".to_string()), Just("exists".to_string()), any::<String>()
    ], value in prop::option::of(".{0,60}"), n in any::<f64>()) {
        guard(move || {
            let mut rec = Record::new(1);
            rec.message = "hello 42".into();
            rec.fields.insert("a".into(), json!(42));
            for v in [value.clone().map(CondValue::Text), Some(CondValue::Number(n)), Some(CondValue::List(vec![value.clone().unwrap_or_default()]))] {
                let cfg = ConditionCfg { field: field.clone(), op: op.clone(), value: v };
                if let Ok(c) = Condition::compile(&cfg) {
                    let _ = c.eval(&rec, "src");
                }
            }
            if let Ok(c) = Condition::new(&field, &op, value.as_deref()) {
                let _ = c.eval(&rec, "src");
            }
        });
    }
}

const PROFILE_FRAGMENTS: &[&str] = &[
    "[parse]\nformat = \"text\"\n",
    "[[extract]]\nname = \"x\"\nregex = \"(?P<a>\\\\d+)\"\n",
    "[[marker]]\nkind = \"k\"\nlabel = \"L\"\nwhen = [{ field = \"@level\", op = \"=\", value = \"Error\" }]\n",
    "[[pipeline]]\nstep = \"mask\"\nregex = \"\\\\d+\"\n",
    "[[chart]]\nid = \"c\"\ntitle = \"T\"\n",
    "[files]\ninclude = [\"*.log\"]\nexclude = [\"[\"]\n",
    "[parse.json]\nmessage = [\"msg\"]\n",
    "[parse]\nassume_timezone = \"+08:00\"\ntimestamp_formats = [\"yyyy\"]\n",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn profile_from_toml_arbitrary(s in any::<String>()) {
        guard(move || { let _ = Profile::from_toml(&s).map(CompiledProfile::compile); });
    }

    #[test]
    fn profile_from_toml_mutated(pick in prop::collection::vec(any::<prop::sample::Index>(), 0..4), o in ops(), base in prop::bool::ANY) {
        let mut text = if base { imt_core::DEFAULT_PROFILE_TOML.to_string() } else { String::new() };
        for i in &pick { text.push_str(PROFILE_FRAGMENTS[i.index(PROFILE_FRAGMENTS.len())]); }
        let text = mutate_str(&text, &o);
        guard(move || {
            if let Ok(p) = Profile::from_toml(&text) {
                let _ = CompiledProfile::compile(p);
            }
        });
    }
}

#[test]
fn profile_from_toml_default_and_sample_are_valid() {
    assert!(Profile::from_toml(imt_core::DEFAULT_PROFILE_TOML).is_ok());
    assert!(CompiledProfile::compile(Profile::default_profile()).is_ok());
    let _ = sample_ctx_profile();
}

// ---------------------------------------------------------------------------
// Importers
// ---------------------------------------------------------------------------

fn run_importers(bytes: Vec<u8>) {
    let Some(text) = importers::decode(&bytes) else { return };
    let head: String = text.chars().take(64 * 1024).collect();
    for profile in [default_ctx_profile(), sample_ctx_profile()] {
        let c = ctx(profile);
        let all: [&dyn Importer; 3] = [&JsonImporter, &CsvImporter, &TextImporter];
        for imp in all {
            let score = imp.sniff(&c, &head);
            assert!(score <= 100, "{} sniff score {score}", imp.id());
            if let Ok(imported) = imp.import(&c, &text) {
                for r in &imported.records {
                    // Fields must be serialisable (they cross the IPC boundary).
                    let _ = serde_json::to_string(&r.fields).unwrap();
                }
            }
        }
        let importers = Importers::new();
        if let Some(imp) = importers.select(&c, &head) {
            let _ = imp.import(&c, &text);
        }
    }
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn builtin_importers_never_panic(bytes in file_bytes()) {
        guard(move || run_importers(bytes));
    }

    #[test]
    fn decode_never_panics(bytes in prop::collection::vec(any::<u8>(), 0..2048)) {
        guard(move || { let _ = importers::decode(&bytes); });
    }

    #[test]
    fn text_importer_with_arbitrary_text_format(fmt in "[ -~]{0,80}", body in "[ -~\n]{0,300}") {
        guard(move || {
            let toml = format!("[parse]\nformat = \"text\"\n[[parse.text]]\nregex = {:?}\n", fmt);
            if let Ok(p) = Profile::from_toml(&toml) {
                if let Ok(cp) = CompiledProfile::compile(p) {
                    let _ = TextImporter.import(&ctx(&cp), &body);
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Archives
// ---------------------------------------------------------------------------

fn gzip(data: &[u8]) -> Vec<u8> {
    let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    e.write_all(data).unwrap();
    e.finish().unwrap()
}

fn zip_of(files: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut w = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    for (name, data) in files {
        w.start_file(name.as_str(), zip::write::SimpleFileOptions::default()).unwrap();
        w.write_all(data).unwrap();
    }
    w.finish().unwrap().into_inner()
}

fn archive_bytes() -> impl Strategy<Value = Vec<u8>> {
    let payload = prop::collection::vec(any::<u8>(), 0..512);
    prop_oneof![
        prop::collection::vec(any::<u8>(), 0..1024),
        (payload.clone(), ops()).prop_map(|(p, o)| mutate(&gzip(&p), &o)),
        (prop::collection::vec(("[a-zA-Z0-9_./\\\\-]{1,24}", payload), 0..4), ops()).prop_map(|(f, o)| mutate(&zip_of(&f), &o)),
        // Valid magic followed by garbage.
        prop::collection::vec(any::<u8>(), 0..256).prop_map(|mut v| {
            v.splice(0..0, *b"PK\x03\x04");
            v
        }),
        prop::collection::vec(any::<u8>(), 0..256).prop_map(|mut v| {
            v.splice(0..0, [0x1f, 0x8b, 8, 0]);
            v
        }),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn archive_expand_never_panics(bytes in archive_bytes(), name in "[a-z./]{0,12}") {
        guard(move || {
            let _ = archive::expand(&name, bytes.clone());
            let _ = archive::read_entry(bytes, &name, "a.log");
        });
    }
}

#[test]
fn archive_roundtrip_smoke() {
    let z = zip_of(&[("a/b.log".into(), b"hello".to_vec())]);
    let entries = archive::expand("x.zip", z).unwrap();
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].bytes, b"hello");
    let g = archive::expand("x.log.gz", gzip(b"abc")).unwrap();
    assert_eq!(g[0].name.as_deref(), Some("x.log"));
}

// ---------------------------------------------------------------------------
// Query language
// ---------------------------------------------------------------------------

const QL_SEEDS: &[&str] = &[
    "level >= warning and message contains \"timeout\" | stats count(), p95(duration) as p95 by source | sort p95 desc | head 10",
    "not (level = error or level = fatal) and exists(Cpu) | fields Cpu, MachineName",
    "Cpu > 5 and MachineName in (\"WEB01\", WEB02) | where message ~ \"^App.*\" | sort Cpu | limit 3",
    "`odd field` != 'x' | stats count_distinct(MachineName), avg(Cpu), min(Cpu), max(Cpu), sum(Cpu), p50(Cpu), p99(Cpu) by @level",
    "@source !contains foo missing(x) !~ \"(\" | head 0",
];

fn ql_text() -> impl Strategy<Value = String> {
    prop_oneof![
        any::<String>(),
        "[a-z0-9 |()=!<>~,.\"'`@$*+?\\\\-]{0,80}",
        (any::<prop::sample::Index>(), ops()).prop_map(|(i, o)| mutate_str(QL_SEEDS[i.index(QL_SEEDS.len())], &o)),
    ]
}

fn sample_records() -> Vec<Record> {
    (0..6u32)
        .map(|i| {
            let mut r = Record::new(i + 1);
            r.id = i;
            r.ts = Some(1_791_972_000_000 + i as i64 * 1000);
            r.level = Level::ALL[i as usize % Level::ALL.len()];
            r.message = format!("timeout {i} on host");
            r.fields.insert("Cpu".into(), json!(i * 3));
            r.fields.insert("MachineName".into(), json!(format!("WEB0{}", i % 2)));
            r
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn ql_parse_and_run_never_panic(text in ql_text()) {
        guard(move || {
            if let Ok(program) = ql::parse(&text) {
                let recs = sample_records();
                let _ = ql::run(&program, &mut recs.iter().map(|r| (r, "src")));
            }
        });
    }

    #[test]
    fn ql_completions_never_panic(text in ql_text(), cursor in 0usize..400, at_end in any::<bool>()) {
        guard(move || {
            let fields = vec!["Cpu".to_string(), "MachineName".to_string(), "odd field".to_string()];
            let cur = if at_end { text.len() } else { cursor };
            let _ = ql::completions(&text, cur, &fields);
        });
    }
}

// ---------------------------------------------------------------------------
// Pipeline
// ---------------------------------------------------------------------------

const STEPS: &[&str] = &[
    "drop",
    "keep",
    "mask",
    "remove_fields",
    "rename",
    "set",
    "set_level",
    "truncate",
    "dedupe",
    "parse_json",
    "parse_kv",
    "lookup",
    "shift_time",
    "plugin",
    "nope",
];

fn small_text() -> impl Strategy<Value = Value> {
    prop_oneof![
        Just(Value::Null),
        "[ -~]{0,12}".prop_map(Value::from),
        prop::sample::select(vec![
            "@message", "@level", "Cpu", "x", "+2.4s", "-150ms", "5m", "(", "\\d+", "Error", "(?i)a+"
        ])
        .prop_map(Value::from),
    ]
}

fn step_json() -> impl Strategy<Value = Value> {
    (
        prop::sample::select(STEPS),
        prop::collection::vec(
            (
                "(field|fields|regex|replace|window|from|to|value|level|max|id|prefix|table|file|key|by|disabled|when)",
                small_text(),
            ),
            0..8,
        ),
        prop::collection::vec(
            (
                prop::sample::select(vec!["@level", "Cpu", "@message", ""]),
                prop::sample::select(vec!["=", "!=", "contains", "matches", "exists", "missing", "gt", "lt", "in", "??"]),
                small_text(),
            ),
            0..3,
        ),
    )
        .prop_map(|(step, kv, when)| {
            let mut o = serde_json::Map::new();
            o.insert("step".into(), json!(step));
            for (k, v) in kv {
                // Mostly well-typed keys; a wrong shape just fails deserialisation, which is also exercised.
                let v = match k.as_str() {
                    "fields" => json!([v]),
                    "max" => json!(3),
                    "disabled" => json!(false),
                    "when" => continue,
                    "table" => json!({"k": {"a": "b"}}),
                    _ => v,
                };
                o.insert(k, v);
            }
            if !when.is_empty() {
                o.insert(
                    "when".into(),
                    Value::Array(
                        when.into_iter()
                            .map(|(f, op, v)| json!({"field": f, "op": op, "value": v}))
                            .collect(),
                    ),
                );
            }
            Value::Object(o)
        })
}

fn json_value() -> impl Strategy<Value = Value> {
    let leaf = prop_oneof![
        Just(Value::Null),
        any::<bool>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<String>().prop_map(Value::from),
    ];
    leaf.prop_recursive(4, 48, 6, |inner| {
        prop_oneof![
            prop::collection::vec(inner.clone(), 0..6).prop_map(Value::Array),
            prop::collection::hash_map("[a-z]{1,8}", inner, 0..6).prop_map(|m| Value::Object(m.into_iter().collect())),
        ]
    })
}

fn pipeline_input() -> Vec<Record> {
    (0..4u32)
        .map(|i| {
            let mut r = sample_records().swap_remove(i as usize);
            r.fields.insert("x".into(), json!(r#"{"a":1,"b":"two"}"#));
            r.message = format!("k=v a=1 \"q\"=2 id {i} 12345");
            r
        })
        .collect()
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn pipeline_steps_compile_and_run(steps in prop::collection::vec(step_json(), 0..5)) {
        guard(move || {
            let cfgs: Vec<StepCfg> = steps.into_iter().filter_map(|v| serde_json::from_value(v).ok()).collect();
            if let Ok(p) = Pipeline::compile(&cfgs, &HashMap::new()) {
                let (out, _) = p.run(pipeline_input(), "src");
                assert!(out.len() <= 4);
                let _ = p.run_with_stats(pipeline_input(), "src");
            }
        });
    }

    #[test]
    fn step_cfg_from_arbitrary_json(v in json_value()) {
        guard(move || {
            if let Ok(cfg) = serde_json::from_value::<StepCfg>(v) {
                if let Ok(p) = Pipeline::compile(&[cfg], &HashMap::new()) {
                    let _ = p.run(pipeline_input(), "src");
                }
            }
        });
    }
}

// ---------------------------------------------------------------------------
// Exceptions and pattern mining
// ---------------------------------------------------------------------------

const EXC_SEEDS: &[&str] = &[
    "System.InvalidOperationException: boom\n   at Foo.Bar(Int32 x) in C:\\src\\Foo.cs:line 12\n   at Baz.Run()\n ---> System.IO.IOException: inner\n   at X.Y()\n   --- End of inner exception stack trace ---\n   at Z()",
    "Traceback (most recent call last):\n  File \"a.py\", line 3, in <module>\n    foo()\nValueError: bad",
    "java.lang.RuntimeException: oops\n\tat com.foo.Bar.baz(Bar.java:42)\nCaused by: java.io.IOException: disk\n\t... 3 more",
    "Error: x\n    at fn (/a/b.js:1:2)\n    at async Object.<anonymous> (/a/c.js:3:4)",
    "panic: runtime error\n\ngoroutine 1 [running]:\nmain.main()\n\t/tmp/a.go:5 +0x1d",
];

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn exception_parse_arbitrary(text in any::<String>()) {
        guard(move || { let _ = exceptions::parse_exception(&text); });
    }

    #[test]
    fn exception_parse_mutated(i in any::<prop::sample::Index>(), o in ops()) {
        let text = mutate_str(EXC_SEEDS[i.index(EXC_SEEDS.len())], &o);
        guard(move || {
            if let Some(info) = exceptions::parse_exception(&text) {
                let _ = exceptions::fingerprint(&info, 3);
                let _ = info.root();
            }
        });
    }

    #[test]
    fn exception_group_never_panics(texts in prop::collection::vec(prop_oneof![any::<String>(), (any::<prop::sample::Index>(), ops()).prop_map(|(i, o)| mutate_str(EXC_SEEDS[i.index(EXC_SEEDS.len())], &o))], 0..6)) {
        guard(move || {
            let recs: Vec<Record> = texts.iter().map(|_| Record::new(1)).collect();
            let _ = exceptions::group(recs.iter().zip(texts.iter().map(String::as_str)));
        });
    }

    #[test]
    fn pattern_mine_never_panics(msgs in prop::collection::vec(prop_oneof![any::<String>(), "[a-z0-9 =:\"'/.-]{0,80}"], 0..30), sim in 0.0f64..=1.0, depth in 0usize..8, kids in 0usize..5) {
        guard(move || {
            let recs: Vec<Record> = msgs.into_iter().enumerate().map(|(i, m)| { let mut r = Record::new(i as u32); r.message = m; r }).collect();
            let cfg = MinerConfig { similarity_threshold: sim, depth, max_children: kids };
            let mined = patterns::mine(recs.iter(), &cfg);
            let total: usize = mined.iter().map(|p| p.count).sum();
            assert_eq!(total, recs.len(), "every record belongs to exactly one pattern");
            let _ = patterns::compare(&mined, &mined);
        });
    }
}

// ---------------------------------------------------------------------------
// Workspace invariants: paging partitions results, timeline totals
// ---------------------------------------------------------------------------

struct TempDir(PathBuf);

impl TempDir {
    fn new() -> TempDir {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static N: AtomicUsize = AtomicUsize::new(0);
        let p = std::env::temp_dir().join(format!("imt-prop-{}-{}", std::process::id(), N.fetch_add(1, Ordering::SeqCst)));
        std::fs::create_dir_all(&p).unwrap();
        TempDir(p)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[derive(Debug, Clone)]
struct GenRec {
    ts: Option<i64>,
    level: usize,
    message: String,
    machine: u8,
    cpu: i32,
}

fn gen_rec() -> impl Strategy<Value = GenRec> {
    (
        prop::option::weighted(0.85, 1_790_000_000_000i64..1_792_000_000_000),
        0usize..6,
        "[a-zA-Z0-9 _.:-]{1,24}",
        0u8..3,
        -50i32..500,
    )
        .prop_map(|(ts, level, message, machine, cpu)| GenRec {
            ts,
            level,
            message,
            machine,
            cpu,
        })
}

fn to_record(g: &GenRec, line: u32) -> Record {
    let mut r = Record::new(line);
    r.ts = g.ts;
    r.level = Level::ALL[g.level];
    r.message = g.message.clone();
    r.fields.insert("MachineName".into(), json!(format!("WEB0{}", g.machine)));
    r.fields.insert("Cpu".into(), json!(g.cpu));
    r
}

fn clef_text(recs: &[GenRec]) -> String {
    recs.iter()
        .enumerate()
        .map(|(i, g)| serde_json::to_string(&clef_value(&to_record(g, i as u32 + 1))).unwrap() + "\n")
        .collect()
}

fn gen_query() -> impl Strategy<Value = Query> {
    (
        prop::option::of(1_790_000_000_000i64..1_792_000_000_000),
        prop::option::of(1_790_000_000_000i64..1_792_000_000_000),
        prop::collection::vec(0usize..6, 0..3),
        prop::sample::select(vec!["", "a", "e", "1", "x y", "."]),
        prop::option::of(prop::sample::select(vec![
            ("Cpu", ">", "100"),
            ("Cpu", "<", "0"),
            ("MachineName", "=", "WEB01"),
            ("MachineName", "!=", "WEB00"),
            ("@message", "contains", "a"),
        ])),
        prop::sample::select(vec![
            "",
            "Cpu > 10",
            "level >= warning",
            "MachineName = WEB02 or Cpu < 5",
            "not exists(nope)",
        ]),
    )
        .prop_map(|(from, to, levels, text, filter, ql)| Query {
            from,
            to,
            levels: levels.into_iter().map(|i| Level::ALL[i]).collect(),
            text: text.to_string(),
            filters: filter
                .map(|(f, op, v)| FieldFilter {
                    field: f.into(),
                    op: op.into(),
                    value: Some(v.into()),
                })
                .into_iter()
                .collect(),
            ql: ql.to_string(),
            ..Query::default()
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn paging_partitions_results(recs in prop::collection::vec(gen_rec(), 1..40), q in gen_query(), size in 1usize..15, desc in any::<bool>()) {
        let dir = TempDir::new();
        std::fs::write(dir.0.join("a.clef"), clef_text(&recs)).unwrap();
        let mut ws = Workspace::new(Profile::default_profile()).unwrap();
        ws.open(std::slice::from_ref(&dir.0)).unwrap();
        prop_assert_eq!(ws.records().len(), recs.len());

        let matching = ws.matching(&q).unwrap();
        let req = |offset, limit| PageRequest { query: q.clone(), offset, limit, descending: desc, anchor: None };
        let full = ws.page(&req(0, 5000)).unwrap();
        prop_assert_eq!(full.total, matching.len());
        let full_ids: Vec<u32> = full.rows.iter().map(|r| r.id).collect();
        let mut expected: Vec<u32> = matching.to_vec();
        if desc { expected.reverse(); }
        prop_assert_eq!(&full_ids, &expected);

        let mut seen = Vec::new();
        let mut offset = 0;
        loop {
            let p = ws.page(&req(offset, size)).unwrap();
            prop_assert_eq!(p.total, matching.len());
            if p.rows.is_empty() { break; }
            prop_assert!(p.rows.len() <= size);
            seen.extend(p.rows.iter().map(|r| r.id));
            offset += size;
            if offset >= p.total { break; }
        }
        prop_assert_eq!(&seen, &expected, "pages must concatenate to the full result");
        let mut dedup = seen.clone();
        dedup.sort_unstable();
        dedup.dedup();
        prop_assert_eq!(dedup.len(), seen.len(), "no duplicates");
    }

    #[test]
    fn timeline_histogram_counts_matching_timestamped_records(recs in prop::collection::vec(gen_rec(), 1..40), q in gen_query(), buckets in 10usize..300) {
        let dir = TempDir::new();
        std::fs::write(dir.0.join("a.clef"), clef_text(&recs)).unwrap();
        let mut ws = Workspace::new(Profile::default_profile()).unwrap();
        ws.open(std::slice::from_ref(&dir.0)).unwrap();
        let matching = ws.matching(&q).unwrap();
        let with_ts = matching.iter().filter(|id| ws.records()[**id as usize].ts.is_some()).count();
        let tl = ws.timeline(&TimelineRequest { query: q.clone(), buckets, always_show_lifecycle: true }).unwrap();
        let total: u64 = tl.histogram.iter().flat_map(|b| b.iter()).map(|n| *n as u64).sum();
        prop_assert_eq!(total, with_ts as u64);
        prop_assert_eq!(tl.histogram.len(), buckets);
    }
}

// ---------------------------------------------------------------------------
// Export round-trip
// ---------------------------------------------------------------------------

fn field_value() -> impl Strategy<Value = Value> {
    prop_oneof![
        any::<i32>().prop_map(Value::from),
        any::<bool>().prop_map(Value::from),
        any::<String>().prop_map(Value::from),
        prop::collection::vec(any::<i32>(), 0..3).prop_map(|v| json!(v)),
    ]
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(CASES))]

    #[test]
    fn clef_export_roundtrips_through_the_json_importer(
        recs in prop::collection::vec(
            (prop::option::of(0i64..4_000_000_000_000), 0usize..6, any::<String>(), prop::collection::btree_map("F[a-z0-9]{0,8}", field_value(), 0..5)),
            1..8,
        )
    ) {
        let records: Vec<Record> = recs
            .iter()
            .enumerate()
            .map(|(i, (ts, lvl, msg, fields))| {
                let mut r = Record::new(i as u32 + 1);
                r.ts = *ts;
                r.level = Level::ALL[*lvl];
                r.message = msg.clone();
                r.fields = fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                r
            })
            .collect();
        let mut out = Vec::new();
        let mut rows = records.iter().map(|r| ExportRow { record: std::borrow::Cow::Borrowed(r), source_name: "s", group: String::new() });
        let n = imt_core::export::ClefExporter.export(&mut rows, &mut out).unwrap();
        prop_assert_eq!(n, records.len());

        let text = String::from_utf8(out).unwrap();
        let c = ctx(default_ctx_profile());
        let imported = JsonImporter.import(&c, &text).unwrap();
        prop_assert_eq!(imported.records.len(), records.len());
        for (a, b) in records.iter().zip(&imported.records) {
            prop_assert_eq!(a.ts, b.ts);
            prop_assert_eq!(a.level, b.level);
            prop_assert_eq!(&a.message, &b.message);
            prop_assert_eq!(&a.fields, &b.fields);
        }
    }
}
