//! Debrief's own logs in Serilog's compact JSON format (CLEF), one event per
//! line, so they can be opened in Debrief itself or in Seq and other Serilog tools.

use chrono::{DateTime, SecondsFormat, Utc};
use log::kv::{Key, Value as KvValue, VisitSource};
use serde_json::{Map, Value};

/// Properties attached to every event.
pub struct Enrichment {
    pub application: &'static str,
    pub version: &'static str,
    pub process_id: u32,
    /// Build commit, OS and the run's session id, so a log or report can be matched to the exact code.
    pub commit: &'static str,
    pub os: &'static str,
    pub session_id: &'static str,
}

impl Enrichment {
    pub fn current() -> Enrichment {
        Enrichment {
            application: "Debrief",
            version: env!("CARGO_PKG_VERSION"),
            process_id: std::process::id(),
            commit: crate::activity::build_commit(),
            os: std::env::consts::OS,
            session_id: crate::activity::session_id(),
        }
    }
}

/// Formats one record as a CLEF line (without the trailing newline). The app formats `tracing` events
/// with [`format_event`]; this is its `log` counterpart, used by tests.
#[cfg(test)]
pub fn format(time: DateTime<Utc>, record: &log::Record, message: &str, enrich: &Enrichment) -> String {
    format_event(
        time,
        record.level(),
        record.target(),
        message,
        key_values(record.key_values()),
        enrich,
    )
}

/// A `log` record's key/values as JSON (booleans and numbers keep their type).
pub fn key_values(source: &dyn log::kv::Source) -> Map<String, Value> {
    let mut props = Props(Map::new());
    let _ = source.visit(&mut props);
    props.0
}

/// Formats one event as a CLEF line (without the trailing newline): `target` is its `SourceContext`
/// (a `SourceContext` property wins), `props` its properties.
pub fn format_event(
    time: DateTime<Utc>,
    level: log::Level,
    target: &str,
    message: &str,
    props: Map<String, Value>,
    enrich: &Enrichment,
) -> String {
    let mut event = Map::new();
    event.insert("@t".into(), Value::from(time.to_rfc3339_opts(SecondsFormat::Micros, true)));
    // CLEF omits @l for Information.
    if let Some(level) = clef_level(level) {
        event.insert("@l".into(), Value::from(level));
    }
    event.insert("@m".into(), Value::from(message));
    event.insert("SourceContext".into(), Value::from(target));
    event.insert("Application".into(), Value::from(enrich.application));
    event.insert("Version".into(), Value::from(enrich.version));
    event.insert("ProcessId".into(), Value::from(enrich.process_id));
    event.insert("Commit".into(), Value::from(enrich.commit));
    event.insert("Os".into(), Value::from(enrich.os));
    event.insert("SessionId".into(), Value::from(enrich.session_id));

    for (k, v) in props {
        // `Severity = "Fatal"` raises the level beyond what the `log` crate has (crashes).
        if k == "Severity" {
            if let Some(level) = v.as_str().filter(|l| *l == "Fatal") {
                event.insert("@l".into(), Value::from(level));
                continue;
            }
        }
        // A message template (`User {Operation} via {Trigger}`) becomes CLEF's `@mt`, next to the rendered `@m`.
        if k == "MessageTemplate" {
            event.insert("@mt".into(), v);
            continue;
        }
        // Keys starting with '@' are reserved by CLEF and escaped as '@@'.
        let k = if k.starts_with('@') { format!("@{k}") } else { k };
        event.insert(k, v);
    }
    Value::Object(event).to_string()
}

fn clef_level(level: log::Level) -> Option<&'static str> {
    match level {
        log::Level::Error => Some("Error"),
        log::Level::Warn => Some("Warning"),
        log::Level::Info => None,
        log::Level::Debug => Some("Debug"),
        log::Level::Trace => Some("Verbose"),
    }
}

struct Props(Map<String, Value>);

