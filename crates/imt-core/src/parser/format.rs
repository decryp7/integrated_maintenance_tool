//! A parser's `[format]`: how a file's lines become events (the former profile `[parse]` section, ADR 0026).
//! Keys left out take the defaults below; `generic.toml` documents every key.

use anyhow::{anyhow, Result};
use serde::{Deserialize, Serialize};

/// `[format]` of a parser file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct FormatCfg {
    /// `auto` (the importers decide), `text`, `json`, `csv` or another importer's id (a plugin's).
    pub kind: String,
    /// Fields that name who wrote an event (host, application, process): they join `[instance] candidates`.
    pub identity: Vec<String>,
    /// Serilog output templates for text lines (tried first, in order).
    pub output_templates: Vec<String>,
    /// Regexes with named groups (`timestamp`, `level`, `message`, `exception`; others become fields).
    pub regexes: Vec<String>,
    /// Extra timestamp formats, .NET style or strftime.
    pub timestamp_formats: Vec<String>,
    /// Zone of timestamps without an offset: `UTC`, `local` or `+08:00`.
    pub assume_timezone: String,
    /// Ambiguous dates like 03/04/2026: `"auto"` (decided per file), true or false.
    pub day_first: DayFirst,
    pub json: JsonCfg,
    pub csv: CsvCfg,
}

impl Default for FormatCfg {
    fn default() -> Self {
        FormatCfg {
            kind: "auto".into(),
            identity: Vec::new(),
            output_templates: Vec::new(),
            regexes: Vec::new(),
            timestamp_formats: Vec::new(),
            assume_timezone: "UTC".into(),
            day_first: DayFirst::Text("auto".into()),
            json: JsonCfg::default(),
            csv: CsvCfg::default(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum DayFirst {
    Bool(bool),
    Text(String),
}

impl DayFirst {
    pub(crate) fn resolve(&self) -> Result<Option<bool>> {
        match self {
            DayFirst::Bool(b) => Ok(Some(*b)),
            DayFirst::Text(t) => match t.to_ascii_lowercase().as_str() {
                "auto" | "" => Ok(None),
                "true" | "yes" => Ok(Some(true)),
                "false" | "no" => Ok(Some(false)),
                _ => Err(anyhow!("format.day_first must be \"auto\", true or false")),
            },
        }
    }
}

fn keys(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

/// `[format.json]`: keys tried in order (nested keys with dots).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct JsonCfg {
    pub timestamp: Vec<String>,
    pub level: Vec<String>,
    pub message: Vec<String>,
    pub template: Vec<String>,
    pub exception: Vec<String>,
    pub properties: Vec<String>,
    pub default_level: String,
}

impl Default for JsonCfg {
    /// The keys of Serilog CLEF and JsonFormatter, NLog/ECS-style and most JSON loggers.
    fn default() -> Self {
        JsonCfg {
            timestamp: keys(&[
                "@t",
                "Timestamp",
                "timestamp",
                "@timestamp",
                "time",
                "ts",
                "date",
                "datetime",
                "timeMillis",
            ]),
            level: keys(&["@l", "Level", "level", "log.level", "severity", "lvl", "loglevel"]),
            message: keys(&["@m", "RenderedMessage", "Message", "message", "msg", "text"]),
            template: keys(&["@mt", "MessageTemplate"]),
            exception: keys(&[
                "@x",
                "Exception",
                "exception",
                "error.stack_trace",
                "stack_trace",
                "stacktrace",
                "error",
            ]),
            properties: keys(&["Properties", "properties", "fields", "context"]),
            default_level: "Information".into(),
        }
    }
}

/// `[format.csv]`: delimited files.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct CsvCfg {
    /// `auto` sniffs `,` `;` tab and `|`; else the delimiter (`tab` or `\t` for a tab).
    pub delimiter: String,
    pub has_header: bool,
    /// Column names for a file without a header row (missing or extra columns are `col<N>`).
    pub columns: Vec<String>,
    pub timestamp: String,
    pub level: String,
    pub message: String,
}

impl Default for CsvCfg {
    fn default() -> Self {
        CsvCfg {
            delimiter: "auto".into(),
            has_header: true,
            columns: Vec::new(),
            timestamp: "auto".into(),
            level: "auto".into(),
            message: "auto".into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_and_day_first() {
        let f = FormatCfg::default();
        assert_eq!((f.kind.as_str(), f.assume_timezone.as_str()), ("auto", "UTC"));
        assert!(f.json.timestamp.contains(&"@t".to_string()));
        assert!(f.csv.has_header && f.csv.columns.is_empty());
        assert_eq!(DayFirst::Bool(true).resolve().unwrap(), Some(true));
        assert_eq!(DayFirst::Text("no".into()).resolve().unwrap(), Some(false));
        assert_eq!(DayFirst::Text("yes".into()).resolve().unwrap(), Some(true));
        assert_eq!(DayFirst::Text("".into()).resolve().unwrap(), None);
        assert!(DayFirst::Text("maybe".into()).resolve().is_err());
    }
}
