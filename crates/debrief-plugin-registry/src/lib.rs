//! Debrief plugin that imports Windows registry exports (`.reg`, as written by
//! regedit or `reg export`), so configuration can be inspected and filtered
//! next to the logs of the machine it came from.
//!
//! Every key and value becomes one record (no timestamp) with the fields
//! `Hive`, `Key`, `ValueName`, `ValueType` and `Data`.
//!
//! The same parser ([`parse_reg`]) also feeds the configuration diff: the
//! plugin contributes [`config::RegConfigParser`] to `config.parsers`.
//!
//! It runs in its own process (`debrief-plugins --plugin net.decryptology.registry`, ADR 0017).

use anyhow::Result;
use debrief_plugin_api::importer::{ImportCtx, Imported, TextImporter};
use debrief_plugin_api::model::Record;
use debrief_plugin_api::{Contributions, Plugin};
use serde_json::Value;

pub mod config;

pub struct RegistryPlugin;

impl Plugin for RegistryPlugin {
    fn id(&self) -> &'static str {
        "net.decryptology.registry"
    }

    fn name(&self) -> &'static str {
        "Windows registry exports"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_importer(RegistryImporter).add_config_parser(config::RegConfigParser);
        Ok(())
    }
}

pub struct RegistryImporter;

impl TextImporter for RegistryImporter {
    fn id(&self) -> &'static str {
        "registry"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        let first = head.lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or("");
        let first = first.trim_start_matches('\u{feff}');
        if first.starts_with("Windows Registry Editor Version") || first == "REGEDIT4" {
            100
        } else {
            0
        }
    }

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let parsed = parse_reg(content);
        let mut out = Imported {
            skipped: parsed.skipped,
            ..Imported::default()
        };
        for entry in parsed.entries {
            match entry {
                RegEntry::Key { line, hive, path, deleted } => {
                    let mut r = record(line, &hive, &path);
                    r.message = if path.is_empty() { hive } else { format!("{hive}\\{path}") };
                    r.fields
                        .insert("ValueType".into(), Value::from(if deleted { "DELETE" } else { "KEY" }));
                    out.records.push(r);
                }
                RegEntry::Value {
                    line,
                    hive,
                    path,
                    name,
                    kind,
                    data,
                } => {
                    let value = data.to_json();
                    let mut r = record(line, &hive, &path);
                    r.message = format!("{path}\\{name} = {}", display(&value));
                    r.fields.insert("ValueName".into(), Value::from(name));
                    r.fields.insert("ValueType".into(), Value::from(kind));
                    r.fields.insert("Data".into(), value);
                    out.records.push(r);
                }
            }
        }
        Ok(out)
    }
}

/// One item of a `.reg` file, in file order.
#[derive(Debug, Clone, PartialEq)]
pub enum RegEntry {
    /// `[HIVE\path]`, or `[-HIVE\path]` when `deleted`.
    Key {
        line: u32,
        hive: String,
        path: String,
        deleted: bool,
    },
    /// `"name"=data` (or `@=data`, named `(Default)`) under the current key.
    Value {
        line: u32,
        hive: String,
        path: String,
        name: String,
        /// `REG_SZ`, `REG_DWORD`, ..., or `DELETE` for `"name"=-`.
        kind: &'static str,
        data: RegData,
    },
}

/// Decoded value data.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RegData {
    Text(String),
    Number(u64),
    Multi(Vec<String>),
    Bytes(Vec<u8>),
    /// `"name"=-`: the value is deleted.
    Delete,
}

impl RegData {
    /// The JSON form records carry (`Data`): numbers, strings, string lists,
    /// bytes as space-separated hex, null for a deletion.
    pub fn to_json(&self) -> Value {
        match self {
            RegData::Text(s) => Value::from(s.as_str()),
            RegData::Number(n) => Value::from(*n),
            RegData::Multi(items) => Value::Array(items.iter().map(|s| Value::from(s.as_str())).collect()),
            RegData::Bytes(b) => Value::from(b.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")),
            RegData::Delete => Value::Null,
        }
    }
}

/// A parsed `.reg` file: its entries and how many lines could not be read.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ParsedReg {
    pub entries: Vec<RegEntry>,
    pub skipped: usize,
}

/// Parses a registry export (the one parser behind the importer and the
/// configuration-diff format). Values under a deleted key, values without a
/// key and undecodable lines are skipped and counted.
pub fn parse_reg(content: &str) -> ParsedReg {
    let mut out = ParsedReg::default();
    let mut key: Option<(String, String)> = None;
    for (line_no, line) in logical_lines(content) {
        let line = line.trim().trim_start_matches('\u{feff}');
        if line.is_empty() || line.starts_with(';') || line.starts_with("Windows Registry Editor") || line == "REGEDIT4" {
            continue;
        }
        if let Some(path) = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) {
            let (deleted, path) = match path.strip_prefix('-') {
                Some(p) => (true, p),
                None => (false, path),
            };
            let (hive, rest) = path.split_once('\\').unwrap_or((path, ""));
            out.entries.push(RegEntry::Key {
                line: line_no,
                hive: hive.to_string(),
                path: rest.to_string(),
                deleted,
            });
            key = (!deleted).then(|| (hive.to_string(), rest.to_string()));
            continue;
        }
        let (Some((hive, path)), Some((name, data))) = (&key, parse_value_line(line)) else {
            out.skipped += 1;
            continue;
        };
        let Some((kind, data)) = decode(data) else {
            out.skipped += 1;
            continue;
        };
        out.entries.push(RegEntry::Value {
            line: line_no,
            hive: hive.clone(),
            path: path.clone(),
            name,
            kind,
            data,
        });
    }
    out
}

