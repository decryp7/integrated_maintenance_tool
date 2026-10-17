use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Level {
    Verbose,
    Debug,
    Information,
    Warning,
    Error,
    Fatal,
}

impl Level {
    pub const ALL: [Level; 6] = [
        Level::Verbose,
        Level::Debug,
        Level::Information,
        Level::Warning,
        Level::Error,
        Level::Fatal,
    ];

    pub fn index(self) -> usize {
        self as usize
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Level::Verbose => "Verbose",
            Level::Debug => "Debug",
            Level::Information => "Information",
            Level::Warning => "Warning",
            Level::Error => "Error",
            Level::Fatal => "Fatal",
        }
    }

    /// Accepts Serilog names and abbreviations as well as the names used by
    /// most other loggers (log4net, NLog, Microsoft.Extensions.Logging, pino...).
    pub fn parse(s: &str) -> Option<Level> {
        let s = s.trim().trim_matches(|c| c == '[' || c == ']');
        let level = match s.to_ascii_lowercase().as_str() {
            "vrb" | "verbose" | "trace" | "trc" | "finest" | "all" => Level::Verbose,
            "dbg" | "debug" | "dbug" | "fine" => Level::Debug,
            "inf" | "info" | "information" | "informational" | "notice" | "i" => Level::Information,
            "wrn" | "warn" | "warning" | "w" => Level::Warning,
            "err" | "error" | "eror" | "fail" | "failure" | "severe" | "e" => Level::Error,
            "ftl" | "fatal" | "critical" | "crit" | "crt" | "emergency" | "alert" | "panic" => Level::Fatal,
            _ => return None,
        };
        Some(level)
    }

    /// pino / bunyan numeric levels.
    pub fn from_number(n: f64) -> Level {
        match n as i64 {
            i64::MIN..=10 => Level::Verbose,
            11..=20 => Level::Debug,
            21..=30 => Level::Information,
            31..=40 => Level::Warning,
            41..=50 => Level::Error,
            _ => Level::Fatal,
        }
    }
}

pub type Fields = Map<String, Value>;

/// One event. Importers fill everything except `id`, `source` and `markers`,
/// which the workspace assigns.
#[derive(Debug, Clone)]
pub struct Record {
    pub id: u32,
    pub ts: Option<i64>,
    pub level: Level,
    pub message: String,
    pub template: Option<String>,
    pub exception: Option<String>,
    pub fields: Fields,
    pub source: u32,
    pub line: u32,
    /// Bit i set = marker rule i of the profile matched.
    pub markers: u32,
}

impl Record {
    pub fn new(line: u32) -> Self {
        Record {
            id: 0,
            ts: None,
            level: Level::Information,
            message: String::new(),
            template: None,
            exception: None,
            fields: Fields::new(),
            source: 0,
            line,
            markers: 0,
        }
    }

    /// Value of a field or pseudo-field as text.
    pub fn field_text<'a>(&'a self, name: &str, source_name: &'a str) -> Option<Cow<'a, str>> {
        match name {
            "@level" => Some(Cow::Borrowed(self.level.as_str())),
            "@message" => Some(Cow::Borrowed(&self.message)),
            "@exception" => self.exception.as_deref().map(Cow::Borrowed),
            "@template" => self.template.as_deref().map(Cow::Borrowed),
            "@source" => Some(Cow::Borrowed(source_name)),
            "@line" => Some(Cow::Owned(self.line.to_string())),
            _ => self.fields.get(name).map(value_text),
        }
    }

    pub fn field_number(&self, name: &str) -> Option<f64> {
        match name {
            "@line" => Some(self.line as f64),
            _ => self.fields.get(name).and_then(value_number),
        }
    }
}

pub fn value_text(v: &Value) -> Cow<'_, str> {
    match v {
        Value::String(s) => Cow::Borrowed(s),
        other => Cow::Owned(other.to_string()),
    }
}

pub fn value_number(v: &Value) -> Option<f64> {
    match v {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => s.trim().parse::<f64>().ok().filter(|f| f.is_finite()),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// Turns a captured string into the most natural JSON value: integers and
/// floats become numbers, everything else stays a string.
pub fn auto_value(s: &str) -> Value {
    let t = s.trim();
    if !t.is_empty() && t.len() < 20 {
        if let Ok(i) = t.parse::<i64>() {
            // Keep leading-zero identifiers ("007") as text.
            if !(t.len() > 1 && t.starts_with('0')) {
                return Value::from(i);
            }
        } else if let Ok(f) = t.parse::<f64>() {
            if f.is_finite() && t.contains('.') {
                return Value::from(f);
            }
        }
    }
    Value::String(s.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels() {
        for l in Level::ALL {
            assert_eq!(Level::parse(l.as_str()), Some(l));
            assert_eq!(Level::ALL[l.index()], l);
        }
        assert_eq!(Level::parse("[WRN]"), Some(Level::Warning));
        assert_eq!(Level::parse("crit"), Some(Level::Fatal));
        assert_eq!(Level::parse("nonsense"), None);
        assert_eq!(Level::from_number(10.0), Level::Verbose);
        assert_eq!(Level::from_number(20.0), Level::Debug);
        assert_eq!(Level::from_number(30.0), Level::Information);
        assert_eq!(Level::from_number(40.0), Level::Warning);
        assert_eq!(Level::from_number(50.0), Level::Error);
        assert_eq!(Level::from_number(60.0), Level::Fatal);
    }

    #[test]
    fn values() {
        use serde_json::json;
        assert_eq!(value_number(&json!(true)), Some(1.0));
        assert_eq!(value_number(&json!(false)), Some(0.0));
        assert_eq!(value_number(&json!(" 2.5 ")), Some(2.5));
        assert_eq!(value_number(&json!("NaN")), None);
        assert_eq!(value_number(&json!(null)), None);
        assert_eq!(value_text(&json!({"a": 1})), "{\"a\":1}");
        assert_eq!(auto_value("42"), json!(42));
        assert_eq!(auto_value("007"), json!("007"));
        assert_eq!(auto_value("1.5"), json!(1.5));
        assert_eq!(auto_value("1e5"), json!("1e5"));
        assert_eq!(auto_value("abc"), json!("abc"));

        let mut r = Record::new(7);
        r.template = Some("T".into());
        r.fields.insert("n".into(), json!(3));
        assert_eq!(r.field_text("@template", "s").unwrap(), "T");
        assert_eq!(r.field_text("@source", "s").unwrap(), "s");
        assert_eq!(r.field_text("@line", "s").unwrap(), "7");
        assert_eq!(r.field_text("@exception", "s"), None);
        assert_eq!(r.field_text("n", "s").unwrap(), "3");
        assert_eq!(r.field_number("@line"), Some(7.0));
        assert_eq!(r.field_number("n"), Some(3.0));
    }
}
