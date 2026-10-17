//! Parsing profiles: the user-editable pipeline (files -> parse -> instance ->
//! extract -> markers -> charts). See `default_profile.toml` for the schema.

use crate::condition::{Condition, ConditionCfg};
use crate::model::{auto_value, Record};
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
    pub parse: ParseCfg,
    pub instance: InstanceCfg,
    #[serde(default)]
    pub extract: Vec<ExtractCfg>,
    #[serde(default)]
    pub marker: Vec<MarkerCfg>,
    #[serde(default)]
    pub chart: Vec<ChartCfg>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FilesCfg {
    pub include: Vec<String>,
    pub exclude: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ParseCfg {
    pub format: String,
    pub output_templates: Vec<String>,
    pub regexes: Vec<String>,
    pub timestamp_formats: Vec<String>,
    pub assume_timezone: String,
    pub day_first: DayFirst,
    pub json: JsonCfg,
    pub csv: CsvCfg,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DayFirst {
    Bool(bool),
    Text(String),
}

impl DayFirst {
    fn resolve(&self) -> Result<Option<bool>> {
        match self {
            DayFirst::Bool(b) => Ok(Some(*b)),
            DayFirst::Text(t) => match t.to_ascii_lowercase().as_str() {
                "auto" | "" => Ok(None),
                "true" | "yes" => Ok(Some(true)),
                "false" | "no" => Ok(Some(false)),
                _ => Err(anyhow!("parse.day_first must be \"auto\", true or false")),
            },
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JsonCfg {
    pub timestamp: Vec<String>,
    pub level: Vec<String>,
    pub message: Vec<String>,
    pub template: Vec<String>,
    pub exception: Vec<String>,
    pub properties: Vec<String>,
    pub default_level: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CsvCfg {
    pub delimiter: String,
    pub has_header: bool,
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstanceCfg {
    pub key: Vec<String>,
    pub candidates: Vec<String>,
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

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldSpec {
    #[serde(rename = "type", default = "default_type")]
    pub kind: String,
    #[serde(default)]
    pub unit: Option<String>,
    #[serde(default)]
    pub unit_from: Option<String>,
    #[serde(default)]
    pub to: Option<String>,
}

fn default_type() -> String {
    "auto".into()
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
    #[serde(default)]
    pub when: Vec<ConditionCfg>,
}

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
}

fn merge(base: &mut toml::Table, over: toml::Table) {
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
    pub label: String,
    pub lifecycle: bool,
    pub color: Option<String>,
    pub when: Vec<Condition>,
}

/// A profile with every regex, glob and condition compiled.
pub struct CompiledProfile {
    pub profile: Profile,
    pub include: GlobSet,
    pub exclude: GlobSet,
    pub text_formats: Vec<TextFormat>,
    pub timestamps: TimestampParser,
    pub extractors: Vec<Extractor>,
    pub markers: Vec<Marker>,
    /// Bitmask of lifecycle marker rules.
    pub lifecycle_mask: u32,
}

impl CompiledProfile {
    pub fn compile(profile: Profile) -> Result<CompiledProfile> {
        let include = globs(&profile.files.include).context("files.include")?;
        let exclude = globs(&profile.files.exclude).context("files.exclude")?;

        let mut text_formats = Vec::new();
        for t in &profile.parse.output_templates {
            let regex =
                crate::user_regex::compile(&output_template_to_regex(t), false).with_context(|| format!("parse.output_templates: {t}"))?;
            let time_only = !t.contains("{Timestamp:yyyy") && !t.contains("{Timestamp}") && t.contains("{Timestamp:");
            text_formats.push(TextFormat {
                regex,
                time_only: time_only && !has_date_specifier(t),
            });
        }
        for r in &profile.parse.regexes {
            let regex = crate::user_regex::compile(r, false).with_context(|| format!("parse.regexes: {r}"))?;
            text_formats.push(TextFormat { regex, time_only: false });
        }

        let timestamps = TimestampParser::new(
            &profile.parse.timestamp_formats,
            &profile.parse.assume_timezone,
            profile.parse.day_first.resolve()?,
        )?;

        let mut extractors = Vec::new();
        for e in &profile.extract {
            let regex = crate::user_regex::compile(&e.regex, false).with_context(|| format!("extract \"{}\": regex", e.name))?;
            let when = compile_conditions(&e.when).with_context(|| format!("extract \"{}\": when", e.name))?;
            let mut fields: Vec<(String, FieldSpec)> = e.fields.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
            for (name, _) in &fields {
                if regex.capture_names().flatten().all(|g| g != name) {
                    return Err(anyhow!(
                        "extract \"{}\": field \"{name}\" has no capture group (?P<{name}>...) in the regex",
                        e.name
                    ));
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
                                kind: "auto".into(),
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

        if profile.marker.len() > 32 {
            return Err(anyhow!("at most 32 [[marker]] rules are supported"));
        }
        let mut markers = Vec::new();
        let mut lifecycle_mask = 0u32;
        for (i, m) in profile.marker.iter().enumerate() {
            let when = compile_conditions(&m.when).with_context(|| format!("marker \"{}\": when", m.kind))?;
            if m.lifecycle {
                lifecycle_mask |= 1 << i;
            }
            markers.push(Marker {
                kind: m.kind.clone(),
                label: m.label.clone().unwrap_or_else(|| m.kind.clone()),
                lifecycle: m.lifecycle,
                color: m.color.clone(),
                when,
            });
        }

        for c in &profile.chart {
            crate::chart::parse_y(&c.y).with_context(|| format!("chart \"{}\": y", c.title))?;
            crate::chart::parse_bucket(&c.bucket).with_context(|| format!("chart \"{}\": bucket", c.title))?;
            compile_conditions(&c.when).with_context(|| format!("chart \"{}\": when", c.title))?;
        }

        Ok(CompiledProfile {
            profile,
            include,
            exclude,
            text_formats,
            timestamps,
            extractors,
            markers,
            lifecycle_mask,
        })
    }

    /// Runs the extractors on a record, adding the produced fields. Returns
    /// the names of the fields it set.
    pub fn apply_extractors(&self, rec: &mut Record, source_name: &str) {
        for ex in &self.extractors {
            if let Some(values) = ex.extract(rec, source_name) {
                for (k, v) in values {
                    rec.fields.insert(k, v);
                }
            }
        }
    }

    pub fn compute_markers(&self, rec: &Record, source_name: &str) -> u32 {
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
            if let Some(v) = convert_value(m.as_str(), &spec.kind, unit, spec.to.as_deref()) {
                out.push((name.clone(), v));
            }
        }
        out
    }
}

fn convert_value(raw: &str, kind: &str, unit: Option<&str>, to: Option<&str>) -> Option<Value> {
    let raw = raw.trim();
    let num = |s: &str| -> Option<f64> {
        let cleaned: String = s.chars().filter(|c| *c != '_' && *c != ',' && *c != ' ').collect();
        cleaned.parse::<f64>().ok().filter(|f| f.is_finite())
    };
    match kind {
        "string" => Some(Value::String(raw.to_string())),
        "auto" => Some(auto_value(raw)),
        "number" => num(raw).map(Value::from),
        "integer" => num(raw).map(|f| Value::from(f.round() as i64)),
        "bool" => match raw.to_ascii_lowercase().as_str() {
            "true" | "yes" | "1" | "on" => Some(Value::Bool(true)),
            "false" | "no" | "0" | "off" => Some(Value::Bool(false)),
            _ => None,
        },
        "bytes" => {
            let (n, suffix) = split_number_suffix(raw);
            let n = num(n)?;
            let from = bytes_factor(unit.or(Some(suffix).filter(|s| !s.is_empty())).unwrap_or("B"))?;
            let to = bytes_factor(to.unwrap_or("MB"))?;
            Some(Value::from(round3(n * from / to)))
        }
        "duration" => {
            let ms = if raw.contains(':') {
                timespan_ms(raw)?
            } else {
                let (n, suffix) = split_number_suffix(raw);
                num(n)? * duration_factor(unit.or(Some(suffix).filter(|s| !s.is_empty())).unwrap_or("ms"))?
            };
            let to = duration_factor(to.unwrap_or("ms"))?;
            Some(Value::from(round3(ms / to)))
        }
        _ => Some(auto_value(raw)),
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

pub fn compile_conditions(cfgs: &[ConditionCfg]) -> Result<Vec<Condition>> {
    cfgs.iter().map(Condition::compile).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_profile_compiles() {
        let c = CompiledProfile::compile(Profile::default_profile()).unwrap();
        assert_eq!(c.markers.len(), 6);
        assert_eq!(c.lifecycle_mask, 0b110);
        assert!(!c.text_formats[0].time_only);
        assert!(c.text_formats[1].time_only);
    }

    #[test]
    fn partial_profile_merges_defaults() {
        let p = Profile::from_toml("name = \"Mine\"\n[parse]\nassume_timezone = \"local\"\n").unwrap();
        assert_eq!(p.name, "Mine");
        assert_eq!(p.parse.assume_timezone, "local");
        assert_eq!(p.parse.format, "auto");
        assert_eq!(p.marker.len(), 6);
    }

    #[test]
    fn reports_error_line() {
        let e = Profile::from_toml("name = \"x\"\n[parse\n").unwrap_err();
        assert_eq!(e.line, Some(2));
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
    fn day_first_values() {
        for (v, want) in [
            ("true", Some(true)),
            ("false", Some(false)),
            ("'yes'", Some(true)),
            ("'no'", Some(false)),
            ("'auto'", None),
        ] {
            let c = compile(&format!("[parse]\nday_first = {v}")).unwrap();
            assert_eq!(c.timestamps.day_first(), want, "{v}");
        }
        assert!(compile("[parse]\nday_first = 'sometimes'").is_err());
    }

    #[test]
    fn compile_errors() {
        assert!(compile("[files]\ninclude = ['[']").is_err());
        assert!(compile("[parse]\nregexes = ['(']").is_err());
        assert!(compile("[parse]\nassume_timezone = 'Mars'").is_err());
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
        assert_eq!(p.extract[0].fields["v"].kind, "auto");
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
        let cv = |raw: &str, kind: &str, unit: Option<&str>, to: Option<&str>| convert_value(raw, kind, unit, to);
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
        assert_eq!(cv("7", "mystery", None, None), Some(Value::from(7)));
    }
}
