//! Classifies a profile change by the data work it needs (engine review finding 4), so a store does
//! the least: a presentation change needs none, a derivation change re-derives fields and markers from
//! the stored events' raw text, a parsing change re-reads the sources.

use crate::profile::Profile;
use serde::Serialize;
use serde_json::Value;

/// The data work a profile change needs, from least to most.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum ChangeClass {
    /// Nothing stored changes (charts, dashboards, lenses, instance key, activity, spans, live tail, marker
    /// looks, overrides, icons, redaction of the view, the name).
    Presentation,
    /// Fields and markers are re-derived from each event's raw text (extractors, pipeline, marker rules).
    Derivation,
    /// Sources are read again (which files; a parser's format changes are classified per parser, ADR 0026).
    Parsing,
}

/// A classified profile change: its class and the profile sections that changed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileChange {
    pub class: ChangeClass,
    /// Changed sections, e.g. `files`, `extract`, `marker.when`, `chart`.
    pub sections: Vec<String>,
}

impl ProfileChange {
    pub fn is_noop(&self) -> bool {
        self.sections.is_empty()
    }
}

/// The marker fields that decide which events carry a marker (stored per event); the others are looks.
const MARKER_RULE: [&str; 2] = ["kind", "when"];

fn json(v: &impl Serialize) -> Value {
    serde_json::to_value(v).unwrap_or(Value::Null)
}

/// `(kind, when)` of every marker rule, in order (the order gives the marker bits).
fn marker_rules(p: &Profile) -> Value {
    Value::Array(
        json(&p.marker)
            .as_array()
            .into_iter()
            .flatten()
            .map(|m| Value::Array(MARKER_RULE.iter().map(|k| m.get(*k).cloned().unwrap_or(Value::Null)).collect()))
            .collect(),
    )
}

/// What changed from `old` to `new` and the work it needs.
pub fn classify(old: &Profile, new: &Profile) -> ProfileChange {
    let mut sections = Vec::new();
    let mut class = ChangeClass::Presentation;
    let mut note = |name: &str, changed: bool, c: ChangeClass| {
        if changed {
            sections.push(name.to_string());
            class = class.max(c);
        }
    };
    note("files", json(&old.files) != json(&new.files), ChangeClass::Parsing);
    note("extract", json(&old.extract) != json(&new.extract), ChangeClass::Derivation);
    note("pipeline", json(&old.pipeline) != json(&new.pipeline), ChangeClass::Derivation);
    note("marker.when", marker_rules(old) != marker_rules(new), ChangeClass::Derivation);
    note(
        "marker",
        marker_rules(old) == marker_rules(new) && json(&old.marker) != json(&new.marker),
        ChangeClass::Presentation,
    );
    for (name, a, b) in [
        ("name", json(&old.name), json(&new.name)),
        ("instance", json(&old.instance), json(&new.instance)),
        ("activity", json(&old.activity), json(&new.activity)),
        (
            "debrief_disabled_markers",
            json(&old.debrief_disabled_markers),
            json(&new.debrief_disabled_markers),
        ),
        ("chart", json(&old.chart), json(&new.chart)),
        ("icon", json(&old.icon), json(&new.icon)),
        ("live", json(&old.live), json(&new.live)),
        ("marker_override", json(&old.marker_override), json(&new.marker_override)),
        ("redact", json(&old.redact), json(&new.redact)),
        ("dashboard", json(&old.dashboard), json(&new.dashboard)),
        ("lens", json(&old.lens), json(&new.lens)),
        ("span", json(&old.span), json(&new.span)),
    ] {
        note(name, a != b, ChangeClass::Presentation);
    }
    ProfileChange { class, sections }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn with(toml: &str) -> Profile {
        Profile::from_toml(toml).unwrap()
    }

    #[test]
    fn changes_are_classified_by_the_work_they_need() {
        let base = Profile::default_profile();
        assert!(classify(&base, &base.clone()).is_noop());
        let renamed = with("name = \"x\"");
        let c = classify(&base, &renamed);
        assert_eq!((c.class, c.sections.clone()), (ChangeClass::Presentation, vec!["name".to_string()]));
        let chart = with("[[chart]]\ntitle = \"t\"\ny = \"count()\"");
        assert_eq!(classify(&base, &chart).class, ChangeClass::Presentation);
        let lens = with("[[lens]]\nid = \"l\"\ntitle = \"L\"");
        let c = classify(&base, &lens);
        assert_eq!((c.class, c.sections), (ChangeClass::Presentation, vec!["lens".to_string()]));
        let extract = with("[[extract]]\nname = \"ms\"\nregex = '(?P<ms>\\d+)ms'");
        assert_eq!(classify(&base, &extract).class, ChangeClass::Derivation);
        let parse = with("[files]\ninclude = [\"**/*.log\"]");
        let c = classify(&base, &parse);
        assert_eq!(c.class, ChangeClass::Parsing);
        assert_eq!(c.sections, vec!["files".to_string()]);
        // A marker's look is presentation; its rule is derivation.
        let marker = |color: &str, value: &str| {
            with(&format!(
                "[[marker]]\nkind = \"boot\"\ncolor = \"{color}\"\nwhen = [{{ field = \"@message\", op = \"contains\", value = \"{value}\" }}]"
            ))
        };
        let a = marker("#f00", "start");
        assert_eq!(classify(&a, &marker("#0f0", "start")).class, ChangeClass::Presentation);
        let c = classify(&a, &marker("#f00", "boot"));
        assert_eq!((c.class, c.sections), (ChangeClass::Derivation, vec!["marker.when".to_string()]));
        // The largest class wins.
        let both = with("name = \"y\"\n[files]\nexclude = []\n[[extract]]\nname = \"a\"\nregex = '(?P<a>x)'");
        assert_eq!(classify(&base, &both).class, ChangeClass::Parsing);
    }
}
