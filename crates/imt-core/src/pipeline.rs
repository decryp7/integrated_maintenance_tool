//! The profile's `[[pipeline]]`: ordered cleaning steps (drop, keep, mask,
//! dedupe, ...) applied to every file's records during import.

use crate::chart::parse_bucket;
use crate::condition::{Condition, ConditionCfg};
use crate::model::{Level, Record};
use crate::plugin::{RecordTransform, TransformCtx, Verdict};
use crate::ql::calc::MAX_CALC_LEN;
use crate::ql::{CalcExpr, QlError};
use crate::user_regex;
use anyhow::{anyhow, Context, Result};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

/// One `[[pipeline]]` entry as written in the profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct StepCfg {
    pub step: String,
    /// Skipped when true (kept in the profile so it can be re-enabled).
    #[serde(default)]
    pub disabled: bool,
    #[serde(default)]
    pub when: Vec<ConditionCfg>,
    #[serde(default)]
    pub field: Option<String>,
    #[serde(default)]
    pub fields: Vec<String>,
    #[serde(default)]
    pub regex: Option<String>,
    #[serde(default)]
    pub replace: Option<String>,
    #[serde(default)]
    pub window: Option<String>,
    #[serde(default)]
    pub from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub level: Option<String>,
    #[serde(default)]
    pub max: Option<usize>,
    #[serde(default)]
    pub id: Option<String>,
    /// `parse_json` / `parse_kv`: prefix for the produced field names.
    #[serde(default)]
    pub prefix: Option<String>,
    /// `lookup`: inline table, key value -> fields to add.
    #[serde(default)]
    pub table: Option<HashMap<String, HashMap<String, toml::Value>>>,
    /// `lookup`: CSV file with a header row; `key` names its key column (default: the first).
    #[serde(default)]
    pub file: Option<String>,
    #[serde(default)]
    pub key: Option<String>,
    /// `shift_time`: signed duration such as "+2.4s" or "-150ms".
    #[serde(default)]
    pub by: Option<String>,
    /// `calc`: expression computing `field` from other fields (see [`crate::ql::calc`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expr: Option<String>,
    /// `script`: Rhai source run once per record (see [`crate::script`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script: Option<String>,
}

/// Events entering and leaving one pipeline step.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
pub struct StepStats {
    pub input: usize,
    pub output: usize,
    /// Records the step failed on and left unchanged (`script` runtime errors and limits).
    pub errors: usize,
}

enum Step {
    /// Any record-level step, built-in or contributed by a plugin.
    Transform(Arc<dyn RecordTransform>),
    /// Needs the whole (time-ordered) batch.
    Dedupe { fields: Vec<String>, window_ms: i64 },
    /// A script run record by record with one reused scope; failures are counted.
    Script(Arc<crate::script::Script>),
}

/// Compiled pipeline. Steps run in order over a file's records.
#[derive(Default)]
pub struct Pipeline {
    steps: Vec<(Vec<Condition>, Step)>,
    /// Fields written by enabled `calc` steps (derived, like extractor fields).
    calc_fields: Vec<String>,
}

/// Field name for the number of collapsed repeats added by `dedupe`.
pub const REPEAT_COUNT: &str = "RepeatCount";

impl Pipeline {
    /// `transforms` are the plugin transforms that `{ step = "plugin" }` may reference.
    pub fn compile(cfgs: &[StepCfg], transforms: &HashMap<String, Arc<dyn RecordTransform>>) -> Result<Pipeline> {
        let mut steps = Vec::new();
        for (i, cfg) in cfgs.iter().enumerate() {
            let ctx = || format!("pipeline step {} ({})", i + 1, cfg.step);
            if cfg.disabled {
                // Keep step indexes aligned with the profile for statistics.
                steps.push((Vec::new(), Step::Transform(Arc::new(Fixed("disabled", Verdict::Keep)))));
                continue;
            }
            let step = compile_step(cfg, transforms).with_context(ctx)?;
            // `keep` evaluates its conditions itself (it drops what does not match).
            let when = if cfg.step == "keep" {
                Vec::new()
            } else {
                cfg.when
                    .iter()
                    .map(Condition::compile)
                    .collect::<Result<Vec<_>>>()
                    .with_context(ctx)?
            };
            steps.push((when, step));
        }
        let calc_fields = cfgs
            .iter()
            .filter(|c| !c.disabled && c.step == "calc")
            .filter_map(|c| c.field.clone())
            .collect();
        Ok(Pipeline { steps, calc_fields })
    }

    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }

    /// Names of the fields computed by enabled `calc` steps.
    pub fn calc_fields(&self) -> impl Iterator<Item = &str> {
        self.calc_fields.iter().map(String::as_str)
    }

    /// Runs every step; returns the surviving records and how many were dropped.
    pub fn run(&self, records: Vec<Record>, source_name: &str) -> (Vec<Record>, usize) {
        let (records, stats) = self.run_with_stats(records, source_name);
        let dropped = stats.first().map_or(0, |s| s.input) - stats.last().map_or(0, |s| s.output);
        (records, dropped)
    }

    /// Like [`Pipeline::run`], reporting events in and out of every step.
    pub fn run_with_stats(&self, records: Vec<Record>, source_name: &str) -> (Vec<Record>, Vec<StepStats>) {
        self.run_marking(records, source_name, &crate::plugin::markers::MarkIndex::default())
    }

    /// [`Pipeline::run_with_stats`]; plugin transforms may mark records with
    /// the kinds in `marks` that their plugin declared.
    pub fn run_marking(
        &self,
        mut records: Vec<Record>,
        source_name: &str,
        marks: &crate::plugin::markers::MarkIndex,
    ) -> (Vec<Record>, Vec<StepStats>) {
        let ctx = TransformCtx::with_marks(source_name, marks);
        let mut stats = Vec::with_capacity(self.steps.len());
        for (when, step) in &self.steps {
            let input = records.len();
            let mut errors = 0;
            let applies = |r: &Record| when.iter().all(|c| c.eval(r, source_name));
            match step {
                Step::Transform(t) if t.batched() => records = run_batched(t.as_ref(), records, &applies, &ctx),
                Step::Transform(t) => records.retain_mut(|r| !applies(r) || t.apply(r, &ctx) == Verdict::Keep),
                Step::Dedupe { fields, window_ms } => {
                    records = dedupe(records, fields, *window_ms, source_name, &applies);
                }
                Step::Script(script) => {
                    let mut runner = script.runner(source_name);
                    records.retain_mut(|r| {
                        // A failing record is kept as it was and counted.
                        !applies(r) || runner.apply(r).unwrap_or_else(|_| (errors += 1, Verdict::Keep).1) == Verdict::Keep
                    });
                }
            }
            stats.push(StepStats {
                input,
                output: records.len(),
                errors,
            });
        }
        (records, stats)
    }
}

