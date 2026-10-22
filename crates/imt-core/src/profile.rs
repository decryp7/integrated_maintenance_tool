//! Analysis profiles: how the workspace analyses events whatever their format (files -> instance ->
//! extract -> pipeline -> markers -> charts). Formats are parsers ([`crate::parser`], ADR 0026). See
//! `default_profile.toml` for the schema.

use crate::condition::{Condition, ConditionCfg};
use crate::model::{auto_value, Record};
use crate::parser::{FormatCfg, Parser};
use crate::ports::ParserRef;
use crate::template::output_template_to_regex;
use crate::time::TimestampParser;
use anyhow::{anyhow, Context, Result};
use globset::{Glob, GlobSet, GlobSetBuilder};
use regex::Regex;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

pub const DEFAULT_PROFILE_TOML: &str = include_str!("default_profile.toml");

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Profile {
    pub name: String,
    pub files: FilesCfg,
    pub instance: InstanceCfg,
    pub activity: ActivityCfg,
    #[serde(default)]
    pub extract: Vec<ExtractCfg>,
    #[serde(default)]
    pub marker: Vec<MarkerCfg>,
    /// Marker rules the user switched off: kept in the profile (so they can be
    /// switched back on at their old position) but never compiled or matched.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub imt_disabled_markers: Vec<DisabledMarkerCfg>,
    #[serde(default)]
    pub chart: Vec<ChartCfg>,
    #[serde(default)]
    pub pipeline: Vec<crate::pipeline::StepCfg>,
    /// Custom marker icons (SVG or PNG), referenced by name from `[[marker]] icon`.
    #[serde(default)]
    pub icon: Vec<crate::icons::IconCfg>,
    /// Live tail: detectors, alert rules and the clock of live windows.
    #[serde(default)]
    pub live: crate::workspace::LiveCfg,
    /// Disables or restyles marker kinds by name, typically ones contributed
    /// by plugins (`<plugin-id>/<kind>`).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub marker_override: Vec<MarkerOverrideCfg>,
    /// Redaction of secrets and personal data: the redacted view, exports,
    /// reports and bundles.
    #[serde(default)]
    pub redact: crate::redact::RedactCfg,
    /// Dashboards: grids of chart, stat, table, top, alert and annotation tiles.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dashboard: Vec<crate::dashboard::DashboardCfg>,
    /// Lenses: named view presets (query, columns, highlights, chart) the grid switches between.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub lens: Vec<crate::lens::LensCfg>,
    /// Runs between lifecycle markers (start … stop), paired per lane.
    #[serde(default)]
    pub span: Vec<crate::spans::SpanCfg>,
    /// `[[route]]`: splits files into logical sources by event fields (log servers, shared files).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub route: Vec<crate::route::RouteCfg>,
}

/// `[[marker_override]]`: switches a marker kind off or changes its look.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct MarkerOverrideCfg {
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub enabled: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub shape: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesCfg {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
    /// Text encoding of the files: `auto` (detected per file) or a label such as `Shift_JIS` or `windows-1252`.
    #[serde(default = "default_encoding")]
    pub encoding: String,
}

fn default_encoding() -> String {
    "auto".into()
}

impl FilesCfg {
    /// The encoding choice (an unknown label was refused when the profile compiled).
    pub fn encoding_choice(&self) -> crate::ingest::EncodingChoice {
        crate::ingest::EncodingChoice::parse(&self.encoding).unwrap_or_default()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceCfg {
    pub key: Vec<String>,
    pub candidates: Vec<String>,
}

/// Which fields describe what users were doing (for the Moment view).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActivityCfg {
    /// Who: the first present field names the user or session.
    pub user: Vec<String>,
    /// What: the first present field names the action; else the message.
    pub action: Vec<String>,
    /// Numeric fields summarised as metrics (besides PerfMon Counter/Value records).
    pub metrics: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExtractCfg {
    pub name: String,
    #[serde(default = "default_from")]
    pub from: String,
    pub regex: String,
    #[serde(default)]
    pub when: Vec<ConditionCfg>,
    /// Output field -> how to convert the capture of the same name. Empty =
    /// every named group, auto-typed.
    #[serde(default)]
    pub fields: BTreeMap<String, FieldSpec>,
}

fn default_from() -> String {
    "@message".into()
}

/// How an extracted value is converted (`type` of an extractor field). Unknown names are refused.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    /// Numbers become numbers, the rest stays text (not a declared column).
    #[default]
    Auto,
    String,
    Number,
    Integer,
    Bool,
    /// A size, converted to `to` (MB by default).
    Bytes,
    /// A duration, converted to `to` (ms by default).
    Duration,
}

impl FieldType {
    pub const NAMES: [&'static str; 7] = ["auto", "string", "number", "integer", "bool", "bytes", "duration"];
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    #[serde(rename = "type", default)]
    pub kind: FieldType,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub unit_from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MarkerCfg {
    pub kind: String,
    #[serde(default)]
    pub label: Option<String>,
    #[serde(default)]
    pub lifecycle: bool,
    #[serde(default)]
    pub color: Option<String>,
    /// Icon shown on the timeline: one of [`MARKER_ICONS`] or a custom `[[icon]]`.
    #[serde(default)]
    pub icon: Option<String>,
    /// Generated badge: 1–3 characters (e.g. "1", "A", "DB") drawn in `shape`;
    /// takes precedence over `icon`.
    #[serde(default)]
    pub badge: Option<String>,
    /// Badge outline: circle (default), square, diamond or hexagon.
    #[serde(default)]
    pub shape: Option<String>,
    /// What the marker means; shown in the legend and tooltips.
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub when: Vec<ConditionCfg>,
}

/// A switched-off marker rule and the priority position it had among all rules.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DisabledMarkerCfg {
    #[serde(flatten)]
    pub rule: MarkerCfg,
    /// Index in the combined (active + disabled) rule order; absent = last.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub imt_index: Option<usize>,
}

/// Icons a marker may use on the timeline (the UI ships one glyph per name).
pub const MARKER_ICONS: &[&str] = &[
    "alert",
    "bug",
    "flag",
    "star",
    "pin",
    "bookmark",
    "info",
    "check",
    "x-circle",
    "user",
    "users",
    "lock",
    "key",
    "shield",
    "database",
    "network",
    "cloud",
    "server",
    "credit-card",
    "cart",
    "mail",
    "clock",
    "timer",
    "rocket",
    "power",
    "restart",
    "cpu",
    "memory",
    "disk",
    "file",
    "download",
    "upload",
    "link",
    "zap",
    "fire",
    "heart",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChartCfg {
    #[serde(default)]
    pub id: Option<String>,
    pub title: String,
    pub y: String,
    #[serde(default = "default_bucket")]
    pub bucket: String,
    #[serde(default)]
    pub split_by: Vec<String>,
    #[serde(rename = "type", default = "default_chart_type")]
    pub kind: String,
    #[serde(default)]
    pub unit: String,
    #[serde(default)]
    pub when: Vec<ConditionCfg>,
}

fn default_bucket() -> String {
    "auto".into()
}
fn default_chart_type() -> String {
    "line".into()
}

/// Error with the 1-based line it occurred on, for the profile editor.
#[derive(Debug, Clone)]
pub struct ProfileError {
    pub message: String,
    pub line: Option<usize>,
}

impl std::fmt::Display for ProfileError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.line {
            Some(l) => write!(f, "line {l}: {}", self.message),
            None => f.write_str(&self.message),
        }
    }
}

impl std::error::Error for ProfileError {}

impl Profile {
    /// Parses a profile; missing sections and keys fall back to the defaults.
    pub fn from_toml(text: &str) -> std::result::Result<Profile, ProfileError> {
        let user: toml::Table = toml::from_str(text).map_err(|e| ProfileError {
            message: e.message().to_string(),
            line: e.span().map(|s| text[..s.start.min(text.len())].lines().count().max(1)),
        })?;
        check_no_formats(&user, text)?;
        crate::spans::config::check_toml(&user, text)?;
        check_field_types(&user, text)?;
        let mut merged: toml::Table = toml::from_str(DEFAULT_PROFILE_TOML).expect("default profile is valid TOML");
        merge(&mut merged, user);
        toml::Value::Table(merged).try_into().map_err(|e: toml::de::Error| ProfileError {
            message: e.message().to_string(),
            line: None,
        })
    }

