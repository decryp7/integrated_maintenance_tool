//! Performance Monitor (PDH) CSV/TSV logs, as written by PerfMon, `logman` and `relog`.
//!
//! The file is a wide table: the first header cell is `(PDH-CSV 4.0) (zone)(bias)`,
//! the others are counter paths `\\MACHINE\Object(Instance#n)\Counter`. The importer
//! turns it into the long format IMT can chart and filter: one record per row and
//! counter with `MachineName`, `Object`, `Instance`, `InstanceIndex`, `Counter`,
//! `CounterPath` and `Value`. Rows of `Process` counters also get `ProcessName` and,
//! when the capture includes `\Process(x)\ID Process`, `ProcessId`.

use anyhow::{bail, Result};
use imt_plugin_api::importer::{ImportCtx, Imported, TextImporter};
use imt_plugin_api::model::Record;
use imt_plugin_api::time::TimestampParser;
use serde_json::Value;
use std::collections::HashMap;

const SIGNATURES: [&str; 2] = ["(PDH-CSV 4.0)", "(PDH-TSV 4.0)"];

pub struct PerfmonCsvImporter;

impl TextImporter for PerfmonCsvImporter {
    fn id(&self) -> &'static str {
        "perfmon-csv"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        if signature(head).is_some() {
            99
        } else {
            0
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        parse_perfmon(content, ctx.timestamps)
    }
}

/// The delimiter implied by the signature, if the text starts with one.
fn signature(text: &str) -> Option<u8> {
    let t = text.trim_start_matches('\u{feff}').trim_start().trim_start_matches('"');
    if t.starts_with(SIGNATURES[0]) {
        Some(b',')
    } else if t.starts_with(SIGNATURES[1]) {
        Some(b'\t')
    } else {
        None
    }
}

/// A counter path split into its parts.
#[derive(Debug, Clone, PartialEq)]
pub struct CounterPath {
    pub machine: String,
    pub object: String,
    pub instance: Option<String>,
    pub instance_index: Option<u32>,
    pub counter: String,
}

/// Splits `\\MACHINE\Object(Instance#n)\Counter`. The machine and instance are optional.
pub fn parse_counter_path(path: &str) -> CounterPath {
    let (machine, rest) = match path.strip_prefix("\\\\") {
        Some(r) => r.split_once('\\').unwrap_or((r, "")),
        None => ("", path.strip_prefix('\\').unwrap_or(path)),
    };
    let (object, instance, counter) = match (rest.find('('), rest.find('\\')) {
        (Some(open), slash) if slash.is_none_or(|s| open < s) => match rest.rfind(")\\") {
            Some(close) if close > open => (&rest[..open], Some(&rest[open + 1..close]), &rest[close + 2..]),
            _ => (rest, None, ""),
        },
        _ => match rest.split_once('\\') {
            Some((o, c)) => (o, None, c),
            None => (rest, None, ""),
        },
    };
    let (instance, instance_index) = match instance {
        Some(i) => match i.rsplit_once('#').and_then(|(n, idx)| Some((n, idx.parse::<u32>().ok()?))) {
            Some((n, idx)) => (Some(n.to_string()), Some(idx)),
            None => (Some(i.to_string()), None),
        },
        None => (None, None),
    };
    CounterPath {
        machine: machine.to_string(),
        object: object.to_string(),
        instance,
        instance_index,
        counter: counter.to_string(),
    }
}

/// Minutes to add to local time to get UTC, from the trailing `(-120)` of the first header cell.
fn bias_minutes(first_cell: &str) -> Option<i64> {
    let t = first_cell.trim().strip_suffix(')')?;
    t[t.rfind('(')? + 1..].trim().parse().ok()
}

struct Column {
    path: CounterPath,
    raw: String,
    /// Instance with its `#n` suffix: what distinguishes two processes of the same name.
    full_instance: String,
}

fn number(cell: &str) -> Option<Value> {
    let v: f64 = cell.trim().parse().ok().filter(|v: &f64| v.is_finite())?;
    if v.fract() == 0.0 && v.abs() < 9e15 {
        Some(Value::from(v as i64))
    } else {
        Some(Value::from(v))
    }
}

