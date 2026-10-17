//! An example process plugin: an importer for the "toy" log format, one line per event:
//!
//! ```text
//! TOY|2024-05-01T10:00:00Z|WARN|disk almost full|disk=C: free=3%
//! ```
//!
//! (`TOY|<RFC 3339 time>|<level>|<message>|<key=value ...>`). It also contributes a transform that drops
//! `DEBUG` events, a marker kind, and a setting. Build it and check it:
//!
//! ```text
//! cargo build -p imt-plugin-sdk --example toy-plugin
//! cargo run -p imt-plugin-sdk --bin imt-plugin-check -- target/debug/examples/toy-plugin
//! ```

use anyhow::Result;
use imt_core::importers::{ImportCtx, Imported, Importer};
use imt_core::model::{Level, Record};
use imt_core::plugin::markers::PluginMarkerKind;
use imt_core::plugin::{RecordTransform, TransformCtx, Verdict};
use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use imt_core::{Plugin, PluginRegistry};
use serde_json::Value;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

struct ToyImporter;

fn parse_line(line: &str, n: u32) -> Option<Record> {
    let mut parts = line.splitn(5, '|');
    if parts.next()? != "TOY" {
        return None;
    }
    let mut r = Record::new(n);
    r.ts = chrono::DateTime::parse_from_rfc3339(parts.next()?)
        .ok()
        .map(|t| t.timestamp_millis());
    r.level = Level::parse(parts.next()?).unwrap_or(Level::Information);
    r.message = parts.next()?.to_string();
    for kv in parts.next().unwrap_or("").split_whitespace() {
        if let Some((k, v)) = kv.split_once('=') {
            let value = v.parse::<i64>().map(Value::from).unwrap_or_else(|_| Value::from(v));
            r.fields.insert(k.to_string(), value);
        }
    }
    Some(r)
}

impl Importer for ToyImporter {
    fn id(&self) -> &'static str {
        "toy"
    }

    fn sniff(&self, _ctx: &ImportCtx, head: &str) -> u8 {
        if head.lines().take(5).any(|l| l.starts_with("TOY|")) {
            90
        } else {
            0
        }
    }

    fn import(&self, _ctx: &ImportCtx, content: &str) -> Result<Imported> {
        let mut out = Imported::default();
        for (i, line) in content.lines().enumerate() {
            match parse_line(line, i as u32 + 1) {
                Some(r) => out.records.push(r),
                None if line.trim().is_empty() => {}
                None => out.skipped += 1,
            }
        }
        Ok(out)
    }

    /// Lines are independent, so appended text parses on its own.
    fn import_tail(&self, ctx: &ImportCtx, _head: &str, tail: &str) -> Option<Result<Imported>> {
        Some(self.import(ctx, tail))
    }

    fn multiline(&self) -> bool {
        false
    }
}

struct DropDebug {
    enabled: Arc<AtomicBool>,
}

impl RecordTransform for DropDebug {
    fn id(&self) -> &str {
        "toy.drop-debug"
    }

    fn apply(&self, record: &mut Record, ctx: &TransformCtx) -> Verdict {
        if record.message.contains("full") {
            ctx.mark("toy-full");
        }
        if self.enabled.load(Ordering::Relaxed) && record.level <= Level::Debug {
            Verdict::Drop
        } else {
            Verdict::Keep
        }
    }
}

struct Settings {
    enabled: Arc<AtomicBool>,
}

impl Configurable for Settings {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new("toy", "Toy format", SectionOwner::Core).field(SettingField::new(
            "dropDebug",
            "Drop debug events",
            FieldKind::Bool,
            true,
        ))
    }

    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        self.enabled.store(values.bool("dropDebug").unwrap_or(true), Ordering::Relaxed);
        Ok(())
    }
}

struct ToyPlugin {
    enabled: Arc<AtomicBool>,
}

impl Plugin for ToyPlugin {
    fn id(&self) -> &'static str {
        "com.example.toy"
    }

    fn name(&self) -> &'static str {
        "Toy format"
    }

    fn version(&self) -> &'static str {
        "1.0.0"
    }

    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        registry.add_importer(Box::new(ToyImporter));
        registry.add_transform(Arc::new(DropDebug {
            enabled: self.enabled.clone(),
        }))?;
        registry.add_marker_kinds(vec![PluginMarkerKind::new("toy-full", "Disk full").color("#d33")])?;
        Ok(())
    }

    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        Some(Arc::new(Settings {
            enabled: self.enabled.clone(),
        }))
    }
}

pub fn main() {
    let plugin = Arc::new(ToyPlugin {
        enabled: Arc::new(AtomicBool::new(true)),
    });
    if let Err(e) = imt_plugin_sdk::serve(plugin) {
        eprintln!("toy-plugin: {e}");
        std::process::exit(1);
    }
}