    pub fn default_profile() -> Profile {
        Profile::from_toml(DEFAULT_PROFILE_TOML).expect("default profile is valid")
    }

    /// This profile with `toml` merged over it (tables merge, other values replace), e.g. a project's
    /// profile overrides or a quick fix. Errors name the line of `toml`.
    pub fn with_override(&self, toml: &str) -> std::result::Result<Profile, ProfileError> {
        let over: toml::Table = toml::from_str(toml).map_err(|e| ProfileError {
            message: e.message().to_string(),
            line: e.span().map(|s| toml[..s.start.min(toml.len())].lines().count().max(1)),
        })?;
        let mut base = toml::Table::try_from(self).map_err(|e| ProfileError {
            message: format!("the active profile cannot be written as TOML: {e}"),
            line: None,
        })?;
        merge(&mut base, over);
        let text = toml::to_string(&base).map_err(|e| ProfileError {
            message: format!("the merged profile cannot be written as TOML: {e}"),
            line: None,
        })?;
        Profile::from_toml(&text)
    }
}

/// Refuses an extractor field `type` this build does not know, naming its line (engine review finding 20:
/// unknown values used to fall back to `auto` silently).
fn check_field_types(user: &toml::Table, text: &str) -> std::result::Result<(), ProfileError> {
    let extracts = user.get("extract").and_then(|e| e.as_array()).into_iter().flatten();
    for e in extracts {
        let fields = e.get("fields").and_then(|f| f.as_table()).into_iter().flatten();
        for (name, spec) in fields {
            let Some(kind) = spec.get("type") else { continue };
            let ok = kind.as_str().is_some_and(|k| FieldType::NAMES.contains(&k));
            if !ok {
                let shown = kind.to_string();
                let line = text.find(&shown).map(|i| text[..i].lines().count().max(1));
                return Err(ProfileError {
                    message: format!(
                        "extract field \"{name}\": unknown type {shown} (use {})",
                        FieldType::NAMES.join(", ")
                    ),
                    line,
                });
            }
        }
    }
    Ok(())
}

/// Refuses the `[parse]` section profiles had before formats became parser files (ADR 0026), naming its line.
fn check_no_formats(user: &toml::Table, text: &str) -> std::result::Result<(), ProfileError> {
    let Some(key) = ["parse", "format", "match"].into_iter().find(|k| user.contains_key(*k)) else {
        return Ok(());
    };
    let line = text
        .lines()
        .position(|l| {
            let t = l.trim_start();
            t.starts_with(&format!("[{key}")) || t.starts_with(key)
        })
        .map(|i| i + 1);
    Err(ProfileError {
        message: format!(
            "[{key}] is not part of a profile: formats are parsers now (Settings ▸ Parsers); move these keys to a parser file"
        ),
        line,
    })
}

/// Merges `over` into `base`: tables merge, other values replace.
pub(crate) fn merge(base: &mut toml::Table, over: toml::Table) {
    for (k, v) in over {
        match (base.get_mut(&k), v) {
            (Some(toml::Value::Table(b)), toml::Value::Table(o)) => merge(b, o),
            (_, v) => {
                base.insert(k, v);
            }
        }
    }
}

pub struct TextFormat {
    pub regex: Regex,
    /// True when the timestamp carries no date (console-style templates).
    pub time_only: bool,
}

pub struct Extractor {
    pub name: String,
    pub from: String,
    pub regex: Regex,
    pub when: Vec<Condition>,
    pub fields: Vec<(String, FieldSpec)>,
}

pub struct Marker {
    pub kind: String,
    /// Who defined the rule: `profile`, `builtin` or a plugin id.
    pub owner: String,
    /// Lifecycle meaning (start, stop, crash) of a plugin kind.
    pub role: Option<crate::plugin::markers::MarkerRole>,
    pub label: String,
    pub lifecycle: bool,
    pub color: Option<String>,
    pub icon: Option<String>,
    pub badge: Option<String>,
    pub shape: Option<String>,
    pub description: Option<String>,
    pub when: Vec<Condition>,
}

/// Plugin record transforms by id (what `{ step = "plugin" }` resolves against).
pub type Transforms = std::collections::HashMap<String, std::sync::Arc<dyn crate::plugin::RecordTransform>>;

/// A profile compiled with one parser (ADR 0026): every regex, glob and condition compiled. The parser's
/// extractors run before the profile's, its pipeline steps before the profile's pipeline.
pub struct CompiledProfile {
    pub profile: Profile,
    /// The parser's `[format]`: how files are read.
    pub format: FormatCfg,
    /// The parser this was compiled with.
    pub parser: ParserRef,
    pub include: GlobSet,
    pub exclude: GlobSet,
    pub text_formats: Vec<TextFormat>,
    /// All of `text_formats` in one set, to find the first format a line matches in one pass (`None` when
    /// the set would exceed the regex size limit; the formats are then tried one by one).
    pub text_format_set: Option<regex::RegexSet>,
    pub timestamps: TimestampParser,
    pub extractors: Vec<Extractor>,
    pub markers: Vec<Marker>,
    /// Bitmask of lifecycle marker rules.
    pub lifecycle_mask: crate::model::MarkerMask,
    /// The parser's `[[pipeline]]` steps (run first; their statistics are not reported per step).
    pub format_pipeline: Option<crate::pipeline::Pipeline>,
    pub pipeline: crate::pipeline::Pipeline,
    pub icons: Vec<crate::icons::CustomIcon>,
    /// `[[span]]` rules, their marker bits resolved against `markers` (see [`CompiledProfile::resolve_spans`]).
    pub spans: Vec<crate::spans::SpanRule>,
    /// `[[route]]` rules, in profile order.
    pub routes: Vec<crate::route::Route>,
}

impl CompiledProfile {
    /// Compiles a profile with the generic parser, without plugin transforms.
    pub fn compile(profile: Profile) -> Result<CompiledProfile> {
        Self::compile_with(profile, &std::collections::HashMap::new())
    }

