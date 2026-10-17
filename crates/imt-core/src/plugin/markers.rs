//! Marker kinds contributed by plugins.
//!
//! A plugin declares kinds with [`super::PluginRegistry::add_marker_kinds`]:
//! style, an optional lifecycle role and default rules in the same condition
//! language as a profile's `[[marker]] when`. Kinds are namespaced
//! `<plugin-id>/<kind>`.
//!
//! The effective marker set, in priority order (the first matching kind is a
//! record's primary marker), is: the profile's own rules, then plugin kinds,
//! then the built-in defaults. `[[marker_override]]` entries in the profile
//! switch kinds off or restyle them. Plugin transforms may also mark records
//! with their own kinds ([`super::TransformCtx::mark`]).
//!
//! Kinds of a plugin that is quarantined, disabled or inactive are hidden
//! (their bits are masked out of query results) until it runs again.

use crate::condition::{CondValue, ConditionCfg};
use crate::model::{MarkerMask, MAX_MARKER_KINDS};
use crate::profile::{compile_marker, CompiledProfile, Marker, MarkerCfg, Profile};
use anyhow::{anyhow, Result};
use serde::Serialize;
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

/// Owner of the profile's own marker rules.
pub const OWNER_PROFILE: &str = "profile";
/// Owner of the built-in default rules (those of the default profile).
pub const OWNER_BUILTIN: &str = "builtin";

/// Lifecycle meaning of a marker kind; such kinds are always shown on the
/// timeline and in charts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum MarkerRole {
    Start,
    Stop,
    Crash,
}

/// A marker kind a plugin contributes.
#[derive(Debug, Clone)]
pub struct PluginMarkerKind {
    /// Short id (`app-crash`); shown as `<plugin-id>/app-crash`.
    pub kind: String,
    pub label: String,
    pub description: Option<String>,
    /// #RGB or #RRGGBB.
    pub color: Option<String>,
    /// One of [`crate::profile::MARKER_ICONS`].
    pub icon: Option<String>,
    /// 1–3 characters; takes precedence over `icon`.
    pub badge: Option<String>,
    /// circle, square, diamond or hexagon.
    pub shape: Option<String>,
    pub role: Option<MarkerRole>,
    /// Default rules (all must match); empty = only marked programmatically.
    pub when: Vec<ConditionCfg>,
}

impl PluginMarkerKind {
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

    pub fn describe(mut self, text: &str) -> Self {
        self.description = Some(text.into());
        self
    }

    pub fn color(mut self, color: &str) -> Self {
        self.color = Some(color.into());
        self
    }

    pub fn icon(mut self, icon: &str) -> Self {
        self.icon = Some(icon.into());
        self
    }

    pub fn badge(mut self, badge: &str, shape: &str) -> Self {
        self.badge = Some(badge.into());
        self.shape = Some(shape.into());
        self
    }

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

    /// The rule as a profile `[[marker]]` entry, under its full name.
    fn to_cfg(&self, owner: &str) -> MarkerCfg {
        MarkerCfg {
            kind: format!("{owner}/{}", self.kind),
            label: Some(self.label.clone()),
            lifecycle: self.role.is_some(),
            color: self.color.clone(),
            icon: self.icon.clone(),
            badge: self.badge.clone(),
            shape: self.shape.clone(),
            description: self.description.clone(),
            when: self.when.clone(),
        }
    }

    /// Checks the kind as `owner` would contribute it.
    pub(crate) fn check(&self, owner: &str) -> Result<()> {
        if self.kind.is_empty() || self.kind.len() > 64 || !self.kind.chars().all(|c| c.is_ascii_alphanumeric() || "._-".contains(c)) {
            return Err(anyhow!(
                "marker kind \"{}\": use 1–64 characters from A–Z, a–z, 0–9, '.', '_' and '-'",
                self.kind
            ));
        }
        compile_marker(&self.to_cfg(owner), &[], owner).map(|_| ())
    }
}

fn builtin_rules() -> &'static [serde_json::Value] {
    static RULES: OnceLock<Vec<serde_json::Value>> = OnceLock::new();
    RULES.get_or_init(|| {
        Profile::default_profile()
            .marker
            .iter()
            .filter_map(|m| serde_json::to_value(m).ok())
            .collect()
    })
}

