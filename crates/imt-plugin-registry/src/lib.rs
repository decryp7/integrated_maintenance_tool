//! IMT plugin that imports Windows registry exports (`.reg`, as written by
//! regedit or `reg export`), so configuration can be inspected and filtered
//! next to the logs of the machine it came from.
//!
//! Every key and value becomes one record (no timestamp) with the fields
//! `Hive`, `Key`, `ValueName`, `ValueType` and `Data`.
//!
//! Register it with:
//! ```
//! let mut registry = imt_core::PluginRegistry::new();
//! registry.install(&imt_plugin_registry::RegistryPlugin).unwrap();
//! ```

use anyhow::Result;
use imt_core::importers::{ImportCtx, Imported, Importer};
use imt_core::model::Record;
use imt_core::{Plugin, PluginRegistry};
use serde_json::Value;

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

    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        registry.add_importer(Box::new(RegistryImporter));
        Ok(())
    }
}

pub struct RegistryImporter;

impl Importer for RegistryImporter {
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
        let mut out = Imported::default();
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
                let mut r = record(line_no, hive, rest);
                r.message = path.to_string();
                r.fields
                    .insert("ValueType".into(), Value::from(if deleted { "DELETE" } else { "KEY" }));
                out.records.push(r);
                key = (!deleted).then(|| (hive.to_string(), rest.to_string()));
                continue;
            }
            let (Some((hive, path)), Some((name, data))) = (&key, parse_value_line(line)) else {
                out.skipped += 1;
                continue;
            };
            let Some((kind, value)) = decode(data) else {
                out.skipped += 1;
                continue;
            };
            let mut r = record(line_no, hive, path);
            r.message = format!("{path}\\{name} = {}", display(&value));
            r.fields.insert("ValueName".into(), Value::from(name));
            r.fields.insert("ValueType".into(), Value::from(kind));
            r.fields.insert("Data".into(), value);
            out.records.push(r);
        }
        Ok(out)
    }
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

/// Decodes a value's data into its registry type and a JSON value.
fn decode(data: &str) -> Option<(&'static str, Value)> {
    let data = data.trim();
    if data == "-" {
        return Some(("DELETE", Value::Null));
    }
    if data.starts_with('"') {
        let (s, rest) = parse_quoted(data)?;
        return rest.trim().is_empty().then(|| ("REG_SZ", Value::from(s)));
    }
    if let Some(hex) = data.strip_prefix("dword:") {
        return Some(("REG_DWORD", Value::from(u32::from_str_radix(hex.trim(), 16).ok()?)));
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
        "REG_EXPAND_SZ" => Value::from(utf16(&bytes).trim_end_matches('\0').to_string()),
        "REG_MULTI_SZ" => Value::Array(utf16(&bytes).split('\0').filter(|s| !s.is_empty()).map(Value::from).collect()),
        "REG_QWORD" if bytes.len() == 8 => Value::from(u64::from_le_bytes(bytes.try_into().ok()?)),
        "REG_DWORD" if bytes.len() == 4 => Value::from(u32::from_le_bytes(bytes.try_into().ok()?)),
        _ => Value::from(bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(" ")),
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
    use imt_core::profile::{CompiledProfile, Profile};
    use serde_json::json;
    use std::path::Path;

    fn import(content: &str) -> Imported {
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x.reg"),
            file_date: None,
        };
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
        let p = CompiledProfile::compile(Profile::default_profile()).unwrap();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("x"),
            file_date: None,
        };
        assert_eq!(RegistryImporter.sniff(&ctx, "2026-10-14 log line"), 0);
        assert_eq!(display(&Value::Null), "");
    }

    #[test]
    fn plugin_registers_the_importer() {
        let mut registry = PluginRegistry::new();
        registry.install(&RegistryPlugin).unwrap();
        let info = &registry.plugins()[0];
        assert_eq!(info.id, "net.decryptology.registry");
        assert_eq!(info.contributes, vec!["importer:registry".to_string()]);
        assert_eq!(RegistryPlugin.name(), "Windows registry exports");
        assert_eq!(RegistryPlugin.version(), env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn imports_the_utf16_sample_through_a_workspace() {
        let mut registry = PluginRegistry::new();
        registry.install(&RegistryPlugin).unwrap();
        let mut ws = imt_core::Workspace::with_registry(Profile::default_profile(), registry).unwrap();
        let sample = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../samples/unsupported/registry-export.reg");
        ws.open(&[sample]).unwrap();
        let s = ws.summary();
        assert_eq!(s.sources.len(), 1);
        assert_eq!(s.sources[0].importer, "registry");
        assert!(s.plugins.iter().any(|p| p.id == "net.decryptology.registry"));
        assert!(ws
            .records()
            .iter()
            .any(|r| r.fields.get("ValueName") == Some(&json!("PaymentTimeoutMs")) && r.fields["Data"] == 5000));
    }
}
