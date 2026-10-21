//! log4j-style XML events, as written by log4j 1.x `XMLLayout`, log4net
//! (`XmlLayoutSchemaLog4j` and its own `XmlLayout`) and NLog
//! (`Log4JXmlEventLayout`):
//!
//! ```xml
//! <log4j:event logger="Billing" timestamp="1791892800000" level="INFO" thread="7">
//!   <log4j:message><![CDATA[Invoice written]]></log4j:message>
//!   <log4j:throwable><![CDATA[System.Exception: ...]]></log4j:throwable>
//!   <log4j:locationInfo class="Billing.Writer" method="Flush" file="Writer.cs" line="42"/>
//!   <log4j:properties><log4j:data name="log4jmachinename" value="BILL01"/></log4j:properties>
//! </log4j:event>
//! ```
//!
//! Such files are a sequence of events with no root element (the appender
//! appends one event at a time), so they are not well-formed XML. Events are
//! found and parsed one at a time: any or no root, any namespace prefix, a
//! truncated last event or a malformed event in the middle only affect that
//! event. `timestamp` is epoch milliseconds (log4j) or ISO 8601 (log4net).

use crate::MAX_FIELDS;
use anyhow::Result;
use chrono::DateTime;
use imt_plugin_api::importer::{ImportCtx, Imported, TextImporter, MAX_LINE_LEN};
use imt_plugin_api::model::{Level, Record};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use regex::Regex;
use serde_json::Value;
use std::sync::OnceLock;

/// Child elements nested deeper than this inside one event are ignored.
const MAX_DEPTH: usize = 16;

pub struct Log4jXmlImporter;

/// The start of an event element: `<log4j:event`, `<log4net:event`, `<event`.
fn start_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"<(?:[A-Za-z_][\w.-]{0,31}:)?event[\s/>]").expect("static regex"))
}

impl TextImporter for Log4jXmlImporter {
    fn id(&self) -> &'static str {
        "log4j-xml"
    }

    /// An event start tag near the start of the file with a `logger` and a
    /// `level` or `timestamp` attribute.
    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let head = &head[..floor_boundary(head, 16 * 1024)];
        let Some(m) = start_re().find(head) else { return 0 };
        let rest = &head[m.start()..];
        let tag = &rest[..rest.find('>').unwrap_or(rest.len())];
        let has =
            |attr: &str| tag.contains(&format!(" {attr}=")) || tag.contains(&format!("\n{attr}=")) || tag.contains(&format!("\t{attr}="));
        if has("logger") && (has("level") || has("timestamp")) {
            95
        } else {
            0
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        Ok(import(ctx, content))
    }

    /// Events are independent, so appended text parses on its own.
    fn import_tail(&self, ctx: &ImportCtx, _head: &str, tail: &str) -> Option<Result<Imported>> {
        Some(Ok(import(ctx, tail)))
    }
}

/// The largest char boundary at or below `n`.
fn floor_boundary(s: &str, n: usize) -> usize {
    let mut end = n.min(s.len());
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    end
}

fn import(ctx: &ImportCtx, content: &str) -> Imported {
    let mut out = Imported::default();
    let (mut pos, mut line, mut counted) = (0usize, 1u32, 0usize);
    while let Some(m) = start_re().find_at(content, pos) {
        let start = m.start();
        line = line.saturating_add(content[counted..start].bytes().filter(|b| *b == b'\n').count() as u32);
        counted = start;
        let (record, used) = parse_event(ctx, &content[start..], line);
        match record {
            Some(r) => out.records.push(r),
            None => out.skipped += 1,
        }
        // Always move forward, to a character boundary.
        pos = start + used.max(1);
        while pos < content.len() && !content.is_char_boundary(pos) {
            pos += 1;
        }
    }
    out
}