    /// Compiles a profile with the generic parser; `{ step = "plugin" }` entries resolve against `transforms`.
    pub fn compile_with(profile: Profile, transforms: &Transforms) -> Result<CompiledProfile> {
        Self::compile_for(profile, &crate::parser::builtin::generic(), transforms)
    }

    /// Compiles a profile with `parser`; `{ step = "plugin" }` entries resolve against `transforms`.
    pub fn compile_for(profile: Profile, parser: &Parser, transforms: &Transforms) -> Result<CompiledProfile> {
        profile.live.compile()?;
        crate::ingest::EncodingChoice::parse(&profile.files.encoding).map_err(|e| anyhow!("files.encoding: {e}"))?;
        let include = globs(&profile.files.include).context("files.include")?;
        let exclude = globs(&profile.files.exclude).context("files.exclude")?;
        let format = parser.format.clone();

        let mut text_formats = Vec::new();
        let mut patterns = Vec::new();
        for t in &format.output_templates {
            let pattern = output_template_to_regex(t);
            let regex = crate::user_regex::compile(&pattern, false).with_context(|| format!("format.output_templates: {t}"))?;
            patterns.push(pattern);
            let time_only = !t.contains("{Timestamp:yyyy") && !t.contains("{Timestamp}") && t.contains("{Timestamp:");
            text_formats.push(TextFormat {
                regex,
                time_only: time_only && !has_date_specifier(t),
            });
        }
        for r in &format.regexes {
            let regex = crate::user_regex::compile(r, false).with_context(|| format!("format.regexes: {r}"))?;
            text_formats.push(TextFormat { regex, time_only: false });
            patterns.push(r.clone());
        }
        let text_format_set = crate::user_regex::compile_set(&patterns).ok();

        let mut timestamps = TimestampParser::new(&format.timestamp_formats, &format.assume_timezone, format.day_first.resolve()?)?;
        if !format.boot_time.trim().is_empty() {
            let boot = chrono::DateTime::parse_from_rfc3339(format.boot_time.trim())
                .map_err(|e| anyhow!("format.boot_time must be an RFC 3339 time like \"2026-10-14T08:00:00Z\": {e}"))?;
            timestamps = timestamps.with_boot(boot.timestamp_millis());
        }

        let mut extractors = Vec::new();
        for e in parser.extract.iter().chain(&profile.extract) {
            let regex = crate::user_regex::compile(&e.regex, false).with_context(|| RuleContext::new("extract", &e.name, Some("regex")))?;
            let when = compile_conditions(&e.when).with_context(|| RuleContext::new("extract", &e.name, Some("when")))?;
            let mut fields: Vec<(String, FieldSpec)> = e.fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            for (name, _) in &fields {
                if regex.capture_names().flatten().all(|g| g != name) {
                    return Err(anyhow!("field \"{name}\" has no capture group (?P<{name}>...) in the regex")
                        .context(RuleContext::new("extract", &e.name, None)));
                }
            }
            if fields.is_empty() {
                fields = regex
                    .capture_names()
                    .flatten()
                    .map(|g| {
                        (
                            g.to_string(),
                            FieldSpec {
                                kind: FieldType::Auto,
                                unit: None,
                                unit_from: None,
                                to: None,
                            },
                        )
                    })
                    .collect();
            }
            extractors.push(Extractor {
                name: e.name.clone(),
                from: e.from.clone(),
                regex,
                when,
                fields,
            });
        }

        let mut icons: Vec<crate::icons::CustomIcon> = Vec::new();
        for cfg in &profile.icon {
            let icon = crate::icons::compile(cfg)?;
            if icons.iter().any(|c| c.name == icon.name) {
                return Err(anyhow!("icon \"{}\" is defined twice", icon.name));
            }
            icons.push(icon);
        }

        if profile.marker.len() > 32 {
            return Err(anyhow!("at most 32 [[marker]] rules are supported"));
        }
        let mut markers = Vec::new();
        let mut lifecycle_mask: crate::model::MarkerMask = 0;
        for (i, m) in profile.marker.iter().enumerate() {
            if m.lifecycle {
                lifecycle_mask |= 1 << i;
            }
            markers.push(compile_marker(m, &icons, "profile")?);
        }

        for c in &profile.chart {
            crate::chart::parse_y(&c.y).with_context(|| RuleContext::new("chart", &c.title, Some("y")))?;
            crate::chart::parse_bucket(&c.bucket).with_context(|| RuleContext::new("chart", &c.title, Some("bucket")))?;
            compile_conditions(&c.when).with_context(|| RuleContext::new("chart", &c.title, Some("when")))?;
        }

        let format_pipeline = if parser.pipeline.is_empty() {
            None
        } else {
            Some(crate::pipeline::Pipeline::compile(&parser.pipeline, transforms).context("the parser's [[pipeline]]")?)
        };
        let pipeline = crate::pipeline::Pipeline::compile(&profile.pipeline, transforms)?;
        crate::redact::Redactor::from_config(&profile.redact, crate::redact::Salt::fixed([0; 16])).context("[redact]")?;
        crate::dashboard::validate_all(&profile.dashboard).context("[[dashboard]]")?;
        let chart_ids: Vec<String> = profile
            .chart
            .iter()
            .enumerate()
            .map(|(i, c)| c.id.clone().unwrap_or_else(|| format!("chart-{}", i + 1)))
            .collect();
        crate::lens::validate_all(&profile.lens, &chart_ids).context("[[lens]]")?;
        let mut spans = crate::spans::config::compile(&profile.span)?;
        crate::spans::config::resolve(&mut spans, &markers);
        let routes = crate::route::compile_all(&profile.route)?;

        Ok(CompiledProfile {
            routes,
            spans,
            icons,
            format_pipeline,
            pipeline,
            format,
            parser: parser.reference(),
            profile,
            include,
            exclude,
            text_formats,
            text_format_set,
            timestamps,
            extractors,
            markers,
            lifecycle_mask,
        })
    }

