//! A test plugin process for the plugin host's tests (never bundled). Its importer "fake" reacts to the
//! first line of the content: `CRASH` aborts, `HANG` never answers, `OOM` fails an allocation (what running
//! out of memory looks like), `PANIC` panics, `PATH` returns the path it was given. Otherwise every line
//! becomes a record (`level|message|key=value`). It also has a transform, an exporter, an annotator, marker
//! kinds, settings, a service and event counting. Flags: `--want-files`, `--want-network` (permissions),
//! `--fail-start` (exits before serving), `--sleep-start=<ms>`.

use anyhow::{anyhow, Result};
use imt_core::api::Severity;
use imt_core::export::{ExportRow, Exporter};
use imt_core::importers::{ImportCtx, Imported, Importer};
use imt_core::model::{Level, Record};
use imt_core::plugin::annotate::{Annotation, AnnotationView, Annotator, RecordCursor, ViewFilter};
use imt_core::plugin::markers::PluginMarkerKind;
use imt_core::plugin::{ActivationCtx, RecordTransform, TransformCtx, Verdict};
use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use imt_core::{Plugin, PluginRegistry};
use imt_plugin_sdk::{ByteService, Permission, PermissionRequest, ServeOptions};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Default)]
struct State {
    prefix: Mutex<String>,
    events: AtomicU64,
}

struct Fake(Arc<State>);

impl Importer for Fake {
    fn id(&self) -> &'static str {
        "fake"
    }
    fn sniff(&self, ctx: &ImportCtx, head: &str) -> u8 {
        if head.starts_with("SNIFFPANIC") {
            panic!("sniff blew up");
        }
        if ctx.path.extension().is_some_and(|e| e == "fake") || head.starts_with("FAKE") {
            80
        } else {
            0
        }
    }
    fn import(&self, ctx: &ImportCtx, content: &str) -> Result<Imported> {
        match content.lines().next().unwrap_or("") {
            "CRASH" => std::process::abort(),
            "HANG" => loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            },
            "OOM" => {
                std::alloc::handle_alloc_error(std::alloc::Layout::from_size_align(1 << 40, 8).unwrap_or(std::alloc::Layout::new::<u8>()))
            }
            "PANIC" => panic!("the importer panicked"),
            "ERROR" => return Err(anyhow!("this file is not valid")),
            "PATH" => {
                let mut r = Record::new(1);
                r.message = ctx.path.display().to_string();
                return Ok(Imported {
                    records: vec![r],
                    skipped: 0,
                    ..Default::default()
                });
            }
            _ => {}
        }
        let prefix = self.0.prefix.lock().unwrap_or_else(|p| p.into_inner()).clone();
        let mut out = Imported::default();
        for (i, line) in content.lines().enumerate() {
            let mut parts = line.splitn(3, '|');
            let (Some(level), Some(message)) = (parts.next(), parts.next()) else {
                out.skipped += 1;
                continue;
            };
            let mut r = Record::new(i as u32 + 1);
            r.level = Level::parse(level).unwrap_or(Level::Information);
            r.message = format!("{prefix}{message}");
            r.ts = Some(1_700_000_000_000 + i as i64);
            for kv in parts.next().unwrap_or("").split_whitespace() {
                if let Some((k, v)) = kv.split_once('=') {
                    r.fields
                        .insert(k.into(), v.parse::<i64>().map(Into::into).unwrap_or_else(|_| v.into()));
                }
            }
            out.records.push(r);
        }
        Ok(out)
    }
    fn import_tail(&self, ctx: &ImportCtx, _head: &str, tail: &str) -> Option<Result<Imported>> {
        if tail.starts_with("NOTAIL") {
            return None;
        }
        Some(self.import(ctx, tail))
    }
    fn multiline(&self) -> bool {
        false
    }
}

struct FakeBin;

impl imt_core::importers::BinaryImporter for FakeBin {
    fn id(&self) -> &'static str {
        "fakebin"
    }
    fn sniff(&self, _: &ImportCtx, head: &[u8]) -> u8 {
        if head.starts_with(b"FB") {
            100
        } else {
            0
        }
    }
    fn import(&self, _: &ImportCtx, bytes: &[u8]) -> Result<Imported> {
        let mut r = Record::new(1);
        r.message = format!("{} bytes", bytes.len());
        Ok(Imported {
            records: vec![r],
            skipped: 0,
            ..Default::default()
        })
    }
}

struct Upper;

impl RecordTransform for Upper {
    fn id(&self) -> &str {
        "fake.upper"
    }
    fn apply(&self, r: &mut Record, ctx: &TransformCtx) -> Verdict {
        match r.message.as_str() {
            "abort" => std::process::abort(),
            "panic" => panic!("the transform panicked"),
            "hang" => loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            },
            _ => {}
        }
        if r.message.contains("mark") {
            ctx.mark("fake-mark");
            ctx.mark("not-mine");
        }
        if r.message.contains("drop") {
            return Verdict::Drop;
        }
        r.message = r.message.to_uppercase();
        r.fields.insert("upper".into(), true.into());
        Verdict::Keep
    }
}

struct Lines;

impl Exporter for Lines {
    fn id(&self) -> &'static str {
        "fake.lines"
    }
    fn name(&self) -> &'static str {
        "Fake lines"
    }
    fn extension(&self) -> &'static str {
        "txt"
    }
    fn export(&self, rows: &mut dyn Iterator<Item = ExportRow<'_>>, out: &mut dyn std::io::Write) -> Result<usize> {
        let mut n = 0;
        for r in rows {
            writeln!(out, "{}|{}|{}", r.source_name, r.group, r.record.message)?;
            n += 1;
        }
        Ok(n)
    }
}