/// Parses the event at the start of `text`. Returns the record (None when
/// the event is unusable) and how many bytes it used. An event that is cut
/// off (end of file, or the next event starts) keeps what was read.
fn parse_event(ctx: &ImportCtx, text: &str, line: u32) -> (Option<Record>, usize) {
    let mut reader = Reader::from_str(text);
    reader.config_mut().check_end_names = false;
    let mut r = Record::new(line);
    match reader.read_event() {
        // '<' is never allowed inside a tag: the tag runs into the next one.
        Ok(Event::Start(e) | Event::Empty(e)) if e.attributes_raw().contains('<') => return (None, 1),
        Ok(Event::Start(e)) => event_attributes(ctx, &e, &mut r),
        Ok(Event::Empty(e)) => {
            event_attributes(ctx, &e, &mut r);
            return (Some(r), reader.buffer_position() as usize);
        }
        _ => return (None, 1),
    }
    let mut path: Vec<String> = Vec::new();
    let mut buf = String::new();
    loop {
        let before = reader.buffer_position() as usize;
        if before > MAX_LINE_LEN {
            return (None, before);
        }
        match reader.read_event() {
            Err(_) => return (None, 1),
            Ok(Event::Eof) => return (Some(cut_off(r, &path, buf)), text.len()),
            Ok(Event::Start(e)) => {
                let name = e.local_name().as_ref().to_string();
                if name == "event" {
                    return (Some(cut_off(r, &path, buf)), before);
                }
                child_attributes(&name, &e, &mut r);
                buf.clear();
                path.push(name);
            }
            Ok(Event::Empty(e)) => {
                let name = e.local_name().as_ref().to_string();
                if name == "event" {
                    return (Some(cut_off(r, &path, buf)), before);
                }
                child_attributes(&name, &e, &mut r);
            }
            Ok(Event::Text(t)) if !path.is_empty() => buf.push_str(&t.xml_content(XmlVersion::Implicit1_0)),
            Ok(Event::CData(c)) if !path.is_empty() => buf.push_str(&c),
            Ok(Event::GeneralRef(g)) if !path.is_empty() => match g.resolve_char_ref() {
                Ok(Some(c)) => buf.push(c),
                _ => buf.push_str(match g.as_ref() {
                    "amp" => "&",
                    "lt" => "<",
                    "gt" => ">",
                    "quot" => "\"",
                    "apos" => "'",
                    _ => "",
                }),
            },
            Ok(Event::End(_)) => {
                // An end tag with no open child closes the event (whatever its name).
                let Some(name) = path.pop() else {
                    return (Some(r), reader.buffer_position() as usize);
                };
                if path.is_empty() {
                    child_text(&name, std::mem::take(&mut buf), &mut r);
                }
                buf.clear();
            }
            Ok(_) => {}
        }
        if path.len() > MAX_DEPTH {
            return (None, reader.buffer_position() as usize);
        }
    }
}

/// An event that ends early keeps the text of the child it was in.
fn cut_off(mut r: Record, path: &[String], buf: String) -> Record {
    if let [child] = path {
        child_text(child, buf, &mut r);
    }
    r
}

/// Attribute values of a start tag as (local name, value), skipping namespace declarations.
fn attributes<'a>(e: &'a BytesStart<'a>) -> impl Iterator<Item = (String, String)> + 'a {
    let mut attrs = e.attributes();
    attrs.with_checks(false);
    attrs.flatten().filter_map(|a| {
        let key = a.key.as_ref();
        if key == "xmlns" || key.starts_with("xmlns:") {
            return None;
        }
        let local = key.rsplit(':').next().unwrap_or(key).to_string();
        let value = a
            .normalized_value(XmlVersion::Implicit1_0)
            .map(|v| v.into_owned())
            .unwrap_or_else(|_| a.value.to_string());
        Some((local, value))
    })
}

fn event_attributes(ctx: &ImportCtx, e: &BytesStart, r: &mut Record) {
    for (key, value) in attributes(e).take(MAX_FIELDS) {
        match key.as_str() {
            "level" => r.level = parse_level(&value),
            "timestamp" => r.ts = parse_timestamp(ctx, &value),
            "logger" => set(r, "Logger", Value::from(value)),
            "thread" => set(r, "Thread", Value::from(value)),
            "domain" => set(r, "Domain", Value::from(value)),
            "username" => set(r, "UserName", Value::from(value)),
            "identity" if !value.is_empty() => set(r, "Identity", Value::from(value)),
            "identity" => {}
            other => set(r, other, Value::from(value)),
        }
    }
}

/// `locationInfo`, `data` (properties) and NLog's `locationInfo assembly`.
fn child_attributes(name: &str, e: &BytesStart, r: &mut Record) {
    match name {
        "locationInfo" => {
            for (key, value) in attributes(e) {
                match key.as_str() {
                    "class" => set(r, "Class", Value::from(value)),
                    "method" => set(r, "Method", Value::from(value)),
                    "file" if !value.is_empty() => set(r, "File", Value::from(value)),
                    "line" => {
                        if let Ok(n) = value.parse::<u64>() {
                            set(r, "Line", Value::from(n))
                        }
                    }
                    "assembly" => set(r, "Assembly", Value::from(value)),
                    _ => {}
                }
            }
        }
        "data" => {
            let (mut key, mut val) = (None, None);
            for (k, v) in attributes(e) {
                match k.as_str() {
                    "name" => key = Some(v),
                    "value" => val = Some(v),
                    _ => {}
                }
            }
            if let (Some(k), Some(v)) = (key, val) {
                let name = match k.as_str() {
                    "log4jmachinename" | "log4net:HostName" => "MachineName",
                    "log4japp" => "Application",
                    "log4net:UserName" => "UserName",
                    "log4net:Identity" => "Identity",
                    other => other,
                }
                .to_string();
                if r.fields.len() < MAX_FIELDS && !r.fields.contains_key(&name) {
                    r.fields.insert(name, Value::from(v));
                }
            }
        }
        _ => {}
    }
}

