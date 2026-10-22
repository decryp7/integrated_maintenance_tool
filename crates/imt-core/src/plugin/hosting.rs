//! Read access to what plugins registered, for serving them out of process.
//!
//! The plugins process (`imt-plugins`) installs a plugin into a registry of
//! its own and then answers IMT's calls over imt-ipc (ADR 0017) with its
//! contributions. These accessors expose those
//! contributions (already wrapped in their guarded adapters) without
//! letting anything else change the registry.

use super::annotate::Annotator;
use super::markers::PluginMarkerKind;
use super::{PluginRegistry, RecordTransform};
use crate::export::Exporter;
use crate::importers::{BinaryImporter, Importer};
use crate::settings::Configurable;
use std::sync::Arc;

impl PluginRegistry {
    /// Text importers, in registration order.
    pub fn hosted_importers(&self) -> impl Iterator<Item = &dyn Importer> {
        self.importers.text_importers()
    }

    /// Binary importers, in registration order.
    pub fn hosted_binary_importers(&self) -> impl Iterator<Item = &dyn BinaryImporter> {
        self.importers.binary_importers()
    }

    /// Transforms, sorted by id.
    pub fn hosted_transforms(&self) -> Vec<Arc<dyn RecordTransform>> {
        let mut t: Vec<_> = self.transforms.values().cloned().collect();
        t.sort_by(|a, b| a.id().cmp(b.id()));
        t
    }

    /// Exporters, in registration order.
    pub fn hosted_exporters(&self) -> &[Arc<dyn Exporter>] {
        &self.exporters
    }

    /// Marker kinds `owner` declared.
    pub fn hosted_marker_kinds(&self, owner: &str) -> Vec<PluginMarkerKind> {
        self.marker_kinds
            .iter()
            .filter(|(o, _)| o == owner)
            .map(|(_, k)| k.clone())
            .collect()
    }

    /// Annotators, in registration order, guarded like every plugin call.
    pub fn hosted_annotators(&self) -> Vec<Arc<dyn Annotator>> {
        self.annotators
            .iter()
            .map(|a| -> Arc<dyn Annotator> {
                match &a.guard {
                    Some(guard) => Arc::new(super::annotate::GuardedAnnotator {
                        inner: a.annotator.clone(),
                        guard: guard.clone(),
                    }),
                    None => a.annotator.clone(),
                }
            })
            .collect()
    }

    /// The settings of plugin `id` (guarded), if it has any.
    pub fn hosted_settings(&self, id: &str) -> Option<Arc<dyn Configurable>> {
        self.configurables
            .iter()
            .find(|(owner, _)| owner == id)
            .map(|(_, c)| -> Arc<dyn Configurable> { c.clone() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::plugin::markers::MarkIndex;
    use crate::plugin::{Plugin, TransformCtx, Verdict};
    use crate::settings::{FieldKind, SectionOwner, SettingField, SettingsSection, ValidValues};
    use anyhow::Result;

    struct T;
    impl RecordTransform for T {
        fn id(&self) -> &str {
            "t"
        }
        fn apply(&self, _: &mut crate::model::Record, _: &TransformCtx) -> Verdict {
            Verdict::Keep
        }
    }

    struct A;
    impl Annotator for A {
        fn id(&self) -> &str {
            "a"
        }
        fn annotate(
            &self,
            _: &dyn super::super::annotate::AnnotationView,
            _: Option<super::super::annotate::RecordCursor>,
        ) -> Result<Vec<super::super::annotate::Annotation>> {
            panic!("annotator blew up")
        }
    }

    struct S;
    impl Configurable for S {
        fn settings_section(&self) -> SettingsSection {
            SettingsSection::new("x", "S", SectionOwner::Core).field(SettingField::new("on", "On", FieldKind::Bool, true))
        }
        fn apply_settings(&self, _: &ValidValues) -> Result<()> {
            Ok(())
        }
    }

    struct P;
    impl Plugin for P {
        fn id(&self) -> &'static str {
            "t.hosted"
        }
        fn name(&self) -> &'static str {
            "Hosted"
        }
        fn version(&self) -> &'static str {
            "1.0.0"
        }
        fn register(&self, r: &mut PluginRegistry) -> Result<()> {
            r.add_importer(Box::new(crate::importers::TextImporter));
            r.add_transform(Arc::new(T))?;
            r.add_exporter(Arc::new(crate::export::CsvExporter))?;
            r.add_marker_kinds(vec![PluginMarkerKind::new("k", "K")])?;
            r.add_annotator(Arc::new(A))?;
            Ok(())
        }
        fn configurable(&self) -> Option<Arc<dyn Configurable>> {
            Some(Arc::new(S))
        }
    }

    #[test]
    fn a_plugins_contributions_are_listed_guarded() {
        let mut r = PluginRegistry::empty();
        r.install(&P).unwrap();
        assert_eq!(r.hosted_importers().map(|i| i.id()).collect::<Vec<_>>(), ["text"]);
        assert_eq!(r.hosted_binary_importers().count(), 0);
        assert_eq!(r.hosted_transforms().len(), 1);
        assert_eq!(r.hosted_exporters()[0].id(), "csv");
        assert_eq!(r.hosted_marker_kinds("t.hosted")[0].kind, "k");
        assert!(r.hosted_marker_kinds("other").is_empty());
        assert_eq!(r.hosted_settings("t.hosted").unwrap().settings_section().id, "t.hosted");
        assert!(r.hosted_settings("other").is_none());
        let a = r.hosted_annotators();
        struct V;
        impl super::super::annotate::AnnotationView for V {
            fn select(&self, _: &super::super::annotate::ViewFilter, _: usize) -> Result<Vec<super::super::annotate::ViewRecord>> {
                Ok(Vec::new())
            }
            fn query(&self, _: &crate::query::Query, _: usize) -> Result<Vec<super::super::annotate::ViewRecord>> {
                Ok(Vec::new())
            }
            fn instances(&self) -> Vec<String> {
                Vec::new()
            }
            fn time_range(&self) -> Option<(i64, i64)> {
                None
            }
        }
        let e = a[0].annotate(&V, None).unwrap_err().to_string();
        assert!(e.contains("crashed") && e.contains("annotator blew up"), "{e}");
        assert_eq!(r.health().get("t.hosted").failures, 1);
    }