/// Parses a PDH-CSV/TSV text. Timestamps are local time of the capture; the bias in the
/// header converts them to UTC, otherwise `fallback` (the profile's timezone) applies.
pub fn parse_perfmon(content: &str, fallback: &TimestampParser) -> Result<Imported> {
    let Some(delim) = signature(content) else {
        bail!("not a Performance Monitor log (the first header cell must start with \"(PDH-CSV 4.0)\")");
    };
    let mut reader = csv::ReaderBuilder::new()
        .delimiter(delim)
        .flexible(true)
        .has_headers(true)
        .from_reader(content.trim_start_matches('\u{feff}').as_bytes());
    let headers = reader.headers()?.clone();
    let bias = bias_minutes(&headers[0]);
    let parser = match bias {
        Some(_) => TimestampParser::default(),
        None => fallback.clone(),
    }
    .with_day_first(Some(false));
    let columns: Vec<Column> = headers
        .iter()
        .skip(1)
        .map(|h| {
            let path = parse_counter_path(h.trim());
            let full_instance = match (&path.instance, path.instance_index) {
                (Some(i), Some(n)) => format!("{i}#{n}"),
                (Some(i), None) => i.clone(),
                _ => String::new(),
            };
            Column {
                path,
                raw: h.trim().to_string(),
                full_instance,
            }
        })
        .collect();
    let is_process = |c: &Column| c.path.object.eq_ignore_ascii_case("Process");
    let pid_column: HashMap<(&str, &str), usize> = columns
        .iter()
        .enumerate()
        .filter(|(_, c)| is_process(c) && c.path.counter.eq_ignore_ascii_case("ID Process"))
        .map(|(i, c)| ((c.path.machine.as_str(), c.full_instance.as_str()), i))
        .collect();

    let mut out = Imported::default();
    for row in reader.records() {
        let Ok(row) = row else {
            out.skipped += 1;
            continue;
        };
        let line = row.position().map_or(0, |p| p.line() as u32);
        let ts = row.get(0).and_then(|c| parser.parse(c)).map(|ms| ms + bias.unwrap_or(0) * 60_000);
        let Some(ts) = ts else {
            out.skipped += 1;
            continue;
        };
        for (i, col) in columns.iter().enumerate() {
            let cell = row.get(i + 1).unwrap_or("").trim();
            let Some(value) = number(cell) else { continue };
            let p = &col.path;
            let mut rec = Record::new(line);
            rec.ts = Some(ts);
            rec.message = match &p.instance {
                Some(inst) => format!("{}({})\\{} = {}", p.object, inst, p.counter, cell),
                None => format!("{}\\{} = {}", p.object, p.counter, cell),
            };
            let f = &mut rec.fields;
            f.insert("MachineName".into(), p.machine.clone().into());
            f.insert("Object".into(), p.object.clone().into());
            if let Some(inst) = &p.instance {
                f.insert("Instance".into(), inst.clone().into());
            }
            if let Some(n) = p.instance_index {
                f.insert("InstanceIndex".into(), n.into());
            }
            f.insert("Counter".into(), p.counter.clone().into());
            f.insert("CounterPath".into(), col.raw.clone().into());
            f.insert("Value".into(), value);
            if is_process(col) {
                if let Some(inst) = &p.instance {
                    f.insert("ProcessName".into(), inst.clone().into());
                }
                let pid = pid_column
                    .get(&(p.machine.as_str(), col.full_instance.as_str()))
                    .and_then(|&c| number(row.get(c + 1).unwrap_or("")));
                if let Some(pid) = pid {
                    f.insert("ProcessId".into(), pid);
                }
            }
            out.records.push(rec);
        }
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_plugin_api::time::TimestampParser;
    use std::path::Path;

    fn ctx_for<'a>(p: &'a TimestampParser, path: &'a Path) -> ImportCtx<'a> {
        ImportCtx::new(path, p)
    }

    const SAMPLE: &str = "\"(PDH-CSV 4.0) (W. Europe Daylight Time)(-120)\",\"\\\\WEB02\\Process(OrderService#1)\\Private Bytes\",\"\\\\WEB02\\Process(OrderService#1)\\ID Process\",\"\\\\WEB02\\Memory\\Available MBytes\",\"\\\\WEB02\\Processor(_Total)\\% Processor Time\"\r\n\
\"10/14/2026 09:00:00.250\",\"600000000\",\"4120\",\"4096.5\",\" \"\r\n\
\"10/14/2026 09:00:15.250\",\"\",\"4120\",\"4090\",\"12.5\"\r\n\
\"not a date\",\"1\",\"1\",\"1\",\"1\"\r\n";

    #[test]
    fn parses_counter_paths() {
        let p = parse_counter_path("\\\\WEB02\\Process(OrderService#12)\\Private Bytes");
        assert_eq!(
            p,
            CounterPath {
                machine: "WEB02".into(),
                object: "Process".into(),
                instance: Some("OrderService".into()),
                instance_index: Some(12),
                counter: "Private Bytes".into()
            }
        );
        let p = parse_counter_path("\\\\M\\Memory\\Available MBytes");
        assert_eq!(
            (p.object.as_str(), p.instance, p.counter.as_str()),
            ("Memory", None, "Available MBytes")
        );
        let p = parse_counter_path("\\\\M\\Network Interface(Intel[R] (2) #3)\\Bytes Total/sec");
        assert_eq!(p.instance.as_deref(), Some("Intel[R] (2) "));
        assert_eq!((p.instance_index, p.counter.as_str()), (Some(3), "Bytes Total/sec"));
        let p = parse_counter_path("\\LogicalDisk(C:)\\Free Megabytes");
        assert_eq!(
            (p.machine.as_str(), p.instance.as_deref(), p.object.as_str()),
            ("", Some("C:"), "LogicalDisk")
        );
        let p = parse_counter_path("\\\\M\\Process(a#x)\\Thread Count");
        assert_eq!((p.instance.as_deref(), p.instance_index), (Some("a#x"), None));
        // Malformed: no counter part.
        assert_eq!(parse_counter_path("\\\\M\\Weird(oops").counter, "");
        assert_eq!(parse_counter_path("\\\\M").object, "");
        assert_eq!(parse_counter_path("\\Obj").object, "Obj");
        let p = parse_counter_path("\\\\M\\Obj\\Counter (a)");
        assert_eq!((p.object.as_str(), p.instance, p.counter.as_str()), ("Obj", None, "Counter (a)"));
    }

    #[test]
    fn bias_comes_from_the_last_parentheses() {
        assert_eq!(bias_minutes("(PDH-CSV 4.0) (W. Europe Daylight Time)(-120)"), Some(-120));
        assert_eq!(bias_minutes("(PDH-CSV 4.0) (UTC)(0)"), Some(0));
        assert_eq!(bias_minutes("(PDH-CSV 4.0)"), None);
        assert_eq!(bias_minutes("(PDH-CSV 4.0) x"), None);
    }

    #[test]
    fn sniffs_with_priority_over_csv() {
        let profile = TimestampParser::default();
        let ctx = ctx_for(&profile, Path::new("a.csv"));
        assert_eq!(PerfmonCsvImporter.sniff(&ctx, SAMPLE), 99);
        assert_eq!(PerfmonCsvImporter.sniff(&ctx, "\u{feff}\"(PDH-TSV 4.0) (UTC)(0)\"\tx"), 99);
        assert_eq!(PerfmonCsvImporter.sniff(&ctx, "time,value\n1,2"), 0);
        assert_eq!(PerfmonCsvImporter.id(), "perfmon-csv");
    }

    #[test]
    fn tsv_without_bias_uses_the_profile_zone_and_errors_are_clear() {
        let profile = TimestampParser::new(&[], "+02:00", None).unwrap();
        let ctx = ctx_for(&profile, Path::new("a.tsv"));
        let tsv = "\"(PDH-TSV 4.0)\"\t\"\\\\M\\Process(a)\\Thread Count\"\n\"10/14/2026 09:00:00.000\"\t\"7.5\"\n";
        let out = PerfmonCsvImporter.import(&ctx, tsv).unwrap();
        assert_eq!(out.records[0].ts, Some(1_791_968_400_000 - 2 * 3_600_000));
        assert_eq!(out.records[0].fields["Value"].as_f64(), Some(7.5));
        assert!(!out.records[0].fields.contains_key("ProcessId"));
        let err = PerfmonCsvImporter.import(&ctx, "a,b\n1,2").err().unwrap().to_string();
        assert!(err.contains("PDH-CSV 4.0"), "{err}");
        // A ragged row is tolerated; a header only file has no records.
        let ragged =
            "\"(PDH-CSV 4.0) (UTC)(0)\",\"\\\\M\\Memory\\Pages/sec\",\"\\\\M\\Memory\\Cache Bytes\"\n\"10/14/2026 09:00:00.000\",\"1\"\n";
        let out = PerfmonCsvImporter.import(&ctx, ragged).unwrap();
        assert_eq!((out.records.len(), out.records[0].ts), (1, Some(1_791_968_400_000)));
        assert!(PerfmonCsvImporter
            .import(&ctx, "\"(PDH-CSV 4.0) (UTC)(0)\",\"x\"\n")
            .unwrap()
            .records
            .is_empty());
    }

    #[test]
    fn long_format_records() {
        let profile = TimestampParser::default();
        let out = PerfmonCsvImporter.import(&ctx_for(&profile, Path::new("a.csv")), SAMPLE).unwrap();
        // Row 1: 3 values (blank " " skipped); row 2: 3 values (empty skipped); bad date skipped.
        assert_eq!(out.records.len(), 6);
        assert_eq!(out.skipped, 1);
        let r = &out.records[0];
        // 09:00:00.250 local with bias -120 => UTC = local - 2 h.
        assert_eq!(r.ts, Some(1_791_968_400_000 - 2 * 3_600_000 + 250));
        assert_eq!(r.message, "Process(OrderService)\\Private Bytes = 600000000");
        assert_eq!(r.level, imt_plugin_api::model::Level::Information);
        let f = |k: &str| r.fields.get(k).cloned().unwrap();
        assert_eq!(f("MachineName"), "WEB02");
        assert_eq!(f("Object"), "Process");
        assert_eq!(f("Instance"), "OrderService");
        assert_eq!(f("InstanceIndex"), 1);
        assert_eq!(f("ProcessName"), "OrderService");
        assert_eq!(f("ProcessId"), 4120);
        assert_eq!(f("Counter"), "Private Bytes");
        assert_eq!(f("CounterPath"), "\\\\WEB02\\Process(OrderService#1)\\Private Bytes");
        assert_eq!(f("Value"), 600000000);
        assert_eq!(out.records[1].fields["ProcessId"], 4120);
        let mem = &out.records[2];
        assert_eq!(mem.message, "Memory\\Available MBytes = 4096.5");
        assert_eq!(mem.fields["Value"].as_f64(), Some(4096.5));
        assert!(!mem.fields.contains_key("Instance") && !mem.fields.contains_key("ProcessId"));
        assert!(!mem.fields.contains_key("InstanceIndex"));
    }
}
