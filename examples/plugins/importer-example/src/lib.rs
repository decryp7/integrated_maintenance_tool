//! An example importer plugin: reads `key=value` log lines such as
//!
//! ```text
//! 2026-10-21T10:00:00Z level=warn msg="disk almost full" host=web01 used=93
//! ```
//!
//! The first token is the time; `level` sets the level, `msg` the message, every other pair becomes a field
//! (numbers as numbers). Lines that do not start with a time are counted as skipped.

use anyhow::Result;
use imt_plugin_api::prelude::*;

/// The importer: one record per line.
pub struct KvLines;

/// Splits `rest` into `key=value` pairs; values may be double-quoted.
fn pairs(rest: &str) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut chars = rest.trim().chars().peekable();
    while chars.peek().is_some() {
        let key: String = chars.by_ref().take_while(|c| *c != '=').collect();
        let value: String = if chars.peek() == Some(&'"') {
            chars.next();
            let v = chars.by_ref().take_while(|c| *c != '"').collect();
            // The space after the closing quote.
            chars.next();
            v
        } else {
            chars.by_ref().take_while(|c| !c.is_whitespace()).collect()
        };
        if !key.trim().is_empty() {
            out.push((key.trim().to_string(), value));
        }
    }
    out
}

fn level(text: &str) -> Level {
    match text.to_ascii_lowercase().as_str() {
        "trace" | "verbose" => Level::Verbose,
        "debug" => Level::Debug,
        "warn" | "warning" => Level::Warning,
        "error" | "err" => Level::Error,
        "fatal" | "critical" => Level::Fatal,
        _ => Level::Information,
    }
}

impl TextImporter for KvLines {
    fn id(&self) -> &str {
        "kv-lines"
    }

    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8 {
        let lines: Vec<&str> = head.lines().filter(|l| !l.trim().is_empty()).take(5).collect();
        let good = lines
            .iter()
            .filter(|l| {
                let (time, rest) = l.split_once(' ').unwrap_or(("", ""));
                ctx.timestamps.parse(time).is_some() && rest.contains('=')
            })
            .count();
        if lines.is_empty() {
            0
        } else {
            (good * 90 / lines.len()) as u8
        }
    }

    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let mut out = Imported::default();
        for (i, line) in content.lines().enumerate() {
            // A long import looks now and then whether IMT gave up on it.
            if i % 4096 == 0 && ctx.cancelled() {
                anyhow::bail!("cancelled");
            }
            let (time, rest) = line.split_once(' ').unwrap_or((line, ""));
            let Some(ts) = ctx.timestamps.parse(time) else {
                out.skipped += 1;
                continue;
            };
            let mut r = Record::new(i as u32 + 1);
            r.ts = Some(ts);
            for (k, v) in pairs(rest) {
                match k.as_str() {
                    "level" => r.level = level(&v),
                    "msg" => r.message = v,
                    _ => {
                        let value = v
                            .parse::<i64>()
                            .map(serde_json::Value::from)
                            .unwrap_or(serde_json::Value::String(v));
                        r.fields.insert(k, value);
                    }
                }
            }
            out.records.push(r);
        }
        Ok(out)
    }

    fn multiline(&self) -> bool {
        false
    }

    fn identity_fields(&self) -> Vec<String> {
        vec!["host".into()]
    }
}

/// The plugin.
pub struct KvLinesPlugin;

impl Plugin for KvLinesPlugin {
    fn id(&self) -> &str {
        "net.decryptology.example.kv-lines"
    }

    fn name(&self) -> &str {
        "Key-value lines (example)"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_importer(KvLines);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pairs_handle_quotes_and_bare_values() {
        assert_eq!(
            pairs(r#"level=warn msg="disk almost full" host=web01"#),
            [
                ("level".to_string(), "warn".to_string()),
                ("msg".into(), "disk almost full".into()),
                ("host".into(), "web01".into())
            ]
        );
        assert!(pairs("").is_empty());
        assert_eq!(level("WARN"), Level::Warning);
        assert_eq!(level("other"), Level::Information);
    }
}