struct Errors;

impl Annotator for Errors {
    fn id(&self) -> &str {
        "fake.errors"
    }
    fn annotate(&self, view: &dyn AnnotationView, _since: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        let errors = view.select(&ViewFilter::default(), 1000)?;
        Ok(errors
            .iter()
            .filter_map(|v| {
                let ts = v.record.ts?;
                Some(Annotation {
                    kind: "fake-mark".into(),
                    from: ts,
                    to: ts,
                    instance: Some(v.instance.clone()),
                    on: None,
                    label: v.record.message.clone(),
                    severity: Severity::Warning,
                    evidence_query: Default::default(),
                })
            })
            .collect())
    }
}

struct Settings(Arc<State>);

impl Configurable for Settings {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new("x", "Fake", SectionOwner::Core).field(SettingField::new(
            "prefix",
            "Prefix",
            FieldKind::Text {
                max_len: 20,
                pattern: None,
            },
            "",
        ))
    }
    fn apply_settings(&self, values: &ValidValues) -> Result<()> {
        let p = values.text("prefix").unwrap_or("").to_string();
        if p == "explode" {
            return Err(anyhow!("refused"));
        }
        if p == "abort" {
            std::process::abort();
        }
        *self.0.prefix.lock().unwrap_or_else(|e| e.into_inner()) = p;
        Ok(())
    }
}

struct Echo(Arc<State>);

impl ByteService for Echo {
    fn call(&self, method: &str, payload: &[u8]) -> Result<Vec<u8>> {
        match method {
            "echo" => Ok(payload.to_vec()),
            "events" => Ok(self.0.events.load(Ordering::SeqCst).to_string().into_bytes()),
            "panic" => panic!("the service panicked"),
            _ => Err(anyhow!("no method {method}")),
        }
    }
}

struct FakePlugin(Arc<State>);

impl Plugin for FakePlugin {
    fn id(&self) -> &'static str {
        "test.fake"
    }
    fn name(&self) -> &'static str {
        "Fake plugin"
    }
    fn version(&self) -> &'static str {
        "1.2.3"
    }
    fn register(&self, registry: &mut PluginRegistry) -> Result<()> {
        registry.add_importer(Box::new(Fake(self.0.clone())));
        registry.add_binary_importer(Box::new(FakeBin));
        registry.add_transform(Arc::new(Upper))?;
        registry.add_exporter(Arc::new(Lines))?;
        registry.add_marker_kinds(vec![PluginMarkerKind::new("fake-mark", "Fake mark").color("#0a0")])?;
        registry.add_annotator(Arc::new(Errors))?;
        Ok(())
    }
    fn manifest(&self) -> imt_core::plugin::Manifest {
        imt_core::plugin::Manifest::new(self.id(), self.version())
            .optionally("test.optional", "^1")
            .provides("fake.echo", "1.0.0")
    }
    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        Some(Arc::new(Settings(self.0.clone())))
    }
    fn activate(&self, ctx: &ActivationCtx) -> Result<()> {
        let state = self.0.clone();
        ctx.events().subscribe::<imt_core::events::RecordsAppended>(self.id(), move |e| {
            state.events.fetch_add(e.count as u64, Ordering::SeqCst);
        })?;
        let state = self.0.clone();
        ctx.events().subscribe::<imt_core::events::ProfileChanged>(self.id(), move |_| {
            state.events.fetch_add(100, Ordering::SeqCst);
        })?;
        let state = self.0.clone();
        ctx.events().subscribe::<imt_core::events::WorkspaceCleared>(self.id(), move |_| {
            state.events.fetch_add(1000, Ordering::SeqCst);
        })?;
        Ok(())
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.iter().any(|a| a == "--fail-start") {
        std::process::exit(4);
    }
    if let Some(ms) = args
        .iter()
        .find_map(|a| a.strip_prefix("--sleep-start="))
        .and_then(|v| v.parse().ok())
    {
        std::thread::sleep(std::time::Duration::from_millis(ms));
    }
    let state = Arc::new(State::default());
    let mut opts = ServeOptions {
        services: vec![(
            "fake.echo".into(),
            "1.0.0".into(),
            Arc::new(Echo(state.clone())) as Arc<dyn ByteService>,
        )],
        events: vec![
            "recordsAppended".into(),
            "profileChanged".into(),
            "workspaceCleared".into(),
            "fromTheFuture".into(),
        ],
        annotator_interest: vec![(
            "fake.errors".into(),
            vec![imt_core::condition::ConditionCfg {
                field: "@level".into(),
                op: "eq".into(),
                value: Some(imt_core::condition::CondValue::Text("Error".into())),
            }],
            100,
        )],
        ..ServeOptions::default()
    };
    if args.iter().any(|a| a == "--want-files") {
        opts.permissions.push(PermissionRequest {
            permission: Permission::ReadOpenedFiles,
            reason: "to test".into(),
        });
    }
    if args.iter().any(|a| a == "--want-network") {
        opts.permissions.push(PermissionRequest {
            permission: Permission::Network {
                host: "example.com".into(),
            },
            reason: "to test".into(),
        });
    }
    if let Err(e) = imt_plugin_sdk::serve_with(Arc::new(FakePlugin(state)), opts) {
        eprintln!("fake plugin: {e}");
        std::process::exit(1);
    }
}