    /// Re-resolves the `[[span]]` marker kinds after the marker set changed (plugin kinds merged in).
    pub fn resolve_spans(&mut self) {
        crate::spans::config::resolve(&mut self.spans, &self.markers);
    }

    /// The ingest pipeline for one file: extractors, the `[[pipeline]]`
    /// steps, then marker rules. Returns the surviving records and the events
    /// in/out of every pipeline step.
    pub fn ingest(&self, mut records: Vec<Record>, source_name: &str) -> (Vec<Record>, Vec<crate::pipeline::StepStats>) {
        for r in &mut records {
            self.apply_extractors(r, source_name);
        }
        let marks = crate::plugin::markers::MarkIndex::of(&self.markers);
        if let Some(format) = &self.format_pipeline {
            records = format.run_marking(records, source_name, &marks).0;
        }
        let (mut records, stats) = self.pipeline.run_marking(records, source_name, &marks);
        for r in &mut records {
            // Plugin transforms may already have marked the record (`TransformCtx::mark`).
            r.markers |= self.compute_markers(r, source_name);
        }
        (records, stats)
    }

    /// Runs the extractors on a record, adding the produced fields.
    pub fn apply_extractors(&self, rec: &mut Record, source_name: &str) {
        for ex in &self.extractors {
            if let Some(values) = ex.extract(rec, source_name) {
                for (k, v) in values {
                    rec.fields.insert(k, v);
                }
            }
        }
    }

    pub fn compute_markers(&self, rec: &Record, source_name: &str) -> crate::model::MarkerMask {
        let mut mask = 0;
        for (i, m) in self.markers.iter().enumerate() {
            if !m.when.is_empty() && m.when.iter().all(|c| c.eval(rec, source_name)) {
                mask |= 1 << i;
            }
        }
        mask
    }
}

impl Extractor {
    pub fn applies(&self, rec: &Record, source_name: &str) -> bool {
        self.when.iter().all(|c| c.eval(rec, source_name))
    }

    pub fn extract(&self, rec: &Record, source_name: &str) -> Option<Vec<(String, Value)>> {
        if !self.applies(rec, source_name) {
            return None;
        }
        let text = rec.field_text(&self.from, source_name)?;
        let caps = self.regex.captures(&text)?;
        Some(self.convert(&caps))
    }

    pub fn convert(&self, caps: &regex::Captures) -> Vec<(String, Value)> {
        let mut out = Vec::new();
        for (name, spec) in &self.fields {
            let Some(m) = caps.name(name) else { continue };
            let unit = spec
                .unit_from
                .as_deref()
                .and_then(|g| caps.name(g))
                .map(|m| m.as_str())
                .or(spec.unit.as_deref());
            if let Some(v) = convert_value(m.as_str(), spec.kind, unit, spec.to.as_deref()) {
                out.push((name.clone(), v));
            }
        }
        out
    }
}

fn convert_value(raw: &str, kind: FieldType, unit: Option<&str>, to: Option<&str>) -> Option<Value> {
    let raw = raw.trim();
    let num = |s: &str| -> Option<f64> {
        let cleaned: String = s.chars().filter(|c| *c != '_' && *c != ',' && *c != ' ').collect();
        cleaned.parse::<f64>().ok().filter(|f| f.is_finite())
    };
    match kind {
        FieldType::String => Some(Value::String(raw.to_string())),
        FieldType::Auto => Some(auto_value(raw)),
        FieldType::Number => num(raw).map(Value::from),
        FieldType::Integer => num(raw).map(|f| Value::from(f.round() as i64)),
        FieldType::Bool => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "on" => Some(Value::Bool(true)),
            "false" | "no" | "0" | "off" => Some(Value::Bool(false)),
            _ => None,
        },
        FieldType::Bytes => {
            let (n, suffix) = split_number_suffix(raw);
            let n = num(n)?;
            let from = bytes_factor(unit.or(Some(suffix).filter(|s| !s.is_empty())).unwrap_or("B"))?;
            let to = bytes_factor(to.unwrap_or("MB"))?;
            Some(Value::from(round3(n * from / to)))
        }
        FieldType::Duration => {
            let ms = if raw.contains(':') {
                timespan_ms(raw)?
            } else {
                let (n, suffix) = split_number_suffix(raw);
                num(n)? * duration_factor(unit.or(Some(suffix).filter(|s| !s.is_empty())).unwrap_or("ms"))?
            };
            let to = duration_factor(to.unwrap_or("ms"))?;
            Some(Value::from(round3(ms / to)))
        }
    }
}

fn round3(f: f64) -> f64 {
    (f * 1000.0).round() / 1000.0
}

fn split_number_suffix(s: &str) -> (&str, &str) {
    let idx = s
        .find(|c: char| !(c.is_ascii_digit() || c == '.' || c == ',' || c == '-' || c == '_' || c == ' '))
        .unwrap_or(s.len());
    (&s[..idx], s[idx..].trim())
}

fn bytes_factor(unit: &str) -> Option<f64> {
    let u = unit.trim().to_ascii_lowercase();
    let u = u.trim_end_matches("ytes").trim_end_matches("yte");
    Some(match u {
        "b" | "" => 1.0,
        "k" | "kb" | "kib" => 1024.0,
        "m" | "mb" | "mib" => 1024.0 * 1024.0,
        "g" | "gb" | "gib" => 1024.0 * 1024.0 * 1024.0,
        "t" | "tb" | "tib" => 1024f64.powi(4),
        _ => return None,
    })
}

