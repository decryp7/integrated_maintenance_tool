//! IMT's own logs in Serilog's compact JSON format (CLEF), one event per
//! line, so they can be opened in IMT itself or in Seq and other Serilog tools.

use chrono::{DateTime, SecondsFormat, Utc};
use log::kv::{Key, Value as KvValue, VisitSource};
use serde_json::{Map, Value};

/// Properties attached to every event.
pub struct Enrichment {
    pub application: &'static str,
    pub version: &'static str,
    pub process_id: u32,
}

impl Enrichment {
    pub fn current() -> Enrichment {
        Enrichment {
            application: "IMT",
            version: env!("CARGO_PKG_VERSION"),
            process_id: std::process::id(),
        }
    }
}

/// Formats one record as a CLEF line (without the trailing newline).
pub fn format(time: DateTime<Utc>, record: &log::Record, message: &str, enrich: &Enrichment) -> String {
    let mut event = Map::new();
    event.insert("@t".into(), Value::from(time.to_rfc3339_opts(SecondsFormat::Micros, true)));
    // CLEF omits @l for Information.
    if let Some(level) = clef_level(record.level()) {
        event.insert("@l".into(), Value::from(level));
    }
    event.insert("@m".into(), Value::from(message));
    event.insert("SourceContext".into(), Value::from(record.target()));
    event.insert("Application".into(), Value::from(enrich.application));
    event.insert("Version".into(), Value::from(enrich.version));
    event.insert("ProcessId".into(), Value::from(enrich.process_id));

    let mut props = Props(Map::new());
    let _ = record.key_values().visit(&mut props);
    for (k, v) in props.0 {
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
            application: "IMT",
            version: "1.2.3",
            process_id: 42,
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
            .target("imt_app")
            .args(format_args!("x"))
            .build();
        let e = parse(&format(t, &info, "Application starting — IMT 1.2.3", &enrich()));
        assert_eq!(e["@t"], "2026-10-17T08:30:00.000000Z");
        assert!(!e.contains_key("@l"));
        assert_eq!(e["@m"], "Application starting — IMT 1.2.3");
        assert_eq!(e["SourceContext"], "imt_app");
        assert_eq!(
            (e["Application"].clone(), e["Version"].clone(), e["ProcessId"].clone()),
            ("IMT".into(), "1.2.3".into(), 42.into())
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
        assert_eq!(Enrichment::current().application, "IMT");
    }
}