/// Replaces `compiled.markers` with the effective marker set: the profile's
/// own rules, then `plugin_kinds` (owner, kind), then built-in defaults, with
/// `[[marker_override]]` applied. Returns notes (owner, text) about kinds
/// that did not fit.
pub(crate) fn apply(compiled: &mut CompiledProfile, plugin_kinds: &[(String, PluginMarkerKind)]) -> Result<Vec<(String, String)>> {
    if plugin_kinds.is_empty() && compiled.profile.marker_override.is_empty() {
        return Ok(Vec::new());
    }
    let builtin = builtin_rules();
    let mut user = Vec::new();
    let mut defaults = Vec::new();
    for m in &compiled.profile.marker {
        let is_builtin = serde_json::to_value(m).is_ok_and(|v| builtin.contains(&v));
        let entry = (if is_builtin { OWNER_BUILTIN } else { OWNER_PROFILE }.to_string(), m.clone(), None);
        if is_builtin {
            defaults.push(entry);
        } else {
            user.push(entry);
        }
    }
    let mut entries: Vec<(String, MarkerCfg, Option<MarkerRole>)> = user;
    entries.extend(plugin_kinds.iter().map(|(owner, k)| (owner.clone(), k.to_cfg(owner), k.role)));
    entries.extend(defaults);

    for o in &compiled.profile.marker_override {
        let Some(i) = entries.iter().position(|(_, m, _)| m.kind == o.kind) else {
            continue; // e.g. the plugin is not installed: keep the override for later
        };
        if o.enabled == Some(false) {
            entries.remove(i);
            continue;
        }
        let m = &mut entries[i].1;
        for (field, value) in [
            (&mut m.label, &o.label),
            (&mut m.color, &o.color),
            (&mut m.icon, &o.icon),
            (&mut m.badge, &o.badge),
            (&mut m.shape, &o.shape),
            (&mut m.description, &o.description),
        ] {
            if value.is_some() {
                field.clone_from(value);
            }
        }
    }

    // Too many: drop the lowest-priority plugin kinds; profile and built-in kinds stay.
    let excess: Vec<usize> = entries
        .iter()
        .enumerate()
        .rev()
        .filter(|(_, (o, _, _))| o != OWNER_PROFILE && o != OWNER_BUILTIN)
        .map(|(i, _)| i)
        .take(entries.len().saturating_sub(MAX_MARKER_KINDS))
        .collect();
    let mut notes = Vec::new();
    for i in excess {
        let (owner, m, _) = entries.remove(i);
        notes.push((
            owner,
            format!("marker kind \"{}\" is not used: at most {MAX_MARKER_KINDS} kinds fit", m.kind),
        ));
    }

    let mut markers = Vec::with_capacity(entries.len());
    let mut lifecycle_mask: MarkerMask = 0;
    for (i, (owner, cfg, role)) in entries.iter().enumerate() {
        let mut marker = compile_marker(cfg, &compiled.icons, owner).map_err(|e| anyhow!("{e:#} (check [[marker_override]])"))?;
        marker.role = *role;
        if marker.lifecycle {
            lifecycle_mask |= 1 << i;
        }
        markers.push(marker);
    }
    compiled.markers = markers;
    compiled.lifecycle_mask = lifecycle_mask;
    Ok(notes)
}

/// Bits of plugin kinds in the effective marker set, for [`super::TransformCtx::mark`].
#[derive(Debug, Default)]
pub struct MarkIndex {
    /// (full kind, owner, bit)
    kinds: Vec<(String, String, u32)>,
}

impl MarkIndex {
    pub fn of(markers: &[Marker]) -> MarkIndex {
        MarkIndex {
            kinds: markers
                .iter()
                .enumerate()
                .filter(|(_, m)| m.owner != OWNER_PROFILE && m.owner != OWNER_BUILTIN)
                .map(|(i, m)| (m.kind.clone(), m.owner.clone(), i as u32))
                .collect(),
        }
    }

    fn bit(&self, owner: &str, kind: &str) -> Option<u32> {
        let full = if kind.contains('/') {
            kind.to_string()
        } else {
            format!("{owner}/{kind}")
        };
        self.kinds.iter().find(|(k, o, _)| *k == full && o == owner).map(|(_, _, b)| *b)
    }
}

/// Marks one plugin transform call made on one record.
pub(crate) struct MarkScope<'a> {
    owner: &'a str,
    marked: Cell<MarkerMask>,
    rejected: RefCell<Vec<String>>,
}

impl<'a> MarkScope<'a> {
    pub fn new(owner: &'a str) -> Self {
        MarkScope {
            owner,
            marked: Cell::new(0),
            rejected: RefCell::new(Vec::new()),
        }
    }

    pub fn mark(&self, marks: &MarkIndex, kind: &str) -> bool {
        match marks.bit(self.owner, kind) {
            Some(b) => {
                self.marked.set(self.marked.get() | (1 << b));
                true
            }
            None => self.reject(kind),
        }
    }

    pub fn reject(&self, kind: &str) -> bool {
        self.rejected.borrow_mut().push(kind.to_string());
        false
    }

    /// The marks made, and the kinds that were refused.
    pub fn finish(self) -> (MarkerMask, Vec<String>) {
        (self.marked.get(), self.rejected.into_inner())
    }
}

/// Bits of `markers` whose owner `available` says may be shown now.
pub(crate) fn visible_mask(markers: &[Marker], available: impl Fn(&str) -> bool) -> MarkerMask {
    markers
        .iter()
        .enumerate()
        .filter(|(_, m)| m.owner == OWNER_PROFILE || m.owner == OWNER_BUILTIN || available(&m.owner))
        .fold(0, |mask, (i, _)| mask | (1 << i))
}

#[cfg(test)]
mod tests;