fn duration_factor(unit: &str) -> Option<f64> {
    Some(match unit.trim().to_ascii_lowercase().as_str() {
        "ns" => 1e-6,
        "us" | "µs" => 1e-3,
        "ms" | "" => 1.0,
        "s" | "sec" | "secs" | "seconds" => 1000.0,
        "m" | "min" | "mins" | "minutes" => 60_000.0,
        "h" | "hr" | "hours" => 3_600_000.0,
        _ => return None,
    })
}

/// .NET TimeSpan text: [d.]hh:mm:ss[.fffffff]
fn timespan_ms(s: &str) -> Option<f64> {
    let (days, rest) = match s.split_once('.') {
        Some((d, r)) if r.contains(':') => (d.parse::<f64>().ok()?, r),
        _ => (0.0, s),
    };
    let parts: Vec<&str> = rest.split(':').collect();
    if parts.len() != 3 {
        return None;
    }
    let h: f64 = parts[0].parse().ok()?;
    let m: f64 = parts[1].parse().ok()?;
    let sec: f64 = parts[2].parse().ok()?;
    Some(((days * 24.0 + h) * 60.0 + m) * 60_000.0 + sec * 1000.0)
}

/// Outlines a generated badge can use.
pub const BADGE_SHAPES: &[&str] = &["circle", "square", "diamond", "hexagon"];

fn is_badge(b: &str) -> bool {
    let b = b.trim();
    let n = b.chars().count();
    (1..=3).contains(&n) && b.chars().all(|c| !c.is_control() && !c.is_whitespace())
}

fn is_hex_color(c: &str) -> bool {
    c.strip_prefix('#')
        .is_some_and(|h| matches!(h.len(), 3 | 6) && h.chars().all(|ch| ch.is_ascii_hexdigit()))
}

fn has_date_specifier(t: &str) -> bool {
    t.split("{Timestamp").skip(1).any(|rest| {
        let spec = rest.split('}').next().unwrap_or("");
        spec.contains('y') || spec.contains('d') || spec.contains("MM-") || spec.contains("/MM")
    })
}

fn globs(patterns: &[String]) -> Result<GlobSet> {
    let mut b = GlobSetBuilder::new();
    for p in patterns {
        b.add(Glob::new(p).with_context(|| format!("invalid glob \"{p}\""))?);
    }
    Ok(b.build()?)
}

/// Which rule of a profile a compile error is about (`extract "stats": regex`): attached as the error's
/// context, so callers find the rule (and its line, [`RuleContext::line_in`]) without reading the message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleContext {
    /// The rule's table: `extract`, `chart` or `marker`.
    pub section: &'static str,
    /// Its name (`name`, `title` or `kind`).
    pub name: String,
    /// The part of the rule that failed (`regex`, `when`, ...), if one.
    pub part: Option<&'static str>,
}

impl RuleContext {
    pub fn new(section: &'static str, name: &str, part: Option<&'static str>) -> RuleContext {
        RuleContext {
            section,
            name: name.to_string(),
            part,
        }
    }

    /// The key that names a rule of this section.
    fn key(&self) -> &'static str {
        match self.section {
            "chart" => "title",
            "marker" => "kind",
            _ => "name",
        }
    }

    /// The 1-based line in `toml` that names this rule (`name = "stats"` under `[[extract]]`), if any.
    pub fn line_in(&self, toml: &str) -> Option<usize> {
        let header = format!("[[{}]]", self.section);
        let mut inside = false;
        for (i, line) in toml.lines().enumerate() {
            let t = line.trim();
            if t.starts_with('[') {
                inside = t == header;
                continue;
            }
            let Some((key, value)) = t.split_once('=') else { continue };
            let value = value.trim().trim_matches(|c| c == '"' || c == '\x27');
            if inside && key.trim() == self.key() && value == self.name {
                return Some(i + 1);
            }
        }
        None
    }

    /// The rule an error is about, when it carries one.
    pub fn of(e: &anyhow::Error) -> Option<&RuleContext> {
        e.downcast_ref::<RuleContext>()
    }
}

impl std::fmt::Display for RuleContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} \"{}\"", self.section, self.name)?;
        match self.part {
            Some(p) => write!(f, ": {p}"),
            None => Ok(()),
        }
    }
}

/// Validates and compiles one marker rule; `icons` are the custom icons it may use.
pub(crate) fn compile_marker(m: &MarkerCfg, icons: &[crate::icons::CustomIcon], owner: &str) -> Result<Marker> {
    compile_marker_rule(m, icons, owner).with_context(|| RuleContext::new("marker", &m.kind, None))
}

fn compile_marker_rule(m: &MarkerCfg, icons: &[crate::icons::CustomIcon], owner: &str) -> Result<Marker> {
    let when = compile_conditions(&m.when).context("when")?;
    if let Some(icon) = m
        .icon
        .as_deref()
        .filter(|i| !MARKER_ICONS.contains(i) && !icons.iter().any(|c| c.name == *i))
    {
        return Err(anyhow!(
            "unknown icon \"{icon}\" (use one of: {}, or define it with [[icon]])",
            MARKER_ICONS.join(", ")
        ));
    }
    if let Some(badge) = m.badge.as_deref().filter(|b| !is_badge(b)) {
        return Err(anyhow!(
            "badge \"{badge}\" must be 1–3 visible characters (e.g. \"1\", \"A\", \"DB\")"
        ));
    }
    if let Some(shape) = m.shape.as_deref().filter(|s| !BADGE_SHAPES.contains(s)) {
        return Err(anyhow!("shape \"{shape}\" must be one of {}", BADGE_SHAPES.join(", ")));
    }
    if let Some(color) = m.color.as_deref().filter(|c| !is_hex_color(c)) {
        return Err(anyhow!("color \"{color}\" must be #RGB or #RRGGBB"));
    }
    Ok(Marker {
        kind: m.kind.clone(),
        owner: owner.to_string(),
        role: None,
        label: m.label.clone().unwrap_or_else(|| m.kind.clone()),
        lifecycle: m.lifecycle,
        color: m.color.clone(),
        icon: m.icon.clone(),
        badge: m.badge.as_ref().map(|b| b.trim().to_string()),
        shape: m.shape.clone(),
        description: m.description.clone(),
        when,
    })
}