/// Runs a [`RecordTransform::batched`] step: the records its conditions
/// select go to it in one call; the others pass through; order is kept.
fn run_batched(t: &dyn RecordTransform, records: Vec<Record>, applies: &dyn Fn(&Record) -> bool, ctx: &TransformCtx) -> Vec<Record> {
    let selected: Vec<bool> = records.iter().map(applies).collect();
    let (mut batch, mut rest): (Vec<Record>, Vec<Record>) = (Vec::new(), Vec::new());
    for (r, s) in records.into_iter().zip(&selected) {
        if *s {
            batch.push(r);
        } else {
            rest.push(r);
        }
    }
    let verdicts = t.apply_batch(&mut batch, ctx);
    let mut kept = batch
        .into_iter()
        .enumerate()
        .map(|(i, r)| (verdicts.get(i).copied().unwrap_or(Verdict::Keep) == Verdict::Keep).then_some(r));
    let mut rest = rest.into_iter();
    selected
        .into_iter()
        .filter_map(|s| if s { kept.next().flatten() } else { rest.next() })
        .collect()
}

fn need<'a>(v: &'a Option<String>, what: &str) -> Result<&'a str> {
    v.as_deref().filter(|s| !s.is_empty()).ok_or_else(|| anyhow!("needs `{what}`"))
}

fn compile_step(cfg: &StepCfg, transforms: &HashMap<String, Arc<dyn RecordTransform>>) -> Result<Step> {
    let t: Arc<dyn RecordTransform> = match cfg.step.as_str() {
        "drop" => Arc::new(Fixed("drop", Verdict::Drop)),
        // `keep` drops what does NOT match, so its condition is inverted at run time.
        "keep" => return keep_step(cfg),
        "mask" => Arc::new(Mask {
            field: cfg.field.clone().unwrap_or_else(|| "@message".into()),
            regex: user_regex::compile(need(&cfg.regex, "regex")?, false)?,
            replace: cfg.replace.clone().unwrap_or_else(|| "***".into()),
        }),
        "remove_fields" => {
            if cfg.fields.is_empty() {
                return Err(anyhow!("needs `fields`"));
            }
            Arc::new(RemoveFields(cfg.fields.clone()))
        }
        "rename" => Arc::new(Rename { from: need(&cfg.from, "from")?.into(), to: need(&cfg.to, "to")?.into() }),
        "set" => Arc::new(Set { field: need(&cfg.field, "field")?.into(), value: cfg.value.clone().unwrap_or_default() }),
        "set_level" => {
            let l = need(&cfg.level, "level")?;
            Arc::new(SetLevel(Level::parse(l).ok_or_else(|| anyhow!("unknown level \"{l}\""))?))
        }
        "truncate" => Arc::new(Truncate {
            field: cfg.field.clone().unwrap_or_else(|| "@message".into()),
            max: cfg.max.filter(|m| *m > 0).ok_or_else(|| anyhow!("needs `max` > 0"))?,
        }),
        "dedupe" => {
            let window = need(&cfg.window, "window")?;
            let window_ms = parse_bucket(window)?.ok_or_else(|| anyhow!("`window` must be a duration like \"5s\""))?;
            let fields = if cfg.fields.is_empty() { vec!["@message".into()] } else { cfg.fields.clone() };
            return Ok(Step::Dedupe { fields, window_ms });
        }
        "parse_json" => Arc::new(ParseJson {
            field: cfg.field.clone().unwrap_or_else(|| "@message".into()),
            prefix: cfg.prefix.clone().unwrap_or_default(),
        }),
        "parse_kv" => Arc::new(ParseKv {
            field: cfg.field.clone().unwrap_or_else(|| "@message".into()),
            prefix: cfg.prefix.clone().unwrap_or_default(),
        }),
        "lookup" => Arc::new(Lookup { field: need(&cfg.field, "field")?.into(), table: lookup_table(cfg)? }),
        "shift_time" => Arc::new(ShiftTime(parse_signed_ms(need(&cfg.by, "by")?)?)),
        "calc" => Arc::new(calc_step(cfg)?),
        "script" => {
            let script = crate::script::Script::compile(cfg.script.as_deref().unwrap_or_default()).map_err(|e| anyhow!("`script` {e}"))?;
            return Ok(Step::Script(Arc::new(script)));
        }
        "plugin" => {
            let id = need(&cfg.id, "id")?;
            transforms.get(id).cloned().ok_or_else(|| anyhow!("no plugin transform \"{id}\" is installed"))?
        }
        other => {
            return Err(anyhow!(
                "unknown step \"{other}\" (use drop, keep, mask, dedupe, remove_fields, rename, set, set_level, truncate, parse_json, parse_kv, lookup, shift_time, calc, script, plugin)"
            ))
        }
    };
    Ok(Step::Transform(t))
}