/// Text of a direct child of the event.
fn child_text(name: &str, text: String, r: &mut Record) {
    match name {
        "message" => r.message = text.trim_matches(['\r', '\n']).to_string(),
        "throwable" | "exception" => {
            let t = text.trim_end();
            if !t.trim().is_empty() {
                r.exception = Some(t.trim_start_matches(['\r', '\n']).to_string());
            }
        }
        "eventSequenceNumber" => match text.trim().parse::<u64>() {
            Ok(n) => set(r, "SequenceNumber", Value::from(n)),
            Err(_) => set(r, "SequenceNumber", Value::from(text.trim())),
        },
        "properties" | "locationInfo" => {}
        other => {
            let t = text.trim();
            if !t.is_empty() {
                set(r, other, Value::from(t));
            }
        }
    }
}

fn set(r: &mut Record, key: &str, value: Value) {
    if r.fields.len() < MAX_FIELDS || r.fields.contains_key(key) {
        r.fields.insert(key.to_string(), value);
    }
}

/// log4j, log4net and NLog level names; unknown ones are Information.
fn parse_level(s: &str) -> Level {
    Level::parse(s).unwrap_or_else(|| match s.trim().to_ascii_lowercase().as_str() {
        "finer" | "verbose" => Level::Verbose,
        "emergency" | "off" => Level::Fatal,
        _ => Level::Information,
    })
}

/// Epoch milliseconds (log4j, NLog) or ISO 8601 with an offset (log4net);
/// anything else goes through the profile's timestamp parser.
fn parse_timestamp(ctx: &ImportCtx, s: &str) -> Option<i64> {
    let s = s.trim();
    if !s.is_empty() && s.len() <= 16 && s.bytes().all(|b| b.is_ascii_digit()) {
        return s.parse::<i64>().ok();
    }
    DateTime::parse_from_rfc3339(s)
        .ok()
        .map(|d| d.timestamp_millis())
        .or_else(|| ctx.timestamps.parse(s))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{garbage, ms, with_ctx};

    const LOG4J: &str = r#"<?xml version="1.0" ?>
<log4j:event logger="Billing.Invoices" timestamp="1792238400123" level="WARN" thread="7">
  <log4j:message><![CDATA[Waiting for <payment> confirmation]]></log4j:message>
  <log4j:NDC>order-88</log4j:NDC>
  <log4j:locationInfo class="Billing.Writer" method="Flush" file="Writer.cs" line="42"/>
  <log4j:properties>
    <log4j:data name="log4jmachinename" value="BILL01"/>
    <log4j:data name="log4japp" value="Billing.exe"/>
    <log4j:data name="tenant" value="contoso"/>
  </log4j:properties>
</log4j:event>
<log4j:event logger="Billing.Erp" timestamp="1792238401000" level="ERROR" thread="9"><log4j:message>Post failed &amp; retried &#x263A;</log4j:message><log4j:throwable><![CDATA[
System.ServiceModel.CommunicationException: closed
   at Billing.Erp.ErpClient.Post()
]]></log4j:throwable></log4j:event>
"#;

    /// NLog's Log4JXmlEventLayout: namespace declarations, nlog: extras.
    const NLOG: &str = r#"<log4j:event logger="Scheduler" level="DEBUG" timestamp="1792238402000" thread="1" xmlns:log4j="http://jakarta.apache.org/log4j/" xmlns:nlog="http://nlog-project.org"><log4j:message>tick</log4j:message><nlog:eventSequenceNumber>17</nlog:eventSequenceNumber><nlog:locationInfo assembly="Scheduler, Version=1.0.0.0"/><log4j:properties><log4j:data name="log4jmachinename" value="SCHED01" /></log4j:properties></log4j:event>
