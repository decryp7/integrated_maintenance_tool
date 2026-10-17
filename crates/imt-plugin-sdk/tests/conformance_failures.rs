//! The conformance kit catches misbehaving plugins: crashes on fuzzed input, exporters that lose rows,
//! annotators and settings that fail, transforms that crash.

use imt_core::importers::{ImportCtx, Imported, Importer};
use imt_core::model::Record;
use imt_core::plugin::{RecordTransform, TransformCtx, Verdict};
use imt_core::settings::{Configurable, FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
use imt_core::{Plugin, PluginRegistry};
use imt_ipc::endpoint::{new_token, PrivateEndpoint};
use imt_plugin_sdk::conformance::{run, Endpoint, Options};
use imt_plugin_sdk::server::PluginService;
use imt_plugin_sdk::ServeOptions;
use std::sync::Arc;
use std::time::Duration;

struct Crashy;
impl Importer for Crashy {
    fn id(&self) -> &'static str {
        "crashy"
    }
    fn sniff(&self, _: &ImportCtx, head: &str) -> u8 {
        if head.len() > 100 {
            panic!("long heads crash");
        }
        0
    }
    fn import(&self, _: &ImportCtx, content: &str) -> anyhow::Result<Imported> {
        if content.contains('\0') || content.len() > 1000 {
            panic!("crash on odd input");
        }
        Ok(Imported::default())
    }
    fn import_tail(&self, _: &ImportCtx, _: &str, _: &str) -> Option<anyhow::Result<Imported>> {
        panic!("tail always crashes")
    }
}

struct BadTransform;
impl RecordTransform for BadTransform {
    fn id(&self) -> &str {
        "bad"
    }
    fn apply(&self, _: &mut Record, _: &TransformCtx) -> Verdict {
        panic!("transform crashes")
    }
}

struct Lossy;
impl imt_core::export::Exporter for Lossy {
    fn id(&self) -> &'static str {
        "lossy"
    }
    fn name(&self) -> &'static str {
        "Lossy"
    }
    fn extension(&self) -> &'static str {
        "x"
    }
    fn export(&self, rows: &mut dyn Iterator<Item = imt_core::export::ExportRow<'_>>, _: &mut dyn std::io::Write) -> anyhow::Result<usize> {
        Ok(rows.count() / 2)
    }
}

struct Broken;
impl imt_core::export::Exporter for Broken {
    fn id(&self) -> &'static str {
        "broken"
    }
    fn name(&self) -> &'static str {
        "Broken"
    }
    fn extension(&self) -> &'static str {
        "x"
    }
    fn export(&self, _: &mut dyn Iterator<Item = imt_core::export::ExportRow<'_>>, _: &mut dyn std::io::Write) -> anyhow::Result<usize> {
        panic!("export crashes")
    }
}

struct Annot;
impl imt_core::plugin::annotate::Annotator for Annot {
    fn id(&self) -> &str {
        "annot"
    }
    fn annotate(
        &self,
        _: &dyn imt_core::plugin::annotate::AnnotationView,
        _: Option<imt_core::plugin::annotate::RecordCursor>,
    ) -> anyhow::Result<Vec<imt_core::plugin::annotate::Annotation>> {
        panic!("annotator crashes")
    }
}

struct Refuses;
impl Configurable for Refuses {
    fn settings_section(&self) -> SettingsSection {
        SettingsSection::new("x", "S", SectionOwner::Core).field(SettingField::new("b", "B", FieldKind::Bool, false))
    }
    fn apply_settings(&self, _: &ValidValues) -> anyhow::Result<()> {
        anyhow::bail!("never")
    }
}

struct P;
impl Plugin for P {
    fn id(&self) -> &'static str {
        "test.bad"
    }
    fn name(&self) -> &'static str {
        "Bad"
    }
    fn version(&self) -> &'static str {
        "0.0.1"
    }
    fn register(&self, r: &mut PluginRegistry) -> anyhow::Result<()> {
        r.add_importer(Box::new(Crashy));
        r.add_transform(Arc::new(BadTransform))?;
        r.add_exporter(Arc::new(Lossy))?;
        r.add_exporter(Arc::new(Broken))?;
        r.add_annotator(Arc::new(Annot))?;
        Ok(())
    }
    fn configurable(&self) -> Option<Arc<dyn Configurable>> {
        Some(Arc::new(Refuses))
    }
}

#[test]
fn a_misbehaving_plugin_fails_the_kit_with_reasons() {
    let endpoint = PrivateEndpoint::create_named("imt-sdk-bad").unwrap();
    let token = new_token().unwrap();
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build().unwrap();
    let routes = PluginService::new(Arc::new(P), ServeOptions::default()).unwrap().routes();
    let (address, t) = (endpoint.address.clone(), token.clone());
    rt.spawn(async move {
        let _ = imt_ipc::server::serve(routes, &address, &t, Default::default(), std::future::pending()).await;
    });
    std::thread::sleep(Duration::from_millis(100));
    let report = run(
        &Endpoint {
            address: endpoint.address.clone(),
            token,
        },
        Options {
            fuzz_cases: 16,
            ..Options::default()
        },
    );
    assert!(!report.passed());
    let failed: Vec<String> = report.failures().iter().map(|c| c.name.clone()).collect();
    for want in [
        "importer \"crashy\": never fails on 16 fuzzed inputs",
        "transform \"bad\": answers every batch with as many rows",
        "exporter \"lossy\": writes every row",
        "exporter \"broken\": writes every row",
        "annotator \"annot\" answers",
        "applying the default settings succeeds",
    ] {
        assert!(failed.iter().any(|f| f == want), "{want} not in {failed:?}\n{report}");
    }
    let text = report.to_string();
    assert!(text.contains("FAIL") && text.contains("test.bad 0.0.1"), "{text}");
}