/// `keep` = drop everything that does not satisfy all `when` conditions. Its
/// conditions are moved into the step so the outer `when` is always true.
fn keep_step(cfg: &StepCfg) -> Result<Step> {
    if cfg.when.is_empty() {
        return Err(anyhow!("needs `when` conditions"));
    }
    let when = cfg.when.iter().map(Condition::compile).collect::<Result<Vec<_>>>()?;
    Ok(Step::Transform(Arc::new(Keep(when))))
}

struct Fixed(&'static str, Verdict);
impl RecordTransform for Fixed {
    fn id(&self) -> &str {
        self.0
    }
    fn apply(&self, _: &mut Record, _: &TransformCtx) -> Verdict {
        self.1
    }
}

struct Keep(Vec<Condition>);
impl RecordTransform for Keep {
    fn id(&self) -> &str {
        "keep"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        if self.0.iter().all(|c| c.eval(r, ctx.source_name)) {
            Verdict::Keep
        } else {
            Verdict::Drop
        }
    }
}

struct Mask {
    field: String,
    regex: Regex,
    replace: String,
}
impl RecordTransform for Mask {
    fn id(&self) -> &str {
        "mask"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        if let Some(text) = r.field_text(&self.field, ctx.source_name) {
            let masked = self.regex.replace_all(&text, self.replace.as_str());
            if masked != text {
                let masked = masked.into_owned();
                set_text(r, &self.field, masked);
            }
        }
        Verdict::Keep
    }
}

struct RemoveFields(Vec<String>);
impl RecordTransform for RemoveFields {
    fn id(&self) -> &str {
        "remove_fields"
    }
    fn apply(&self, r: &mut Record, _: &TransformCtx) -> Verdict {
        for f in &self.0 {
            match f.as_str() {
                "@exception" => r.exception = None,
                "@template" => r.template = None,
                other => {
                    r.fields.shift_remove(other);
                }
            }
        }
        Verdict::Keep
    }
}

struct Rename {
    from: String,
    to: String,
}
impl RecordTransform for Rename {
    fn id(&self) -> &str {
        "rename"
    }
    fn apply(&self, r: &mut Record, _: &TransformCtx) -> Verdict {
        if let Some(v) = r.fields.shift_remove(&self.from) {
            r.fields.insert(self.to.clone(), v);
        }
        Verdict::Keep
    }
}

struct Set {
    field: String,
    value: String,
}
impl RecordTransform for Set {
    fn id(&self) -> &str {
        "set"
    }
    fn apply(&self, r: &mut Record, _: &TransformCtx) -> Verdict {
        set_text(r, &self.field, self.value.clone());
        Verdict::Keep
    }
}

struct SetLevel(Level);
impl RecordTransform for SetLevel {
    fn id(&self) -> &str {
        "set_level"
    }
    fn apply(&self, r: &mut Record, _: &TransformCtx) -> Verdict {
        r.level = self.0;
        Verdict::Keep
    }
}

struct Truncate {
    field: String,
    max: usize,
}
impl RecordTransform for Truncate {
    fn id(&self) -> &str {
        "truncate"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        if let Some(text) = r.field_text(&self.field, ctx.source_name) {
            if text.chars().count() > self.max {
                let mut cut: String = text.chars().take(self.max).collect();
                cut.push('…');
                set_text(r, &self.field, cut);
            }
        }
        Verdict::Keep
    }
}

/// Expands a JSON object found in a field (the whole value, or from its first
/// `{`) into fields; nested objects become dotted names.
struct ParseJson {
    field: String,
    prefix: String,
}
impl RecordTransform for ParseJson {
    fn id(&self) -> &str {
        "parse_json"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        let Some(text) = r.field_text(&self.field, ctx.source_name) else {
            return Verdict::Keep;
        };
        let parsed = serde_json::from_str::<Value>(text.trim())
            .ok()
            .or_else(|| text.find('{').and_then(|i| serde_json::from_str::<Value>(text[i..].trim()).ok()));
        if let Some(Value::Object(obj)) = parsed {
            for (k, v) in obj {
                flatten(&mut r.fields, format!("{}{k}", self.prefix), v);
            }
        }
        Verdict::Keep
    }
}

fn flatten(fields: &mut serde_json::Map<String, Value>, key: String, v: Value) {
    match v {
        Value::Object(inner) if !inner.is_empty() => {
            for (k, v) in inner {
                flatten(fields, format!("{key}.{k}"), v);
            }
        }
        other => {
            fields.insert(key, other);
        }
    }
}

/// Expands `key=value` and `key="quoted value"` pairs into fields.
struct ParseKv {
    field: String,
    prefix: String,
}
fn kv_regex() -> &'static Regex {
    static RE: std::sync::OnceLock<Regex> = std::sync::OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"([A-Za-z_][\w.\-]*)=(?:"([^"]*)"|([^\s,;]+))"#).unwrap())
}
impl RecordTransform for ParseKv {
    fn id(&self) -> &str {
        "parse_kv"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        let Some(text) = r.field_text(&self.field, ctx.source_name).map(|t| t.into_owned()) else {
            return Verdict::Keep;
        };
        for c in kv_regex().captures_iter(&text) {
            let value = c.get(2).or(c.get(3)).map_or("", |m| m.as_str());
            r.fields
                .insert(format!("{}{}", self.prefix, &c[1]), crate::model::auto_value(value));
        }
        Verdict::Keep
    }
}

/// Adds fields from a table keyed by the value of `field`.
struct Lookup {
    field: String,
    table: HashMap<String, Vec<(String, Value)>>,
}
impl RecordTransform for Lookup {
    fn id(&self) -> &str {
        "lookup"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        let hit = r.field_text(&self.field, ctx.source_name).and_then(|k| self.table.get(k.as_ref()));
        if let Some(values) = hit {
            for (k, v) in values.clone() {
                r.fields.insert(k, v);
            }
        }
        Verdict::Keep
    }
}

fn lookup_table(cfg: &StepCfg) -> Result<HashMap<String, Vec<(String, Value)>>> {
    let mut out = HashMap::new();
    if let Some(table) = &cfg.table {
        for (key, row) in table {
            let mut values: Vec<(String, Value)> = row
                .iter()
                .map(|(k, v)| (k.clone(), serde_json::to_value(v).unwrap_or(Value::Null)))
                .collect();
            values.sort_by(|a, b| a.0.cmp(&b.0));
            out.insert(key.clone(), values);
        }
    }
    if let Some(file) = &cfg.file {
        let mut reader = csv::Reader::from_path(file).with_context(|| format!("cannot read lookup file {file}"))?;
        let headers: Vec<String> = reader.headers()?.iter().map(str::to_string).collect();
        let key_col = match &cfg.key {
            Some(k) => headers
                .iter()
                .position(|h| h == k)
                .ok_or_else(|| anyhow!("lookup file has no column \"{k}\""))?,
            None => 0,
        };
        for row in reader.records() {
            let row = row?;
            let Some(key) = row.get(key_col) else { continue };
            let values = headers
                .iter()
                .enumerate()
                .filter(|(i, _)| *i != key_col)
                .filter_map(|(i, h)| {
                    row.get(i)
                        .filter(|v| !v.is_empty())
                        .map(|v| (h.clone(), crate::model::auto_value(v)))
                })
                .collect();
            out.insert(key.to_string(), values);
        }
    }
    if out.is_empty() {
        return Err(anyhow!("needs a non-empty `table` or `file`"));
    }
    Ok(out)
}

/// Moves timestamps by a fixed offset, e.g. to correct a machine's clock skew.
struct ShiftTime(i64);
impl RecordTransform for ShiftTime {
    fn id(&self) -> &str {
        "shift_time"
    }
    fn apply(&self, r: &mut Record, _: &TransformCtx) -> Verdict {
        if let Some(ts) = r.ts.as_mut() {
            *ts += self.0;
        }
        Verdict::Keep
    }
}

/// Writes the value of an expression to a field; a null result leaves the record as it is.
struct Calc {
    field: String,
    expr: CalcExpr,
}
impl RecordTransform for Calc {
    fn id(&self) -> &str {
        "calc"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        if let Some(v) = self.expr.eval_json(r, ctx.source_name) {
            r.fields.insert(self.field.clone(), v);
        }
        Verdict::Keep
    }
}

/// Parses a `calc` step once, when the profile is compiled.
fn calc_step(cfg: &StepCfg) -> Result<Calc> {
    let field = need(&cfg.field, "field")?;
    if field.starts_with('@') {
        return Err(anyhow!(
            "`field` \"{field}\" is a built-in; calc writes a field of its own (use a name without @)"
        ));
    }
    let text = need(&cfg.expr, "expr")?;
    let expr = CalcExpr::parse(text).map_err(|e| anyhow!("`expr` {}", describe_error(text, &e)))?;
    Ok(Calc { field: field.into(), expr })
}

/// "error at column N: message" with the expression and a caret under the position.
fn describe_error(text: &str, e: &QlError) -> String {
    let at = e.offset.min(text.len());
    let column = text.char_indices().take_while(|(i, _)| *i < at).count() + 1;
    let message = format!("error at column {column}: {}", e.message);
    if text.len() > MAX_CALC_LEN {
        return message;
    }
    // Control characters (newlines, tabs) become spaces so the caret lines up.
    let shown: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    format!("{message}\n  {shown}\n  {}^", " ".repeat(column - 1))
}

/// "+2.4s", "-150ms", "1m" -> milliseconds.
fn parse_signed_ms(s: &str) -> Result<i64> {
    let s = s.trim();
    let (sign, rest) = match s.strip_prefix('-') {
        Some(r) => (-1.0, r),
        None => (1.0, s.strip_prefix('+').unwrap_or(s)),
    };
    let idx = rest.find(|c: char| !(c.is_ascii_digit() || c == '.')).unwrap_or(rest.len());
    let n: f64 = rest[..idx]
        .parse()
        .map_err(|_| anyhow!("`by` must be like \"+2.4s\" or \"-150ms\""))?;
    let unit = match &rest[idx..] {
        "ms" => 1.0,
        "s" => 1000.0,
        "m" | "min" => 60_000.0,
        "h" => 3_600_000.0,
        other => return Err(anyhow!("unknown unit \"{other}\" in `by` (use ms, s, m, h)")),
    };
    Ok((sign * n * unit).round() as i64)
}

/// Writes text to a field or to the message/exception/template.
fn set_text(r: &mut Record, field: &str, value: String) {
    match field {
        "@message" => r.message = value,
        "@exception" => r.exception = Some(value),
        "@template" => r.template = Some(value),
        "@level" => {
            if let Some(l) = Level::parse(&value) {
                r.level = l;
            }
        }
        other => {
            r.fields.insert(other.to_string(), Value::String(value));
        }
    }
}

/// Collapses records with the same key within `window_ms` of the first one
/// into that first record, counting them in [`REPEAT_COUNT`].
fn dedupe(records: Vec<Record>, fields: &[String], window_ms: i64, source: &str, applies: &dyn Fn(&Record) -> bool) -> Vec<Record> {
    let mut out: Vec<Record> = Vec::with_capacity(records.len());
    // key -> (index in out, time of the first record of the run)
    let mut open: HashMap<String, (usize, i64)> = HashMap::new();
    for r in records {
        let (Some(ts), true) = (r.ts, applies(&r)) else {
            out.push(r);
            continue;
        };
        let key = crate::query::group_key(fields, &r, source);
        if let Some((idx, first)) = open.get(&key).copied() {
            if ts - first <= window_ms {
                let kept = &mut out[idx];
                let n = kept.fields.get(REPEAT_COUNT).and_then(Value::as_u64).unwrap_or(1) + 1;
                kept.fields.insert(REPEAT_COUNT.into(), Value::from(n));
                continue;
            }
        }
        open.insert(key, (out.len(), ts));
        out.push(r);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rec(ts: i64, level: Level, msg: &str, ctx: &str) -> Record {
        let mut r = Record::new(1);
        r.ts = Some(ts);
        r.level = level;
        r.message = msg.into();
        r.fields.insert("SourceContext".into(), ctx.into());
        r.fields.insert("ThreadId".into(), 7.into());
        r
    }

    fn pipeline(toml: &str) -> Result<Pipeline> {
        #[derive(Deserialize)]
        struct P {
            pipeline: Vec<StepCfg>,
        }
        let p: P = toml::from_str(toml).unwrap();
        Pipeline::compile(&p.pipeline, &HashMap::new())
    }

    fn sample() -> Vec<Record> {
        vec![
            rec(1_000, Level::Information, "health ok", "HealthChecks"),
            rec(2_000, Level::Warning, "login password=hunter2 user=ann", "Auth"),
            rec(3_000, Level::Error, "boom", "Payments"),
            rec(3_500, Level::Error, "boom", "Payments"),
            rec(4_000, Level::Error, "boom", "Payments"),
            rec(20_000, Level::Error, "boom", "Payments"),
        ]
    }

    #[test]
    fn drop_mask_dedupe_and_tidy() {
        let p = pipeline(
            r#"
[[pipeline]]
step = "drop"
when = [{ field = "SourceContext", op = "contains", value = "health" }]
[[pipeline]]
step = "mask"
regex = '(password)=\S+'
replace = "$1=***"
[[pipeline]]
step = "dedupe"
window = "5s"
when = [{ field = "@level", op = "eq", value = "Error" }]
[[pipeline]]
step = "remove_fields"
fields = ["ThreadId", "@exception", "@template"]
[[pipeline]]
step = "rename"
from = "SourceContext"
to = "Logger"
"#,
        )
        .unwrap();
        assert!(!p.is_empty());
        let (out, dropped) = p.run(sample(), "f");
        assert_eq!(dropped, 3);
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].message, "login password=*** user=ann");
        assert_eq!(out[1].fields[REPEAT_COUNT], 3);
        assert!(!out[2].fields.contains_key(REPEAT_COUNT), "outside the window starts a new run");
        assert!(out
            .iter()
            .all(|r| !r.fields.contains_key("ThreadId") && r.fields.contains_key("Logger")));
    }

    #[test]
    fn keep_set_level_truncate_and_set() {
        let p = pipeline(
            r#"
[[pipeline]]
step = "keep"
when = [{ field = "@level", op = "gt", value = "2" }]
[[pipeline]]
step = "set_level"
level = "Fatal"
when = [{ field = "SourceContext", op = "eq", value = "Auth" }]
[[pipeline]]
step = "truncate"
max = 3
[[pipeline]]
step = "set"
field = "Team"
value = "payments"
[[pipeline]]
step = "set"
field = "@level"
value = "nonsense"
[[pipeline]]
step = "set"
field = "@exception"
value = "x"
[[pipeline]]
step = "set"
field = "@template"
value = "t"
"#,
        )
        .unwrap();
        let (out, dropped) = p.run(sample(), "f");
        assert_eq!(dropped, 1);
        assert_eq!(out[0].level, Level::Fatal);
        assert_eq!(out[0].message, "log…");
        assert_eq!(out[1].message, "boo…");
        assert_eq!(out[1].fields["Team"], "payments");
        assert_eq!((out[1].exception.as_deref(), out[1].template.as_deref()), (Some("x"), Some("t")));
        let (out, _) = pipeline("[[pipeline]]\nstep='set'\nfield='@level'\nvalue='Debug'")
            .unwrap()
            .run(sample(), "f");
        assert_eq!(out[0].level, Level::Debug);
        let (out, _) = pipeline("[[pipeline]]\nstep='mask'\nfield='Missing'\nregex='x'")
            .unwrap()
            .run(sample(), "f");
        assert_eq!(out.len(), 6);
        // Truncating a field the record does not have leaves it untouched.
        let (out, _) = pipeline("[[pipeline]]\nstep='truncate'\nfield='Missing'\nmax=1")
            .unwrap()
            .run(sample(), "f");
        assert!(out.iter().all(|r| !r.fields.contains_key("Missing")));
        assert_eq!(out[1].message, "login password=hunter2 user=ann");
    }

    #[test]
    fn dedupe_defaults_and_untimed_records() {
        let mut records = sample();
        records.push(Record::new(9));
        records.push(Record::new(10));
        let (out, dropped) = pipeline("[[pipeline]]\nstep='dedupe'\nwindow='1m'").unwrap().run(records, "f");
        assert_eq!(dropped, 3, "the four 'boom' within a minute collapse; untimed records are kept");
        assert_eq!(out.len(), 5);
        assert_eq!(out[2].fields[REPEAT_COUNT], 4);
    }

    #[test]
    fn plugin_steps() {
        struct Upper;
        impl RecordTransform for Upper {
            fn id(&self) -> &str {
                "upper"
            }
            fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
                r.message = format!("{}:{}", ctx.source_name, r.message.to_uppercase());
                Verdict::Keep
            }
        }
        let mut t: HashMap<String, Arc<dyn RecordTransform>> = HashMap::new();
        t.insert("upper".into(), Arc::new(Upper));
        let cfg = vec![StepCfg {
            step: "plugin".into(),
            id: Some("upper".into()),
            ..Default::default()
        }];
        assert_eq!(t["upper"].id(), "upper");
        let (out, _) = Pipeline::compile(&cfg, &t).unwrap().run(sample(), "src");
        assert_eq!(out[2].message, "src:BOOM");
        let missing = vec![StepCfg {
            step: "plugin".into(),
            id: Some("nope".into()),
            ..Default::default()
        }];
        assert!(Pipeline::compile(&missing, &t)
            .err()
            .unwrap()
            .to_string()
            .contains("pipeline step 1"));
    }

    #[test]
    fn rejects_bad_steps() {
        for bad in [
            "step='explode'",
            "step='keep'",
            "step='mask'",
            "step='mask'\nregex='('",
            "step='remove_fields'",
            "step='rename'\nfrom='a'",
            "step='set'",
            "step='set_level'\nlevel='loud'",
            "step='set_level'",
            "step='truncate'",
            "step='truncate'\nmax=0",
            "step='dedupe'",
            "step='dedupe'\nwindow='auto'",
            "step='dedupe'\nwindow='5x'",
            "step='plugin'",
            "step='drop'\nwhen=[{field='a',op='nope'}]",
            "step='keep'\nwhen=[{field='a',op='nope'}]",
        ] {
            assert!(pipeline(&format!("[[pipeline]]\n{bad}")).is_err(), "{bad}");
        }
        assert_eq!(Fixed("drop", Verdict::Drop).id(), "drop");
        assert_eq!(Keep(vec![]).id(), "keep");
        for t in [
            Box::new(RemoveFields(vec![])) as Box<dyn RecordTransform>,
            Box::new(Rename {
                from: "a".into(),
                to: "b".into(),
            }),
            Box::new(Set {
                field: "a".into(),
                value: "b".into(),
            }),
            Box::new(SetLevel(Level::Debug)),
            Box::new(Truncate { field: "a".into(), max: 1 }),
            Box::new(Mask {
                field: "a".into(),
                regex: Regex::new("a").unwrap(),
                replace: "b".into(),
            }),
        ] {
            assert!(!t.id().is_empty());
        }
    }

    #[test]
    fn parse_json_kv_lookup_and_shift() {
        let mut r = rec(
            1_000,
            Level::Information,
            r#"Request done {"Status": 500, "User": {"Id": 7}}"#,
            "Api",
        );
        r.fields.insert("Raw".into(), r#"{"a": 1}"#.into());
        r.fields.insert("Kv".into(), r#"user=ann elapsed=532 note="two words" bad"#.into());
        r.fields.insert("MachineName".into(), "WEB01".into());
        let file = std::env::temp_dir().join(format!("imt-lookup-{}.csv", std::process::id()));
        std::fs::write(&file, "Machine,Owner,Empty\nWEB02,team-b,\n").unwrap();
        let p = pipeline(&format!(
            r#"
[[pipeline]]
step = "parse_json"
[[pipeline]]
step = "parse_json"
field = "Raw"
prefix = "raw."
[[pipeline]]
step = "parse_kv"
field = "Kv"
[[pipeline]]
step = "lookup"
field = "MachineName"
file = '{}'
key = "Machine"
table = {{ WEB01 = {{ Datacenter = "AMS", Rack = 4 }} }}
[[pipeline]]
step = "shift_time"
by = "+2.5s"
when = [{{ field = "MachineName", op = "eq", value = "WEB01" }}]
[[pipeline]]
step = "shift_time"
by = "-1m"
when = [{{ field = "MachineName", op = "eq", value = "nobody" }}]
"#,
            file.display()
        ))
        .unwrap();
        let mut other = rec(5, Level::Information, "no json {here", "Api");
        other.fields.insert("MachineName".into(), "WEB02".into());
        let (out, stats) = p.run_with_stats(vec![r, other, Record::new(3)], "f");
        assert_eq!(stats.len(), 6);
        assert!(stats.iter().all(|s| s.input == 3 && s.output == 3));
        let r = &out[0];
        assert_eq!(r.fields["Status"], 500);
        assert_eq!(r.fields["User.Id"], 7);
        assert_eq!(r.fields["raw.a"], 1);
        assert_eq!(
            (r.fields["user"].clone(), r.fields["elapsed"].clone(), r.fields["note"].clone()),
            ("ann".into(), 532.into(), "two words".into())
        );
        assert_eq!((r.fields["Datacenter"].clone(), r.fields["Rack"].clone()), ("AMS".into(), 4.into()));
        assert_eq!(r.ts, Some(3_500));
        assert_eq!(out[1].fields["Owner"], "team-b");
        assert!(!out[1].fields.contains_key("Empty"));
        assert_eq!(out[1].ts, Some(5));
        assert_eq!(out[2].ts, None);
        std::fs::remove_file(file).unwrap();
        assert_eq!(parse_signed_ms("-150ms").unwrap(), -150);
        assert_eq!(parse_signed_ms("1h").unwrap(), 3_600_000);
        assert_eq!(parse_signed_ms("2min").unwrap(), 120_000);
    }

    #[test]
    fn rejects_bad_enrichment_steps() {
        for bad in [
            "step='lookup'\nfield='a'",
            "step='lookup'\ntable={}",
            "step='lookup'\nfield='a'\nfile='/definitely/missing.csv'",
            "step='shift_time'",
            "step='shift_time'\nby='soon'",
            "step='shift_time'\nby='5 weeks'",
        ] {
            assert!(pipeline(&format!("[[pipeline]]\n{bad}")).is_err(), "{bad}");
        }
        let file = std::env::temp_dir().join(format!("imt-lookup-bad-{}.csv", std::process::id()));
        std::fs::write(&file, "A,B\nx,1\n").unwrap();
        assert!(pipeline(&format!(
            "[[pipeline]]\nstep='lookup'\nfield='a'\nfile='{}'\nkey='Nope'",
            file.display()
        ))
        .is_err());
        std::fs::remove_file(file).unwrap();
        for t in [
            Box::new(ParseJson {
                field: "a".into(),
                prefix: String::new(),
            }) as Box<dyn RecordTransform>,
            Box::new(ParseKv {
                field: "a".into(),
                prefix: String::new(),
            }),
            Box::new(Lookup {
                field: "a".into(),
                table: HashMap::new(),
            }),
            Box::new(ShiftTime(1)),
        ] {
            assert!(!t.id().is_empty());
            let mut r = Record::new(1);
            assert_eq!(t.apply(&mut r, &TransformCtx::new("s")), Verdict::Keep);
        }
    }

    #[test]
    fn disabled_steps_are_skipped_but_counted() {
        let p = pipeline("[[pipeline]]\nstep='drop'\ndisabled=true\n[[pipeline]]\nstep='nonsense'\ndisabled=true").unwrap();
        let (out, stats) = p.run_with_stats(sample(), "f");
        assert_eq!(out.len(), 6);
        assert_eq!(
            stats,
            vec![
                StepStats {
                    input: 6,
                    output: 6,
                    errors: 0
                };
                2
            ]
        );
    }

    #[test]
    fn calc_computes_fields() {
        let mut records = sample();
        records[0].fields.insert("ElapsedMs".into(), 1500.into());
        records[1].fields.insert("ElapsedMs".into(), "250".into());
        let p = pipeline(
            "[[pipeline]]\nstep='calc'\nfield='ElapsedSec'\nexpr='ElapsedMs / 1000'\n\
             [[pipeline]]\nstep='calc'\nfield='Slow'\nexpr='ElapsedSec > 1'\n\
             [[pipeline]]\nstep='calc'\nfield='Where'\nexpr='lower(SourceContext) + \"#\" + ThreadId'\n\
             when=[{ field='@level', op='eq', value='Error' }]",
        )
        .unwrap();
        let (out, stats) = p.run_with_stats(records, "f");
        assert_eq!(stats.len(), 3);
        assert_eq!(out[0].fields["ElapsedSec"], 1.5);
        assert_eq!(out[0].fields["Slow"], true);
        assert_eq!(out[1].fields["ElapsedSec"], 0.25, "numeric text is used as a number");
        assert_eq!(out[1].fields["Slow"], false);
        // Missing inputs write nothing; `when` limits the step.
        assert!(!out[2].fields.contains_key("ElapsedSec") && !out[2].fields.contains_key("Slow"));
        assert!(!out[0].fields.contains_key("Where"));
        assert_eq!(out[2].fields["Where"], "payments#7");
    }

    #[test]
    fn script_steps_transform_drop_and_count_errors() {
        let p = pipeline(
            r#"
[[pipeline]]
step = "script"
when = [{ field = "@level", op = "eq", value = "Error" }]
script = '''
if fields.ThreadId == 7 && message == "boom" && timestamp >= 4000 { drop(); }
fields.Seen = source;
'''
[[pipeline]]
step = "script"
script = 'if message.contains("password=") { message = 1; } else { level = "Fatal"; }'
"#,
        )
        .unwrap();
        let (out, stats) = p.run_with_stats(sample(), "f.log");
        // Two of the four errors are from 4 s on; `when` keeps the script off the others.
        assert_eq!(out.len(), 4);
        assert_eq!(
            stats[0],
            StepStats {
                input: 6,
                output: 4,
                errors: 0
            }
        );
        assert_eq!(out[2].fields["Seen"], "f.log");
        assert!(!out[0].fields.contains_key("Seen"));
        // The login record fails (message must stay a string): kept unchanged and counted.
        assert_eq!(
            stats[1],
            StepStats {
                input: 4,
                output: 4,
                errors: 1
            }
        );
        assert_eq!(
            (out[1].level, out[1].message.as_str()),
            (Level::Warning, "login password=hunter2 user=ann")
        );
        assert!(out.iter().filter(|r| r.message != out[1].message).all(|r| r.level == Level::Fatal));
    }

    #[test]
    fn script_errors_are_profile_errors() {
        let err = |step: &str| format!("{:#}", pipeline(&format!("[[pipeline]]\nstep='script'\n{step}")).err().unwrap());
        assert_eq!(err(""), "pipeline step 1 (script): `script` is empty");
        let e = err("script = '''\nlet a = 1;\nlevel = ;\n'''");
        assert!(
            e.starts_with("pipeline step 1 (script): `script` syntax error at line 2, column 9: "),
            "{e}"
        );
    }

    #[test]
    fn calc_null_keeps_an_existing_value() {
        let mut records = sample();
        records[0].fields.insert("Ratio".into(), "keep me".into());
        let p = pipeline("[[pipeline]]\nstep='calc'\nfield='Ratio'\nexpr='Missing / 2'").unwrap();
        let (out, _) = p.run(records, "f");
        assert_eq!(out[0].fields["Ratio"], "keep me");
        assert_eq!(out.len(), 6, "calc never drops events");
    }

    #[test]
    fn calc_rejects_bad_configuration() {
        let err = |step: &str| format!("{:#}", pipeline(&format!("[[pipeline]]\nstep='calc'\n{step}")).err().unwrap());
        assert!(err("expr='1'").contains("needs `field`"));
        assert!(err("field='X'").contains("needs `expr`"));
        assert!(err("field='@message'\nexpr='1'").contains("built-in"));
        let e = err("field='X'\nexpr='ElapsedMs / foo(1)'");
        assert!(
            e.starts_with("pipeline step 1 (calc): `expr` error at column 13: unknown function 'foo'"),
            "{e}"
        );
        assert!(e.ends_with("\n  ElapsedMs / foo(1)\n              ^"), "{e}");
        // Columns count characters; a newline in a multi-line string shows as a space.
        let e = err("field='X'\nexpr=\"\"\"é +\n(\"\"\"");
        assert!(e.ends_with("error at column 6: expected a value\n  é + (\n       ^"), "{e}");
        let long = format!("1{}", "+1".repeat(MAX_CALC_LEN));
        let e = err(&format!("field='X'\nexpr='{long}'"));
        assert!(e.contains("too long") && !e.contains('^'), "{e}");
        assert_eq!(
            Calc {
                field: "X".into(),
                expr: CalcExpr::parse("1").unwrap()
            }
            .id(),
            "calc"
        );
    }
}