impl<'kvs> VisitSource<'kvs> for Props {
    fn visit_pair(&mut self, key: Key<'kvs>, value: KvValue<'kvs>) -> Result<(), log::kv::Error> {
        let v = if let Some(b) = value.to_bool() {
            Value::from(b)
        } else if let Some(i) = value.to_i64() {
            Value::from(i)
        } else if let Some(f) = value.to_f64() {
            Value::from(f)
        } else {
            Value::from(value.to_string())
        };
        self.0.insert(key.as_str().to_string(), v);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn enrich() -> Enrichment {
        Enrichment {
            application: "Debrief",
            version: "1.2.3",
            process_id: 42,
            commit: "abc1234",
            os: "macos",
            session_id: "0123456789abcdef",
        }
    }

    fn parse(line: &str) -> Map<String, Value> {
        serde_json::from_str::<Value>(line).unwrap().as_object().unwrap().clone()
    }

    #[test]
    fn formats_levels_and_enrichment() {
        let t = Utc.with_ymd_and_hms(2026, 10, 17, 8, 30, 0).unwrap();
        let info = log::Record::builder()
            .level(log::Level::Info)
            .target("debrief_app")
            .args(format_args!("x"))
            .build();
        let e = parse(&format(t, &info, "Application starting — Debrief 1.2.3", &enrich()));
        assert_eq!(e["@t"], "2026-10-17T08:30:00.000000Z");
        assert!(!e.contains_key("@l"));
        assert_eq!(e["@m"], "Application starting — Debrief 1.2.3");
        assert_eq!(e["SourceContext"], "debrief_app");
        assert_eq!(
            (e["Application"].clone(), e["Version"].clone(), e["ProcessId"].clone()),
            ("Debrief".into(), "1.2.3".into(), 42.into())
        );

        for (level, want) in [
            (log::Level::Error, "Error"),
            (log::Level::Warn, "Warning"),
            (log::Level::Debug, "Debug"),
            (log::Level::Trace, "Verbose"),
        ] {
            let r = log::Record::builder().level(level).target("t").args(format_args!("m")).build();
            assert_eq!(parse(&format(t, &r, "m", &enrich()))["@l"], want);
        }
    }

    #[test]
    fn includes_key_values() {
        let t = Utc.with_ymd_and_hms(2026, 10, 17, 8, 30, 0).unwrap();
        let kvs: &[(&str, KvValue)] = &[
            ("Sources", KvValue::from(6u64)),
            ("Ok", KvValue::from(true)),
            ("Ratio", KvValue::from(0.5f64)),
            ("@x", KvValue::from("e")),
        ];
        let ctx: &[(&str, &str)] = &[("SourceContext", "ui.timeline")];
        let r = log::Record::builder()
            .level(log::Level::Info)
            .target("webview")
            .key_values(&kvs)
            .args(format_args!("m"))
            .build();
        let e = parse(&format(t, &r, "Workspace opened", &enrich()));
        assert_eq!(e["Sources"], 6);
        assert_eq!(e["Ok"], true);
        assert_eq!(e["Ratio"], 0.5);
        assert_eq!(e["@@x"], "e");
        // A SourceContext key/value from the UI overrides the target.
        let r = log::Record::builder()
            .level(log::Level::Info)
            .target("webview")
            .key_values(&ctx)
            .args(format_args!("m"))
            .build();
        assert_eq!(parse(&format(t, &r, "m", &enrich()))["SourceContext"], "ui.timeline");
        let e = parse(&format(t, &r, "m", &enrich()));
        assert_eq!(
            (e["Commit"].clone(), e["Os"].clone(), e["SessionId"].clone()),
            ("abc1234".into(), "macos".into(), "0123456789abcdef".into())
        );
        // Fatal severity and message templates.
        let kvs: &[(&str, &str)] = &[
            ("Severity", "Fatal"),
            ("MessageTemplate", "User {Operation} via {Trigger}"),
            ("Operation", "x"),
        ];
        let r = log::Record::builder()
            .level(log::Level::Error)
            .target("t")
            .key_values(&kvs)
            .args(format_args!("m"))
            .build();
        let e = parse(&format(t, &r, "User x via menu", &enrich()));
        assert_eq!(
            (e["@l"].clone(), e["@mt"].clone()),
            ("Fatal".into(), "User {Operation} via {Trigger}".into())
        );
        assert!(!e.contains_key("Severity") && !e.contains_key("MessageTemplate"));
        let kvs: &[(&str, &str)] = &[("Severity", "Minor")];
        let r = log::Record::builder()
            .level(log::Level::Warn)
            .target("t")
            .key_values(&kvs)
            .args(format_args!("m"))
            .build();
        let e = parse(&format(t, &r, "m", &enrich()));
        assert_eq!((e["@l"].clone(), e["Severity"].clone()), ("Warning".into(), "Minor".into()));
        assert_eq!(Enrichment::current().application, "Debrief");
    }
}