fn record(line: u32, hive: &str, key: &str) -> Record {
    let mut r = Record::new(line);
    r.fields.insert("Hive".into(), Value::from(hive));
    r.fields.insert("Key".into(), Value::from(key));
    r
}

fn display(v: &Value) -> String {
    match v {
        Value::String(s) => s.clone(),
        Value::Array(items) => items.iter().map(display).collect::<Vec<_>>().join(", "),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// Joins lines continued with a trailing backslash (long hex values),
/// keeping the number of the first physical line.
fn logical_lines(content: &str) -> Vec<(u32, String)> {
    let mut out: Vec<(u32, String)> = Vec::new();
    let mut pending: Option<(u32, String)> = None;
    for (i, raw) in content.lines().enumerate() {
        let raw = raw.trim_end_matches('\r');
        let (start, mut text) = match pending.take() {
            Some((n, mut acc)) => {
                acc.push_str(raw.trim_start());
                (n, acc)
            }
            None => (i as u32 + 1, raw.to_string()),
        };
        if text.ends_with('\\') && !text.trim_start().starts_with('[') {
            text.pop();
            pending = Some((start, text));
        } else {
            out.push((start, text));
        }
    }
    out.extend(pending);
    out
}

/// `"Name"=data` or `@=data` -> (name, data).
fn parse_value_line(line: &str) -> Option<(String, &str)> {
    if let Some(rest) = line.strip_prefix('@') {
        return Some(("(Default)".into(), rest.trim_start().strip_prefix('=')?));
    }
    let (name, rest) = parse_quoted(line)?;
    Some((name, rest.trim_start().strip_prefix('=')?))
}

/// Parses a leading `"..."` with `\\` and `\"` escapes; returns it and the rest.
fn parse_quoted(s: &str) -> Option<(String, &str)> {
    let mut chars = s.strip_prefix('"')?.char_indices();
    let mut out = String::new();
    while let Some((i, c)) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?.1),
            '"' => return Some((out, &s[i + 2..])),
            c => out.push(c),
        }
    }
    None
}

/// Decodes a value's data into its registry type and value.
fn decode(data: &str) -> Option<(&'static str, RegData)> {
    let data = data.trim();
    if data == "-" {
        return Some(("DELETE", RegData::Delete));
    }
    if data.starts_with('"') {
        let (s, rest) = parse_quoted(data)?;
        return rest.trim().is_empty().then_some(("REG_SZ", RegData::Text(s)));
    }
    if let Some(hex) = data.strip_prefix("dword:") {
        return Some(("REG_DWORD", RegData::Number(u32::from_str_radix(hex.trim(), 16).ok()?.into())));
    }
    let (kind, hex) = if let Some(h) = data.strip_prefix("hex:") {
        ("REG_BINARY", h)
    } else {
        let rest = data.strip_prefix("hex(")?;
        let (code, h) = rest.split_once("):")?;
        let kind = match code.to_ascii_lowercase().as_str() {
            "0" => "REG_NONE",
            "2" => "REG_EXPAND_SZ",
            "7" => "REG_MULTI_SZ",
            "b" => "REG_QWORD",
            "3" => "REG_BINARY",
            "4" => "REG_DWORD",
            _ => "REG_UNKNOWN",
        };
        (kind, h)
    };
    let bytes: Vec<u8> = hex
        .split(',')
        .map(str::trim)
        .filter(|b| !b.is_empty())
        .map(|b| u8::from_str_radix(b, 16).ok())
        .collect::<Option<_>>()?;
    let value = match kind {
        "REG_EXPAND_SZ" => RegData::Text(utf16(&bytes).trim_end_matches('\0').to_string()),
        "REG_MULTI_SZ" => RegData::Multi(utf16(&bytes).split('\0').filter(|s| !s.is_empty()).map(String::from).collect()),
        "REG_QWORD" if bytes.len() == 8 => RegData::Number(u64::from_le_bytes(bytes.try_into().ok()?)),
        "REG_DWORD" if bytes.len() == 4 => RegData::Number(u32::from_le_bytes(bytes.try_into().ok()?).into()),
        _ => RegData::Bytes(bytes),
    };
    Some((kind, value))
}

