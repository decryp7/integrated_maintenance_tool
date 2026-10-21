//! Marker kinds a plugin contributes: style, an optional lifecycle role and default rules in the condition
//! language of a profile's `[[marker]] when`. They are namespaced `<plugin-id>/<kind>` by the host, which
//! compiles their rules.

use crate::condition::{CondValue, ConditionCfg};
use serde::{Deserialize, Serialize};

/// Lifecycle meaning of a marker kind; such kinds are always shown on the timeline and in charts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum MarkerRole {
    /// The kind marks the start of a run or session.
    Start,
    /// The kind marks the end of a run or session.
    Stop,
    /// The kind marks an abnormal termination.
    Crash,
}

/// A marker kind a plugin contributes.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PluginMarkerKind {
    /// Short id (`app-crash`); shown as `<plugin-id>/app-crash`.
    pub kind: String,
    /// Human-readable name.
    pub label: String,
    /// Longer explanation shown in tooltips and settings.
    pub description: Option<String>,
    /// #RGB or #RRGGBB.
    pub color: Option<String>,
    /// One of the profile's marker icons.
    pub icon: Option<String>,
    /// 1–3 characters; takes precedence over `icon`.
    pub badge: Option<String>,
    /// circle, square, diamond or hexagon.
    pub shape: Option<String>,
    /// Lifecycle role, when the kind marks a start, stop or crash.
    pub role: Option<MarkerRole>,
    /// Default rules (all must match); empty = only marked programmatically.
    pub when: Vec<ConditionCfg>,
}

impl PluginMarkerKind {
    /// Creates a kind with a short id and a label and no style or rules.
    pub fn new(kind: &str, label: &str) -> Self {
        PluginMarkerKind {
            kind: kind.into(),
            label: label.into(),
            description: None,
            color: None,
            icon: None,
            badge: None,
            shape: None,
            role: None,
            when: Vec::new(),
        }
    }

    /// Sets the description.
    pub fn describe(mut self, text: &str) -> Self {
        self.description = Some(text.into());
        self
    }

    /// Sets the colour (`#RGB` or `#RRGGBB`).
    pub fn color(mut self, color: &str) -> Self {
        self.color = Some(color.into());
        self
    }

    /// Sets one of the profile's marker icons.
    pub fn icon(mut self, icon: &str) -> Self {
        self.icon = Some(icon.into());
        self
    }

    /// Sets the badge text (1â€“3 characters) and its shape (circle, square, diamond or hexagon).
    pub fn badge(mut self, badge: &str, shape: &str) -> Self {
        self.badge = Some(badge.into());
        self.shape = Some(shape.into());
        self
    }

    /// Sets the lifecycle role.
    pub fn role(mut self, role: MarkerRole) -> Self {
        self.role = Some(role);
        self
    }

    /// Adds a condition (`field`, `op`, `value`) to the default rule.
    pub fn when(mut self, field: &str, op: &str, value: &str) -> Self {
        self.when.push(ConditionCfg {
            field: field.into(),
            op: op.into(),
            value: Some(CondValue::Text(value.into())),
        });
        self
    }

    /// Adds an `in` condition: `field` is one of `values`.
    pub fn when_in(mut self, field: &str, values: &[&str]) -> Self {
        self.when.push(ConditionCfg {
            field: field.into(),
            op: "in".into(),
            value: Some(CondValue::List(values.iter().map(|v| v.to_string()).collect())),
        });
        self
    }

    /// Whether the short id is well formed (1–64 of A–Z, a–z, 0–9, '.', '_', '-').
    pub fn valid_id(&self) -> bool {
        !self.kind.is_empty() && self.kind.len() <= 64 && self.kind.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_build_and_roundtrip() {
        let k = PluginMarkerKind::new("app-crash", "Crash")
            .describe("d")
            .color("#f00")
            .icon("bug")
            .badge("!", "circle")
            .role(MarkerRole::Crash)
            .when("Provider", "eq", "x")
            .when_in("EventId", &["1", "2"]);
        assert!(k.valid_id() && !PluginMarkerKind::new("bad kind", "x").valid_id());
        let back: PluginMarkerKind = serde_json::from_value(serde_json::to_value(&k).unwrap()).unwrap();
        assert_eq!(back, k);
    }
}
