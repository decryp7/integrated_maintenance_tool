//! An example annotator plugin: marks **error bursts**, three or more errors on one instance within a minute, as
//! ranges on the timeline. An annotator reads the events through a bounded, read-only view the host lends it for
//! the call; it never sees files.

use anyhow::Result;
use debrief_plugin_api::annotate::{Annotation, AnnotationView, Annotator, Evidence, RecordCursor, Severity, ViewFilter};
use debrief_plugin_api::condition::{CondValue, ConditionCfg};
use debrief_plugin_api::markers::PluginMarkerKind;
use debrief_plugin_api::prelude::*;

/// Errors closer than this belong to one burst.
pub const GAP_MS: i64 = 60_000;
/// Errors a burst needs.
pub const MIN_ERRORS: usize = 3;
/// Most errors read per call.
const LIMIT: usize = 10_000;

/// The annotator.
pub struct ErrorBursts;

impl Annotator for ErrorBursts {
    fn id(&self) -> &str {
        "error-bursts"
    }

    fn annotate(&self, view: &dyn AnnotationView, _since: Option<RecordCursor>) -> Result<Vec<Annotation>> {
        let filter = ViewFilter {
            when: vec![ConditionCfg::new(
                "@level",
                "in",
                CondValue::List(vec!["Error".into(), "Fatal".into()]),
            )],
            ..ViewFilter::default()
        };
        let errors = view.select(&filter, LIMIT)?;
        let mut out = Vec::new();
        for instance in view.instances() {
            let times: Vec<i64> = errors
                .iter()
                .filter(|r| r.instance == instance)
                .filter_map(|r| r.record.ts)
                .collect();
            let mut start = 0;
            for i in 1..=times.len() {
                if i == times.len() || times[i] - times[i - 1] > GAP_MS {
                    let n = i - start;
                    if n >= MIN_ERRORS {
                        out.push(Annotation {
                            kind: "burst".into(),
                            from: times[start],
                            to: times[i - 1],
                            instance: Some(instance.clone()),
                            on: None,
                            label: format!("{n} errors"),
                            severity: Severity::Warning,
                            evidence: Evidence {
                                text: "level:error".into(),
                                from: Some(times[start]),
                                to: Some(times[i - 1]),
                                ..Evidence::default()
                            },
                        });
                    }
                    start = i;
                }
            }
        }
        Ok(out)
    }
}

/// The plugin.
pub struct BurstsPlugin;

impl Plugin for BurstsPlugin {
    fn id(&self) -> &str {
        "net.decryptology.example.annotator"
    }

    fn name(&self) -> &str {
        "Error bursts (example)"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        // Its annotations share the style of its marker kind.
        c.add_marker_kinds(vec![PluginMarkerKind::new("burst", "Error burst")]);
        c.add_annotator(ErrorBursts);
        Ok(())
    }
}
