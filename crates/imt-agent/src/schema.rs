//! A description of the config format for the UI's schema-driven editor:
//! every field with its label, kind, limits, default and help. Defaults are
//! read from the Rust defaults so the two cannot drift apart; validation
//! stays in [`crate::config`].

use crate::config::{AgentSection, LogSection, RetentionSection, CONFIG_VERSION};
use serde::Serialize;
use serde_json::{json, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum FieldKind {
    Text,
    Number,
    Boolean,
    /// `30s`, `15m`, `2h`, `1d`.
    Duration,
    Cron,
    Select,
    Path,
    /// A list of strings (one per line in the UI).
    List,
    /// An IMT query-language expression.
    Query,
    /// A JSON object (e.g. an inline collector plan).
    Json,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FieldSchema {
    pub key: String,
    pub label: String,
    pub kind: FieldKind,
    pub required: bool,
    pub help: String,
    pub default: Value,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<f64>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Choice>,
}

/// One variant of a tagged object (`kind = "..."`) and its fields.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct VariantSchema {
    pub kind: String,
    pub label: String,
    pub help: String,
    pub fields: Vec<FieldSchema>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentSchema {
    pub version: u32,
    pub agent: Vec<FieldSchema>,
    pub log: Vec<FieldSchema>,
    pub retention: Vec<FieldSchema>,
    pub schedule: Vec<FieldSchema>,
    /// `cron`, `every` or `trigger`.
    pub timing: Vec<VariantSchema>,
    pub triggers: Vec<VariantSchema>,
    pub actions: Vec<VariantSchema>,
}

struct F(FieldSchema);

fn field(key: &str, label: &str, kind: FieldKind, help: &str) -> F {
    F(FieldSchema {
        key: key.into(),
        label: label.into(),
        kind,
        required: false,
        help: help.into(),
        default: Value::Null,
        min: None,
        max: None,
        options: Vec::new(),
    })
}

impl F {
    fn required(mut self) -> F {
        self.0.required = true;
        self
    }
    fn default(mut self, v: Value) -> F {
        self.0.default = v;
        self
    }
    fn range(mut self, min: f64, max: f64) -> F {
        self.0.min = Some(min);
        self.0.max = Some(max);
        self
    }
    fn options(mut self, opts: &[(&str, &str)]) -> F {
        self.0.options = opts
            .iter()
            .map(|(v, l)| Choice {
                value: (*v).into(),
                label: (*l).into(),
            })
            .collect();
        self
    }
    fn done(self) -> FieldSchema {
        self.0
    }
}

fn variant(kind: &str, label: &str, help: &str, fields: Vec<FieldSchema>) -> VariantSchema {
    VariantSchema {
        kind: kind.into(),
        label: label.into(),
        help: help.into(),
        fields,
    }
}

const METRICS: &[(&str, &str)] = &[
    ("privateBytes", "Private bytes (MB)"),
    ("workingSet", "Working set (MB)"),
    ("virtualBytes", "Virtual bytes (MB)"),
    ("cpu", "CPU (% of the machine)"),
    ("handles", "Handles"),
    ("threads", "Threads"),
];

/// The schema of the current config version.
pub fn schema() -> AgentSchema {
    use FieldKind::*;
    let a = serde_json::to_value(AgentSection::default()).unwrap_or_default();
    let l = serde_json::to_value(LogSection::default()).unwrap_or_default();
    let r = serde_json::to_value(RetentionSection::default()).unwrap_or_default();
    AgentSchema {
        version: CONFIG_VERSION,
        agent: vec![
            field(
                "outputDir",
                "Output folder",
                Path,
                "Each run gets a folder here, with a manifest.json.",
            )
            .required()
            .done(),
            field(
                "stateDir",
                "State folder",
                Path,
                "Lock files and state.json. Default: <output>/.imt-agent.",
            )
            .done(),
            field("logDir", "Log folder", Path, "The agent's CLEF logs. Default: <state>/logs.").done(),
            field(
                "minFreeMb",
                "Minimum free space (MB)",
                Number,
                "A run is skipped when the output volume has less free space.",
            )
            .default(a["minFreeMb"].clone())
            .range(0.0, 10_000_000.0)
            .done(),
            field(
                "tickSecs",
                "Check every (s)",
                Number,
                "How often schedules and triggers are evaluated.",
            )
            .default(a["tickSecs"].clone())
            .range(1.0, 3600.0)
            .done(),
            field(
                "timezone",
                "Cron time zone",
                Select,
                "Whether cron expressions use local time or UTC.",
            )
            .default(a["timezone"].clone())
            .options(&[("local", "Local time"), ("utc", "UTC")])
            .done(),
            field(
                "logmanTimeoutSecs",
                "logman timeout (s)",
                Number,
                "How long one logman call of a PerfMon capture (start, stop, query) may take (Windows).",
            )
            .default(a["logmanTimeoutSecs"].clone())
            .range(
                crate::config::LOGMAN_TIMEOUT_SECS.0 as f64,
                crate::config::LOGMAN_TIMEOUT_SECS.1 as f64,
            )
            .done(),
        ],
        log: vec![
            field("level", "Log level", Select, "Least severe level written to the agent's log.")
                .default(l["level"].clone())
                .options(&[
                    ("debug", "Debug"),
                    ("information", "Information"),
                    ("warning", "Warning"),
                    ("error", "Error"),
                ])
                .done(),
            field("maxFileMb", "Roll at (MB)", Number, "Start a new log file at this size.")
                .default(l["maxFileMb"].clone())
                .range(1.0, 1024.0)
                .done(),
            field("keepFiles", "Files kept", Number, "Rolled log files kept.")
                .default(l["keepFiles"].clone())
                .range(1.0, 100.0)
                .done(),
            field("maxAgeDays", "Delete logs after (days)", Number, "Older log files are deleted.")
                .default(l["maxAgeDays"].clone())
                .range(1.0, 3650.0)
                .done(),
        ],
        retention: vec![
            field("maxRuns", "Keep at most (runs)", Number, "Empty: no limit.")
                .default(r["maxRuns"].clone())
                .range(1.0, 100_000.0)
                .done(),
            field(
                "maxTotalMb",
                "Keep at most (MB)",
                Number,
                "Oldest runs are deleted until the rest fit. Empty: no limit.",
            )
            .default(r["maxTotalMb"].clone())
            .range(1.0, 100_000_000.0)
            .done(),
            field("maxAgeDays", "Delete runs after (days)", Number, "Empty: no limit.")
                .default(r["maxAgeDays"].clone())
                .range(1.0, 36_500.0)
                .done(),
        ],
        schedule: vec![
            field("name", "Name", Text, "Letters, digits, spaces, '-', '_' and '.'; unique.")
                .required()
                .done(),
            field("enabled", "Enabled", Boolean, "Disabled schedules only run when started by name.")
                .default(json!(true))
                .done(),
        ],
        timing: vec![
            variant(
                "cron",
                "Cron expression",
                "minute hour day month weekday, e.g. 0 2 * * * (02:00 daily); @hourly, @daily, L, W and # work too.",
                vec![field("cron", "Cron", Cron, "").required().default(json!("0 2 * * *")).done()],
            ),
            variant(
                "every",
                "Fixed interval",
                "Between 1m and 31d.",
                vec![field("every", "Every", Duration, "e.g. 15m, 6h, 1d")
                    .required()
                    .default(json!("1h"))
                    .done()],
            ),
            variant("trigger", "Trigger", "Runs when a condition is met.", Vec::new()),
        ],
        triggers: vec![
            variant(
                "process",
                "Process threshold",
                "Fires when a matching process stays above the threshold; then waits for the cooldown and for the value to drop again.",
                vec![
                    field("process", "Process name", Text, "A glob such as OrderService*")
                        .required()
                        .done(),
                    field("metric", "Metric", Select, "")
                        .required()
                        .default(json!("privateBytes"))
                        .options(METRICS)
                        .done(),
                    field("above", "Above", Number, "MB for memory, % for CPU, a count otherwise.")
                        .required()
                        .range(0.0, 1e12)
                        .done(),
                    field("forSecs", "For at least (s)", Number, "")
                        .default(json!(0))
                        .range(0.0, 86_400.0)
                        .done(),
                    field("sampleSecs", "Sample every (s)", Number, "")
                        .default(json!(5))
                        .range(1.0, 3600.0)
                        .done(),
                    field("cooldownSecs", "Cooldown (s)", Number, "")
                        .default(json!(900))
                        .range(0.0, 604_800.0)
                        .done(),
                ],
            ),
            variant(
                "log",
                "Log event",
                "Fires when a new event in the watched log or folder matches the query.",
                vec![
                    field("path", "Log file or folder", Path, "").required().done(),
                    field("ql", "Query", Query, "e.g. level >= Error").required().done(),
                    field("pollSecs", "Check every (s)", Number, "")
                        .default(json!(5))
                        .range(1.0, 3600.0)
                        .done(),
                    field("cooldownSecs", "Cooldown (s)", Number, "")
                        .default(json!(900))
                        .range(0.0, 604_800.0)
                        .done(),
                ],
            ),
        ],
        actions: vec![
            variant(
                "collect",
                "Collect diagnostics",
                "Runs the diagnostics collector into an .imtz bundle.",
                vec![
                    field(
                        "planFile",
                        "Plan file",
                        Path,
                        "A collector plan (TOML or JSON). Empty: the inline plan or the default plan.",
                    )
                    .done(),
                    field("plan", "Inline plan", Json, "Collector plan keys as JSON, e.g. {\"msinfo\": true}.").done(),
                ],
            ),
            variant(
                "monitor",
                "Record process monitor",
                "Samples processes for a while into CLEF files.",
                vec![
                    field(
                        "targets",
                        "Processes",
                        List,
                        "Names (globs), executable paths or PIDs, one per line.",
                    )
                    .required()
                    .done(),
                    field("duration", "Duration", Duration, "1s to 24h")
                        .required()
                        .default(json!("5m"))
                        .done(),
                    field("intervalMs", "Interval (ms)", Number, "")
                        .default(json!(1000))
                        .range(250.0, 3_600_000.0)
                        .done(),
                    field("includeChildren", "Include child processes", Boolean, "")
                        .default(json!(false))
                        .done(),
                ],
            ),
            variant(
                "copyLogs",
                "Copy logs",
                "Copies log files, newest first, within the size limit.",
                vec![
                    field("from", "Folder or file", Path, "").required().done(),
                    field("include", "Include", List, "Globs, one per line. Empty: everything.").done(),
                    field("maxAgeDays", "Modified within (days)", Number, "Empty: any age.")
                        .range(1.0, 36_500.0)
                        .done(),
                    field("maxTotalMb", "At most (MB)", Number, "")
                        .default(json!(200))
                        .range(1.0, 100_000.0)
                        .done(),
                ],
            ),
            variant(
                "perfmon",
                "Capture PerfMon (Windows)",
                "Starts an installed Data Collector Set, stops it after the duration and copies its logs.",
                vec![
                    field("collector", "Collector set", Text, "Its name in Performance Monitor.")
                        .required()
                        .done(),
                    field("duration", "Duration", Duration, "1s to 24h")
                        .required()
                        .default(json!("5m"))
                        .done(),
                ],
            ),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn schema_matches_the_format() {
        let s = schema();
        assert_eq!(s.version, CONFIG_VERSION);
        let tick = s.agent.iter().find(|f| f.key == "tickSecs").unwrap();
        assert_eq!(tick.default, json!(15));
        let logman = s.agent.iter().find(|f| f.key == "logmanTimeoutSecs").unwrap();
        assert_eq!(
            (logman.default.clone(), logman.min, logman.max),
            (json!(120), Some(5.0), Some(600.0))
        );
        assert_eq!(s.retention[0].default, json!(50));
        assert_eq!(
            s.actions.iter().map(|a| a.kind.as_str()).collect::<Vec<_>>(),
            ["collect", "monitor", "copyLogs", "perfmon"]
        );
        assert_eq!(s.triggers.iter().map(|a| a.kind.as_str()).collect::<Vec<_>>(), ["process", "log"]);
        let json = serde_json::to_value(&s).unwrap();
        assert_eq!(json["agent"][0]["kind"], "path");
        assert_eq!(json["agent"][0]["required"], true);
        assert!(json["agent"][0].get("min").is_none());
        // Every action field name is accepted by the file format.
        for a in &s.actions {
            let mut obj = serde_json::Map::new();
            obj.insert("kind".into(), json!(a.kind));
            for f in &a.fields {
                let v = match f.kind {
                    FieldKind::List => json!(["x"]),
                    FieldKind::Number => json!(1),
                    FieldKind::Boolean => json!(true),
                    FieldKind::Json => json!({}),
                    FieldKind::Duration => json!("1m"),
                    _ => json!("x"),
                };
                obj.insert(f.key.clone(), v);
            }
            serde_json::from_value::<crate::config::ActionFile>(Value::Object(obj)).unwrap_or_else(|e| panic!("{}: {e}", a.kind));
        }
    }
}