pub fn compile_conditions(cfgs: &[ConditionCfg]) -> Result<Vec<Condition>> {
    cfgs.iter().map(Condition::compile).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_files_encoding_defaults_to_auto_and_unknown_labels_fall_back() {
        use crate::ingest::EncodingChoice;
        let cfg: FilesCfg = toml::from_str("include = ['*.log']\nexclude = []").unwrap();
        assert_eq!(cfg.encoding, "auto");
        assert_eq!(cfg.encoding_choice(), EncodingChoice::Auto);
        let sjis = FilesCfg {
            encoding: "Shift_JIS".into(),
            ..cfg.clone()
        };
        assert!(matches!(sjis.encoding_choice(), EncodingChoice::Fixed(_)));
        // A label the profile compile would have refused reads as automatic detection, never a failure.
        let odd = FilesCfg {
            encoding: "no-such-encoding".into(),
            ..cfg
        };
        assert_eq!(odd.encoding_choice(), EncodingChoice::Auto);
    }

    #[test]
    fn default_profile_compiles() {
        let c = CompiledProfile::compile(Profile::default_profile()).unwrap();
        assert_eq!(c.markers.len(), 6);
        assert_eq!(c.lifecycle_mask, 0b110);
        assert!(!c.text_formats[0].time_only);
        assert!(c.text_formats[1].time_only);
        assert_eq!((c.parser.id.as_str(), c.format.kind.as_str()), ("generic", "auto"));
        assert!(c.format_pipeline.is_none());
    }

    #[test]
    fn partial_profile_merges_defaults() {
        let p = Profile::from_toml("name = \"Mine\"\n[instance]\nkey = ['Host']\n").unwrap();
        assert_eq!(p.name, "Mine");
        assert_eq!(p.instance.key, ["Host"]);
        assert!(!p.instance.candidates.is_empty());
        assert_eq!(p.marker.len(), 6);
    }

    #[test]
    fn reports_error_line() {
        let e = Profile::from_toml("name = \"x\"\n[instance\n").unwrap_err();
        assert_eq!(e.line, Some(2));
    }

    #[test]
    fn formats_are_refused_with_a_pointer_to_parsers() {
        let e = Profile::from_toml("name = \"x\"\n\n[parse]\nregexes = []\n").unwrap_err();
        assert_eq!(e.line, Some(3));
        assert!(e.message.contains("Settings ▸ Parsers"), "{e}");
        let dotted = Profile::from_toml("parse.regexes = []\n").unwrap_err();
        assert_eq!(dotted.line, Some(1));
        // A parser's sections pasted into a profile are refused too.
        let format = Profile::from_toml("name = \"x\"\n[format]\nkind = \"text\"\n").unwrap_err();
        assert!(format.message.starts_with("[format]") && format.line == Some(2), "{format}");
        assert!(!DEFAULT_PROFILE_TOML.lines().any(|l| l.starts_with("[parse")));
    }

    #[test]
    fn a_parser_adds_its_format_extractors_and_steps() {
        let parser = Parser::from_toml(
            "schema = 1\nid = \"web\"\nname = \"Web\"\nversion = \"1.0.0\"\n[format]\nkind = \"text\"\nregexes = ['^(?P<timestamp>\\S+ \\S+) (?P<Status>\\d+) (?P<message>.*)$']\ntimestamp_formats = ['%Y-%m-%d %H:%M:%S']\n[[extract]]\nname = \"ms\"\nregex = 'took (?P<ms>\\d+)'\n[[pipeline]]\nstep = \"set_level\"\nlevel = \"Error\"\nwhen = [{ field = \"Status\", op = \"gt\", value = 499 }]\n",
        )
        .unwrap();
        let profile = Profile::from_toml("[[extract]]\nname = \"order\"\nregex = 'order (?P<order>\\d+)'\n").unwrap();
        let c = CompiledProfile::compile_for(profile, &parser, &Default::default()).unwrap();
        assert_eq!(c.parser, parser.reference());
        assert_eq!(c.text_formats.len(), 1);
        let names: Vec<&str> = c.extractors.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["ms", "order"]);
        let mut r = Record::new(1);
        r.message = "order 7 took 12".into();
        r.fields.insert("Status".into(), 503.into());
        let (out, stats) = c.ingest(vec![r], "s");
        assert_eq!(out[0].level, crate::model::Level::Error);
        assert_eq!((out[0].fields["ms"].clone(), out[0].fields["order"].clone()), (12.into(), 7.into()));
        // Per-step statistics are the profile's steps only.
        assert!(stats.is_empty());
    }

    #[test]
    fn extractor_units() {
        let p = Profile::from_toml(
            r#"
[[extract]]
name = "stats"
regex = 'CPU (?P<cpu>[\d.]+)% .* WorkingSet (?P<ws>[\d.]+) ?(?P<ws_unit>KB|MB|GB) took (?P<t>\S+)'
[extract.fields]
cpu = { type = "number" }
ws = { type = "bytes", unit_from = "ws_unit", to = "MB" }
t = { type = "duration" }
"#,
        )
        .unwrap();
        let c = CompiledProfile::compile(p).unwrap();
        let mut r = Record::new(1);
        r.message = "CPU 96.8% · WorkingSet 2 GB took 00:00:01.5".into();
        c.apply_extractors(&mut r, "x");
        assert_eq!(r.fields["cpu"], 96.8);
        assert_eq!(r.fields["ws"], 2048.0);
        assert_eq!(r.fields["t"], 1500.0);
    }

    fn compile(toml: &str) -> anyhow::Result<CompiledProfile> {
        CompiledProfile::compile(Profile::from_toml(toml).map_err(|e| anyhow!(e.to_string()))?)
    }

    #[test]
    fn disabled_markers_are_kept_but_inactive() {
        let text = r#"
[[marker]]
kind = "boom"
when = [{ field = "@message", op = "contains", value = "boom" }]

[[imt_disabled_markers]]
kind = "quiet"
label = "Quiet"
imt_index = 0
when = [{ field = "@message", op = "contains", value = "quiet" }]

[[imt_disabled_markers]]
kind = "broken"
when = [{ field = "@message", op = "matches", value = "(" }]
"#;
        let p = Profile::from_toml(text).unwrap();
        assert_eq!(p.imt_disabled_markers.len(), 2);
        assert_eq!(p.imt_disabled_markers[0].rule.kind, "quiet");
        assert_eq!(p.imt_disabled_markers[0].imt_index, Some(0));
        assert_eq!(p.imt_disabled_markers[1].imt_index, None);
        // Round-trips through serialisation (profile history, saving).
        let again: Profile = toml::from_str(&toml::to_string(&p).unwrap()).unwrap();
        assert_eq!(again.imt_disabled_markers.len(), 2);
        // Disabled rules are never compiled: an invalid one does not break the profile.
        let c = CompiledProfile::compile(p).unwrap();
        let kinds: Vec<&str> = c.markers.iter().map(|m| m.kind.as_str()).collect();
        assert_eq!(kinds, vec!["boom"]);
        // Absent key: nothing disabled, nothing serialised.
        let d = Profile::default_profile();
        assert!(d.imt_disabled_markers.is_empty());
        assert!(!toml::to_string(&d).unwrap().contains("imt_disabled_markers"));
    }

    #[test]
    fn day_first_values() {
        for (v, want) in [
            ("true", Some(true)),
            ("false", Some(false)),
            ("'yes'", Some(true)),
            ("'no'", Some(false)),
            ("'auto'", None),
        ] {
            let c = crate::parser::testing::compiled(&format!("[format]\nday_first = {v}"));
            assert_eq!(c.timestamps.day_first(), want, "{v}");
        }
        assert!(crate::parser::builtin::generic()
            .with_override("[format]\nday_first = 'sometimes'")
            .is_err());
    }

    #[test]
    fn compile_errors() {
        assert!(compile("[files]\ninclude = ['[']").is_err());
        let generic = crate::parser::builtin::generic();
        assert!(generic.with_override("[format]\nregexes = ['(']").is_err());
        assert!(generic.with_override("[format]\nassume_timezone = 'Mars'").is_err());
        assert!(compile("[[extract]]\nname='x'\nregex='('").is_err());
        assert!(compile("[[extract]]\nname='x'\nregex='(?P<a>.)'\n[extract.fields]\nb = { type = 'number' }").is_err());
        assert!(compile("[[extract]]\nname='x'\nregex='a'\nwhen=[{field='f',op='nope'}]").is_err());
        assert!(compile("[[marker]]\nkind='x'\nwhen=[{field='f',op='nope'}]").is_err());
        assert!(compile("[[chart]]\ntitle='t'\ny='nope'").is_err());
        assert!(compile("[[chart]]\ntitle='t'\ny='count()'\nbucket='5x'").is_err());
        assert!(compile("[[chart]]\ntitle='t'\ny='count()'\nwhen=[{field='f',op='nope'}]").is_err());
        let many: String = (0..33).map(|i| format!("[[marker]]\nkind='k{i}'\n")).collect();
        assert!(compile(&many).err().unwrap().to_string().contains("32"));
        // Type errors after merging have no line.
        let e = Profile::from_toml("name = 5").unwrap_err();
        assert_eq!(e.line, None);
        assert!(e.to_string().contains("string"));
        let e = Profile::from_toml("x = \n").unwrap_err();
        assert!(e.to_string().starts_with("line 1"));
    }

    #[test]
    fn chart_and_field_defaults() {
        let p =
            Profile::from_toml("[[chart]]\ntitle='t'\ny='count()'\n[[extract]]\nname='e'\nregex='(?P<v>\\d+)'\n[extract.fields]\nv = {}")
                .unwrap();
        assert_eq!(p.chart[0].bucket, "auto");
        assert_eq!(p.chart[0].kind, "line");
        assert_eq!(p.extract[0].fields["v"].kind, FieldType::Auto);
        assert_eq!(p.extract[0].from, "@message");
    }

    #[test]
    fn extractor_without_fields_takes_all_groups() {
        let c = compile(
            "[[extract]]\nname='e'\nregex='id=(?P<id>\\d+) user=(?P<user>\\w+)'\nwhen=[{field='@level',op='eq',value='Information'}]",
        )
        .unwrap();
        let mut r = Record::new(1);
        r.message = "id=42 user=ann".into();
        c.apply_extractors(&mut r, "s");
        assert_eq!(r.fields["id"], 42);
        assert_eq!(r.fields["user"], "ann");
        // `when` not satisfied / source field missing / no match.
        let mut w = Record::new(1);
        w.level = crate::model::Level::Error;
        w.message = "id=1 user=x".into();
        c.apply_extractors(&mut w, "s");
        assert!(w.fields.is_empty());
        let mut n = Record::new(1);
        n.message = "nothing".into();
        c.apply_extractors(&mut n, "s");
        assert!(n.fields.is_empty());
        let e = &c.extractors[0];
        let mut m = Record::new(1);
        m.message = "x".into();
        assert!(Extractor {
            from: "Missing".into(),
            ..clone_extractor(e)
        }
        .extract(&m, "s")
        .is_none());
    }

    fn clone_extractor(e: &Extractor) -> Extractor {
        Extractor {
            name: e.name.clone(),
            from: e.from.clone(),
            regex: e.regex.clone(),
            when: e.when.clone(),
            fields: e.fields.clone(),
        }
    }

    #[test]
    fn conversions() {
        let cv = |raw: &str, kind: &str, unit: Option<&str>, to: Option<&str>| {
            let kind: FieldType = serde_json::from_value(serde_json::Value::String(kind.into())).unwrap();
            convert_value(raw, kind, unit, to)
        };
        assert_eq!(cv(" x ", "string", None, None), Some(Value::from("x")));
        assert_eq!(cv("1,204", "integer", None, None), Some(Value::from(1204)));
        assert_eq!(cv("abc", "number", None, None), None);
        for t in ["true", "yes", "1", "on"] {
            assert_eq!(cv(t, "bool", None, None), Some(Value::Bool(true)));
        }
        for f in ["false", "no", "0", "off"] {
            assert_eq!(cv(f, "bool", None, None), Some(Value::Bool(false)));
        }
        assert_eq!(cv("maybe", "bool", None, None), None);
        assert_eq!(cv("512KB", "bytes", None, Some("MB")), Some(Value::from(0.5)));
        assert_eq!(cv("1", "bytes", Some("TB"), Some("GB")), Some(Value::from(1024.0)));
        assert_eq!(cv("1048576", "bytes", None, None), Some(Value::from(1.0)));
        assert_eq!(cv("1", "bytes", Some("parsecs"), None), None);
        assert_eq!(cv("1.5s", "duration", None, None), Some(Value::from(1500.0)));
        assert_eq!(cv("2", "duration", Some("min"), Some("s")), Some(Value::from(120.0)));
        assert_eq!(cv("1", "duration", Some("h"), Some("m")), Some(Value::from(60.0)));
        assert_eq!(cv("1500", "duration", None, Some("s")), Some(Value::from(1.5)));
        assert_eq!(cv("1.02:00:00", "duration", None, Some("h")), Some(Value::from(26.0)));
        assert_eq!(cv("1:2", "duration", None, None), None);
        assert_eq!(cv("5", "duration", Some("fortnights"), None), None);
        assert_eq!(cv("5", "duration", None, Some("ns")), Some(Value::from(5_000_000.0)));
        assert_eq!(cv("5", "duration", Some("us"), None), Some(Value::from(0.005)));
        assert_eq!(cv("7", "auto", None, None), Some(Value::from(7)));
    }

    #[test]
    fn unknown_field_types_are_refused_with_their_line() {
        // Regression (engine review finding 20): an unknown type fell back to `auto` silently.
        let text = "name = \"x\"\n[[extract]]\nname = \"ms\"\nregex = '(?P<ms>[0-9]+)'\n[extract.fields.ms]\ntype = \"millis\"\n";
        let e = Profile::from_toml(text).unwrap_err();
        assert_eq!(e.line, Some(6), "{e}");
        assert!(
            e.message.contains("unknown type \"millis\"") && e.message.contains("duration"),
            "{e}"
        );
        let ok = Profile::from_toml(&text.replace("millis", "duration")).unwrap();
        assert_eq!(ok.extract[0].fields["ms"].kind, FieldType::Duration);
    }

    #[test]
    fn overrides_merge_over_the_profile() {
        let base = Profile::from_toml("name = \"base\"\n[instance]\nkey = [\"Host\"]").unwrap();
        let merged = base.with_override("[instance]\ncandidates = [\"App\"]").unwrap();
        assert_eq!(merged.name, "base");
        assert_eq!(
            (merged.instance.key.clone(), merged.instance.candidates.clone()),
            (vec!["Host".to_string()], vec!["App".to_string()])
        );
        let e = base.with_override("[instance\n").unwrap_err();
        assert_eq!(e.line, Some(1));
    }

    #[test]
    fn marker_icons_and_colors_are_validated() {
        let ok =
            compile("[[marker]]\nkind='pay'\nicon='credit-card'\ncolor='#E9AE3F'\nwhen=[{field='@message',op='contains',value='payment'}]")
                .unwrap();
        let m = ok.markers.iter().find(|m| m.kind == "pay").unwrap();
        assert_eq!((m.icon.as_deref(), m.color.as_deref()), (Some("credit-card"), Some("#E9AE3F")));
        let err = compile("[[marker]]\nkind='x'\nicon='unicorn'").err().unwrap();
        let e = format!("{err:#}");
        assert!(e.starts_with("marker \"x\": unknown icon") && e.contains("credit-card"), "{e}");
        assert_eq!(RuleContext::of(&err), Some(&RuleContext::new("marker", "x", None)));
        assert!(compile("[[marker]]\nkind='x'\ncolor='red'").is_err());
        assert!(compile("[[marker]]\nkind='x'\ncolor='#abc'").is_ok());
        assert!(MARKER_ICONS.len() >= 30);
    }

    #[test]
    fn custom_icons_are_compiled_and_referenced() {
        let c =
            compile("[[icon]]\nname='acme'\nsvg='<svg viewBox=\"0 0 1 1\"><script>x</script></svg>'\n[[marker]]\nkind='k'\nicon='acme'")
                .unwrap();
        assert_eq!(c.icons[0].svg.as_deref(), Some("<svg viewBox=\"0 0 1 1\"></svg>"));
        assert_eq!(c.markers.iter().find(|m| m.kind == "k").unwrap().icon.as_deref(), Some("acme"));
        assert!(compile("[[icon]]\nname='a'\nsvg='<svg/>'\n[[icon]]\nname='a'\nsvg='<svg/>'").is_err());
        assert!(compile("[[icon]]\nname='a'\nsvg='<p/>'").is_err());
    }

    #[test]
    fn badges_and_shapes() {
        let c = compile("[[marker]]\nkind='gw'\nbadge=' 3 '\nshape='hexagon'\ndescription='Payment gateway timeout'\ncolor='#3987e5'\nwhen=[{field='@message',op='matches',value='(?i)gateway.*timeout'}]").unwrap();
        let m = c.markers.iter().find(|m| m.kind == "gw").unwrap();
        assert_eq!(
            (m.badge.as_deref(), m.shape.as_deref(), m.description.as_deref()),
            (Some("3"), Some("hexagon"), Some("Payment gateway timeout"))
        );
        for bad in ["badge=''", "badge='LONG'", "badge='a b'", "shape='star'"] {
            assert!(compile(&format!("[[marker]]\nkind='x'\n{bad}")).is_err(), "{bad}");
        }
        assert!(compile("[[marker]]\nkind='x'\nbadge='DB'").is_ok());
        assert!(compile("[[marker]]\nkind='x'\nbadge='!'").is_ok());
    }
}

#[cfg(test)]
mod rule_context_tests {
    use super::*;

    #[test]
    fn compile_errors_name_their_rule_and_its_line() {
        let toml = "name = 'p'\n[[extract]]\nname = \"stats\"\nregex = '('\n[[chart]]\ntitle = 'Load'\ny = 'nonsense('\n";
        let profile = Profile::from_toml(toml).unwrap();
        let err = CompiledProfile::compile(profile).err().expect("the regex does not compile");
        let rule = RuleContext::of(&err).expect("a typed rule");
        assert_eq!((rule.section, rule.name.as_str(), rule.part), ("extract", "stats", Some("regex")));
        assert_eq!(rule.line_in(toml), Some(3));
        assert!(format!("{err:#}").starts_with("extract \"stats\": regex"));
        assert_eq!(RuleContext::new("chart", "Load", Some("y")).line_in(toml), Some(6));
        assert_eq!(RuleContext::new("marker", "Load", None).line_in(toml), None, "another section");
        assert_eq!(RuleContext::new("marker", "m", None).to_string(), "marker \"m\"");
        assert!(RuleContext::of(&anyhow!("plain")).is_none());
    }
}