    /// A transform that wants whole batches: drops records containing "drop", marks those containing
    /// "mark", panics on "panic".
    struct Batched;
    impl RecordTransform for Batched {
        fn id(&self) -> &str {
            "batched"
        }
        fn apply(&self, _: &mut crate::model::Record, _: &TransformCtx) -> Verdict {
            unreachable!("the pipeline hands batched transforms whole batches")
        }
        fn batched(&self) -> bool {
            true
        }
        fn apply_batch(&self, records: &mut [crate::model::Record], ctx: &TransformCtx) -> Vec<Verdict> {
            let answered = records.len().saturating_sub(1).max(1);
            records
                .iter_mut()
                .map(|r| {
                    if r.message == "panic" {
                        panic!("batch blew up");
                    }
                    if r.message.contains("mark") {
                        assert!(ctx.mark_record(r, "k"));
                        assert!(!ctx.mark_record(r, "other.plugin/k"));
                    }
                    r.message = r.message.to_uppercase();
                    if r.message.contains("DROP") {
                        Verdict::Drop
                    } else {
                        Verdict::Keep
                    }
                })
                .take(answered)
                .collect()
        }
    }

    struct BatchPlugin;
    impl Plugin for BatchPlugin {
        fn id(&self) -> &'static str {
            "t.batch"
        }
        fn name(&self) -> &'static str {
            "Batch"
        }
        fn version(&self) -> &'static str {
            "1.0.0"
        }
        fn register(&self, r: &mut PluginRegistry) -> Result<()> {
            r.add_transform(Arc::new(Batched))?;
            r.add_marker_kinds(vec![PluginMarkerKind::new("k", "K")])?;
            Ok(())
        }
    }

    fn rec(i: u32, m: &str, level: crate::model::Level) -> crate::model::Record {
        let mut r = crate::model::Record::new(i);
        r.message = m.into();
        r.level = level;
        r
    }

    #[test]
    fn the_pipeline_hands_batched_transforms_whole_batches_in_order() {
        use crate::model::Level;
        let mut r = PluginRegistry::empty();
        r.install(&BatchPlugin).unwrap();
        let steps: Vec<crate::pipeline::StepCfg> = serde_json::from_value(serde_json::json!([
            { "step": "plugin", "id": "batched", "when": [{ "field": "@level", "op": "eq", "value": "Error" }] }
        ]))
        .unwrap();
        let pipeline = crate::pipeline::Pipeline::compile(&steps, &r.transforms).unwrap();
        let marks = MarkIndex::of_kinds("t.batch", &["k".to_string()]);
        let records = vec![
            rec(1, "info stays", Level::Information),
            rec(2, "please drop", Level::Error),
            rec(3, "mark me", Level::Error),
            rec(4, "also info", Level::Information),
            rec(5, "last error", Level::Error),
        ];
        let (out, stats) = pipeline.run_marking(records, "s", &marks);
        let lines: Vec<u32> = out.iter().map(|r| r.line).collect();
        assert_eq!(lines, [1, 3, 4, 5], "order kept, the dropped one gone");
        assert_eq!(out[0].message, "info stays", "not selected: untouched");
        assert_eq!(out[1].message, "MARK ME");
        assert_eq!(out[1].markers, 1);
        // The batch returned one verdict too few (and left that record alone): a missing verdict means Keep.
        assert_eq!(out[3].message, "last error");
        assert_eq!((stats[0].input, stats[0].output), (5, 4));
        assert!(r.health().notes("t.batch").iter().any(|n| n.contains("other.plugin/k")));
        // A crash keeps every record and counts.
        let (out, _) = pipeline.run_marking(vec![rec(1, "panic", Level::Error), rec(2, "x", Level::Error)], "s", &marks);
        assert_eq!(out.len(), 2);
        assert_eq!(r.health().get("t.batch").failures, 1);
        // Without a scope (a bare context) marks are refused quietly.
        let mut lone = rec(1, "mark", Level::Error);
        assert!(!TransformCtx::new("s").mark_record(&mut lone, "k"));
        let unmarked = TransformCtx::with_marks("s", &marks);
        assert!(!unmarked.mark_record(&mut lone, "k"), "no plugin scope: nothing is marked");
        assert_eq!(lone.markers, 0);
    }

    #[test]
    fn guarded_per_record_transforms_also_take_batches() {
        let mut r = PluginRegistry::empty();
        r.install(&P).unwrap();
        let t = r.hosted_transforms().pop().unwrap();
        assert!(!t.batched());
        let mut recs = vec![crate::model::Record::new(1), crate::model::Record::new(2)];
        assert_eq!(t.apply_batch(&mut recs, &TransformCtx::new("s")), [Verdict::Keep, Verdict::Keep]);
        // The default of the trait (no guard).
        assert_eq!(T.apply_batch(&mut recs, &TransformCtx::new("s")).len(), 2);
    }
}
