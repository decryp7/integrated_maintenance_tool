use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};
use std::borrow::Cow;

/// Severity of a [`Record`], ordered from least to most severe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum Level {
    /// Very detailed tracing output.
    Verbose,
    /// Diagnostic detail useful while debugging.
    Debug,
    /// Normal, informational events.
    Information,
    /// Something unexpected that did not stop the operation.
    Warning,
    /// An operation failed.
    Error,
    /// A failure the application could not recover from.
    Fatal,
}

impl Level {
    /// Every level, in ascending severity.
    pub const ALL: [Level; 6] = [
        Level::Verbose,
        Level::Debug,
        Level::Information,
        Level::Warning,
        Level::Error,
        Level::Fatal,
    ];

    /// Position in [`Level::ALL`] (0 = `Verbose` ... 5 = `Fatal`).
    pub fn index(self) -> usize {
        self as usize
    }

    /// The canonical display name (`"Information"`, `"Warning"`, ...).
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
    /// most other loggers (log4net, NLog, Microsoft.Extensions.Logging, pino, java.util.logging,
    /// MySQL's `[Note]`, Android's one-letter V/D/I/W/E/F...).
    pub fn parse(s: &str) -> Option<Level> {
        let s = s.trim().trim_matches(|c| c == '[' || c == ']');
        let level = match s.to_ascii_lowercase().as_str() {
            "vrb" | "verbose" | "trace" | "trc" | "finest" | "finer" | "all" | "v" => Level::Verbose,
            "dbg" | "debug" | "dbug" | "fine" | "config" | "d" => Level::Debug,
            "inf" | "info" | "information" | "informational" | "notice" | "note" | "i" => Level::Information,
            "wrn" | "warn" | "warning" | "w" => Level::Warning,
            "err" | "error" | "eror" | "fail" | "failure" | "severe" | "e" => Level::Error,
            "ftl" | "fatal" | "critical" | "crit" | "crt" | "emergency" | "emerg" | "alert" | "panic" | "f" => Level::Fatal,
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

/// Free-form structured properties of a [`Record`], keyed by field name.
pub type Fields = Map<String, Value>;

/// Bit i set = marker kind i of the effective marker set matched.
pub type MarkerMask = u64;

/// Most marker kinds (profile, plugin and built-in together) a workspace can use.
pub const MAX_MARKER_KINDS: usize = MarkerMask::BITS as usize;

/// One event. Importers fill everything except `id`, `source` and `markers`,
/// which the workspace assigns.
#[derive(Debug, Clone)]
pub struct Record {
    /// Id assigned by the workspace; importers leave it 0.
    pub id: u32,
    /// Timestamp in milliseconds since the Unix epoch (UTC), when the event has one.
    pub ts: Option<i64>,
    /// Severity of the event.
    pub level: Level,
    /// The rendered message text.
    pub message: String,
    /// The unrendered message template, when the format has one.
    pub template: Option<String>,
    /// Exception or stack-trace text, when present.
    pub exception: Option<String>,
    /// Structured properties other than the standard ones above.
    pub fields: Fields,
    /// Index of the source file within the workspace, assigned by the workspace.
    pub source: u32,
    /// One-based line number in the source file where the event starts.
    pub line: u32,
    /// Bit i set = marker kind i of the effective marker set matched.
    pub markers: MarkerMask,
}

impl Record {
    /// Creates an empty `Information` record that starts at `line`.
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

    /// Value of a field or pseudo-field as a number; `@line` and numeric, numeric-string or boolean fields qualify.
    pub fn field_number(&self, name: &str) -> Option<f64> {
        match name {
            "@line" => Some(self.line as f64),
            _ => self.fields.get(name).and_then(value_number),
        }
    }
}

/// Renders a JSON value as text: strings verbatim, other values as compact JSON.
pub fn value_text(v: &Value) -> Cow<'_, str> {
    match v {
        Value::String(s) => Cow::Borrowed(s),
        other => Cow::Owned(other.to_string()),
    }
}

/// Reads a JSON value as a finite number: numbers, numeric strings and booleans (1 or 0); anything else is `None`.
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
        for (s, l) in [
            ("Note", Level::Information),
            ("CONFIG", Level::Debug),
            ("finer", Level::Verbose),
            ("D", Level::Debug),
            ("F", Level::Fatal),
        ] {
            assert_eq!(Level::parse(s), Some(l), "{s}");
        }
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