<log4j:event logger="Scheduler" level="Fatal" timestamp="1792238403000" thread="1"><log4j:message>stopping</log4j:message><nlog:eventSequenceNumber>x</nlog:eventSequenceNumber></log4j:event>"#;

    /// log4net XmlLayout: own prefix, ISO timestamps, inside a root element.
    const LOG4NET: &str = r#"<log4net:events>
<log4net:event logger="Gateway" timestamp="2026-10-14T12:00:00.1234567+02:00" level="NOTICE" thread="3" domain="Gateway.exe" username="CORP\svc" identity="">
<log4net:message>ready</log4net:message>
<log4net:properties><log4net:data name="log4net:HostName" value="GW01" /><log4net:data name="log4net:UserName" value="CORP\svc"/></log4net:properties>
<log4net:exception>System.Exception: x</log4net:exception>
<log4net:custom> extra </log4net:custom>
</log4net:event>
</log4net:events>"#;

    #[test]
    fn parses_log4j_events() {
        let out = with_ctx(|c| {
            assert_eq!(Log4jXmlImporter.sniff(c, LOG4J), 95);
            Log4jXmlImporter.import(c, LOG4J).unwrap()
        });
        assert_eq!(Log4jXmlImporter.id(), "log4j-xml");
        assert_eq!((out.records.len(), out.skipped), (2, 0));
        let a = &out.records[0];
        assert_eq!((a.line, a.ts, a.level), (2, Some(1_792_238_400_123), Level::Warning));
        assert_eq!(a.message, "Waiting for <payment> confirmation");
        for (k, v) in [
            ("Logger", "Billing.Invoices"),
            ("Thread", "7"),
            ("NDC", "order-88"),
            ("Class", "Billing.Writer"),
            ("Method", "Flush"),
            ("File", "Writer.cs"),
            ("MachineName", "BILL01"),
            ("Application", "Billing.exe"),
            ("tenant", "contoso"),
        ] {
            assert_eq!(a.fields[k], v, "{k}");
        }
        assert_eq!(a.fields["Line"], 42);
        let b = &out.records[1];
        assert_eq!((b.line, b.level), (12, Level::Error));
        assert_eq!(b.message, "Post failed & retried \u{263A}");
        assert_eq!(
            b.exception.as_deref(),
            Some("System.ServiceModel.CommunicationException: closed\n   at Billing.Erp.ErpClient.Post()")
        );
    }

    #[test]
    fn parses_nlog_variant() {
        let out = with_ctx(|c| {
            assert_eq!(Log4jXmlImporter.sniff(c, NLOG), 95);
            Log4jXmlImporter.import(c, NLOG).unwrap()
        });
        assert_eq!(out.records.len(), 2);
        let a = &out.records[0];
        assert_eq!((a.message.as_str(), a.level), ("tick", Level::Debug));
        assert_eq!(a.fields["SequenceNumber"], 17);
        assert_eq!(a.fields["Assembly"], "Scheduler, Version=1.0.0.0");
        assert_eq!(a.fields["MachineName"], "SCHED01");
        assert!(!a.fields.keys().any(|k| k.contains("xmlns") || k == "log4j" || k == "nlog"));
        let b = &out.records[1];
        assert_eq!((b.level, b.fields["SequenceNumber"].as_str(), b.line), (Level::Fatal, Some("x"), 2));
    }

    #[test]
    fn parses_log4net_layout_inside_a_root() {
        let out = with_ctx(|c| {
            assert_eq!(Log4jXmlImporter.sniff(c, LOG4NET), 95);
            Log4jXmlImporter.import(c, LOG4NET).unwrap()
        });
        assert_eq!((out.records.len(), out.skipped), (1, 0));
        let r = &out.records[0];
        assert_eq!(r.ts, Some(ms("2026-10-14T10:00:00.123Z")));
        assert_eq!(r.level, Level::Information);
        assert_eq!(
            (r.fields["Domain"].as_str(), r.fields["UserName"].as_str()),
            (Some("Gateway.exe"), Some("CORP\\svc"))
        );
        assert_eq!(r.fields["MachineName"], "GW01");
        assert_eq!(r.fields["custom"], "extra");
        assert!(!r.fields.contains_key("Identity"));
        assert_eq!(r.exception.as_deref(), Some("System.Exception: x"));
    }

    #[test]
    fn tolerates_truncated_and_malformed_events() {
        let text = concat!(
            "junk before\n",
            r#"<log4j:event logger="a" level="INFO" timestamp="1"><log4j:message>cut off"#,
            "\n",
            r#"<log4j:event logger="b" level="INFO" timestamp="2"><log4j:message>ok</log4j:message></log4j:event>"#,
            "\n",
            r#"<log4j:event logger="c" level="INFO" timestamp="3" <<broken"#,
            "\n",
            r#"<log4j:event logger="d" level="INFO" timestamp="2026-10-14 10:00:00"/>"#,
            "\n",
            r#"<log4j:event logger="e" level="bogus" timestamp="nonsense"><log4j:message>tail"#,
        );
        let out = with_ctx(|c| Log4jXmlImporter.import(c, text).unwrap());
        let loggers: Vec<&str> = out.records.iter().map(|r| r.fields["Logger"].as_str().unwrap()).collect();
        assert_eq!(loggers, vec!["a", "b", "d", "e"]);
        assert_eq!(out.skipped, 1);
        assert_eq!(out.records[0].message, "cut off");
        assert_eq!(out.records[2].ts, Some(ms("2026-10-14T10:00:00Z")));
        assert_eq!(
            (out.records[3].level, out.records[3].ts, out.records[3].line),
            (Level::Information, None, 6)
        );
        // Stray end tag closes an event; deep nesting and oversize events are rejected.
        let stray = r#"<event logger="x" level="info"></eventSet><event logger="y" level="info"/>"#;
        assert_eq!(with_ctx(|c| Log4jXmlImporter.import(c, stray).unwrap()).records.len(), 2);
        let deep = format!(r#"<event logger="x">{}</event>"#, "<a>".repeat(MAX_DEPTH + 2));
        assert_eq!(with_ctx(|c| Log4jXmlImporter.import(c, &deep).unwrap()).skipped, 1);
        let big = format!(
            r#"<event logger="x"><message>{}<b/></message></event>"#,
            "x".repeat(MAX_LINE_LEN + 1)
        );
        let out = with_ctx(|c| Log4jXmlImporter.import(c, &big).unwrap());
        assert!(out.records.is_empty());
        let tail = with_ctx(|c| {
            Log4jXmlImporter
                .import_tail(c, "", r#"<event logger="t" level="info"/>"#)
                .unwrap()
                .unwrap()
        });
        assert_eq!(tail.records.len(), 1);
    }

    #[test]
    fn caps_fields_and_maps_levels() {
        let props: String = (0..MAX_FIELDS + 10).map(|i| format!(r#"<data name="p{i}" value="v"/>"#)).collect();
        let text = format!(r#"<event logger="x" level="finer" a="1">{props}<x>y</x></event>"#);
        let out = with_ctx(|c| Log4jXmlImporter.import(c, &text).unwrap());
        assert_eq!(out.records[0].fields.len(), MAX_FIELDS);
        assert_eq!(out.records[0].level, Level::Verbose);
        assert_eq!(parse_level("OFF"), Level::Fatal);
        assert_eq!(parse_level("ALERT"), Level::Fatal);
    }

    #[test]
    fn sniff_negatives() {
        with_ctx(|c| {
            for head in [
                "",
                "<?xml version=\"1.0\"?><MsInfo><Category name=\"x\"/></MsInfo>",
                "<Event xmlns=\"http://schemas.microsoft.com/win/2004/08/events/event\"><System/></Event>",
                "<log4j:eventSet><log4j:event thread=\"1\"></log4j:event>",
                "{\"@t\":\"2026-10-14T10:00:00Z\"}",
                "<events><event name=\"logger\" level=\"x\"/></events>",
            ] {
                assert_eq!(Log4jXmlImporter.sniff(c, head), 0, "{head}");
            }
            let late = format!("{}<log4j:event logger=\"x\" level=\"INFO\">", "é".repeat(9000));
            assert_eq!(Log4jXmlImporter.sniff(c, &late), 0);
            assert_eq!(Log4jXmlImporter.sniff(c, "<event\nlogger=\"x\"\ttimestamp=\"1\">"), 95);
        });
    }

    #[test]
    fn garbage_never_panics() {
        with_ctx(|c| {
            for g in garbage(
                &[
                    "<log4j:event logger=\"a\" level=\"INFO\" timestamp=\"1\">",
                    "</log4j:event>",
                    "<log4j:message>",
                    "</log4j:message>",
                    "<![CDATA[",
                    "]]>",
                    "&amp;",
                    "&#xZZ;",
                    "<log4j:data name=\"a\" value=\"b\"/>",
                    "<",
                    ">",
                    "\"",
                    "é",
                    "\n",
                    "<!--",
                ],
                300,
            ) {
                let _ = Log4jXmlImporter.sniff(c, &g);
                let _ = Log4jXmlImporter.import(c, &g);
            }
        });
    }
}