fn utf16(bytes: &[u8]) -> String {
    let units: Vec<u16> = bytes.as_chunks::<2>().0.iter().map(|c| u16::from_le_bytes(*c)).collect();
    String::from_utf16_lossy(&units)
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_plugin_api::time::TimestampParser;
    use serde_json::json;
    use std::path::Path;

    fn import(content: &str) -> Imported {
        let ts = TimestampParser::default();
        let ctx = ImportCtx::new(Path::new("x.reg"), &ts);
        assert_eq!(RegistryImporter.sniff(&ctx, content), 100);
        RegistryImporter.import(&ctx, content).unwrap()
    }

    #[test]
    fn decodes_every_value_type() {
        let out = import(concat!(
            "\u{feff}Windows Registry Editor Version 5.00\r\n\r\n",
            "; comment\r\n",
            "[HKEY_LOCAL_MACHINE\\SOFTWARE\\Contoso\\Order \"Svc\"]\r\n",
            "@=\"default value\"\r\n",
            "\"Path\"=\"C:\\\\Program Files\\\\Contoso\"\r\n",
            "\"Timeout\"=dword:00001388\r\n",
            "\"Big\"=hex(b):00,10,00,00,00,00,00,00\r\n",
            "\"Expand\"=hex(2):25,00,54,00,4d,00,50,00,25,00,00,00\r\n",
            "\"Multi\"=hex(7):61,00,00,00,62,00,00,00,00,00\r\n",
            "\"Blob\"=hex:de,ad,\\\r\n  be,ef\r\n",
            "\"None\"=hex(0):\r\n",
            "\"Dword4\"=hex(4):0a,00,00,00\r\n",
            "\"Odd\"=hex(9):01\r\n",
            "\"Gone\"=-\r\n",
            "garbage line\r\n",
            "\"Bad\"=dword:zz\r\n",
            "[-HKEY_CURRENT_USER\\Software\\Old]\r\n",
            "\"Orphan\"=\"no key\"\r\n",
        ));
        let by_name = |n: &str| out.records.iter().find(|r| r.fields.get("ValueName") == Some(&json!(n))).unwrap();
        let key = &out.records[0];
        assert_eq!(
            (
                key.fields["Hive"].clone(),
                key.fields["Key"].clone(),
                key.fields["ValueType"].clone()
            ),
            (json!("HKEY_LOCAL_MACHINE"), json!("SOFTWARE\\Contoso\\Order \"Svc\""), json!("KEY"))
        );
        assert_eq!(by_name("(Default)").fields["Data"], "default value");
        assert_eq!(by_name("Path").fields["Data"], "C:\\Program Files\\Contoso");
        assert_eq!(by_name("Timeout").fields["Data"], 5000);
        assert_eq!(by_name("Timeout").message, "SOFTWARE\\Contoso\\Order \"Svc\"\\Timeout = 5000");
        assert_eq!(by_name("Big").fields["Data"], 4096);
        assert_eq!(by_name("Expand").fields["Data"], "%TMP%");
        assert_eq!(by_name("Multi").fields["Data"], json!(["a", "b"]));
        assert_eq!(by_name("Multi").message.rsplit(" = ").next(), Some("a, b"));
        assert_eq!(by_name("Blob").fields["Data"], "de ad be ef");
        assert_eq!(by_name("Blob").line, 11);
        assert_eq!(by_name("None").fields["ValueType"], "REG_NONE");
        assert_eq!(by_name("Dword4").fields["Data"], 10);
        assert_eq!(by_name("Odd").fields["ValueType"], "REG_UNKNOWN");
        assert_eq!(by_name("Gone").fields["ValueType"], "DELETE");
        assert_eq!(by_name("Gone").fields["Data"], Value::Null);
        assert!(out
            .records
            .iter()
            .any(|r| r.fields["ValueType"] == "DELETE" && r.message == "HKEY_CURRENT_USER\\Software\\Old"));
        // garbage, bad dword, value after a deleted key
        assert_eq!(out.skipped, 3);
    }

    #[test]
    fn regedit4_and_edge_cases() {
        let out = import("REGEDIT4\n\n[HKEY_USERS]\n\"A\"=\"x\" trailing\n\"Unterminated=\"x\n@\n\"Bad\"=hex:zz\n\"Cont\"=hex:01,\\\n");
        assert_eq!(out.records.len(), 2);
        assert_eq!(out.records[0].fields["Key"], "");
        assert_eq!(out.records[1].fields["Data"], "01");
        assert_eq!(out.skipped, 4);
        let ts = TimestampParser::default();
        let ctx = ImportCtx::new(Path::new("x"), &ts);
        assert_eq!(RegistryImporter.sniff(&ctx, "2026-10-14 log line"), 0);
        assert_eq!(display(&Value::Null), "");
    }

    #[test]
    fn the_plugin_contributes_the_importer_and_the_config_parser() {
        let mut c = Contributions::default();
        RegistryPlugin.register(&mut c).unwrap();
        let m = c.manifest(&RegistryPlugin);
        m.check().unwrap();
        assert_eq!(m.id.as_ref(), "net.decryptology.registry");
        assert_eq!(m.contributions(), ["importer:registry", "config-parser:registry"]);
        assert_eq!(RegistryPlugin.name(), "Windows registry exports");
        assert_eq!(RegistryPlugin.version(), env!("CARGO_PKG_VERSION"));
    }
}
