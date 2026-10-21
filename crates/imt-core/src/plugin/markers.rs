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

use crate::model::{MarkerMask, MAX_MARKER_KINDS};
use crate::profile::{compile_marker, CompiledProfile, Marker, MarkerCfg, Profile};
use anyhow::{anyhow, Result};
use std::cell::{Cell, RefCell};
use std::sync::OnceLock;

/// Owner of the profile's own marker rules.
pub const OWNER_PROFILE: &str = "profile";
/// Owner of the built-in default rules (those of the default profile).
pub const OWNER_BUILTIN: &str = "builtin";

pub use imt_plugin_api::markers::{MarkerRole, PluginMarkerKind};

/// The rule of a plugin kind as a profile `[[marker]]` entry, under its full name.
fn to_cfg(k: &PluginMarkerKind, owner: &str) -> MarkerCfg {
    MarkerCfg {
        kind: format!("{owner}/{}", k.kind),
        label: Some(k.label.clone()),
        lifecycle: k.role.is_some(),
        color: k.color.clone(),
        icon: k.icon.clone(),
        badge: k.badge.clone(),
        shape: k.shape.clone(),
        description: k.description.clone(),
        when: k.when.clone(),
    }
}

/// Checks a kind as `owner` would contribute it.
pub(crate) fn check(k: &PluginMarkerKind, owner: &str) -> Result<()> {
    if !k.valid_id() {
        return Err(anyhow!(
            "marker kind \"{}\": use 1–64 characters from A–Z, a–z, 0–9, '.', '_' and '-'",
            k.kind
        ));
    }
    compile_marker(&to_cfg(k, owner), &[], owner).map(|_| ())
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
    entries.extend(plugin_kinds.iter().map(|(owner, k)| (owner.clone(), to_cfg(k, owner), k.role)));
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

    /// An index of `owner`'s `kinds` (short names) on bits 0, 1, ... (at most
    /// [`MAX_MARKER_KINDS`]), for hosts that run a plugin's transforms outside
    /// a workspace and report its marks by name (the plugin host process).
    pub fn of_kinds(owner: &str, kinds: &[String]) -> MarkIndex {
        MarkIndex {
            kinds: kinds
                .iter()
                .take(MAX_MARKER_KINDS)
                .enumerate()
                .map(|(i, k)| (format!("{owner}/{k}"), owner.to_string(), i as u32))
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

    /// The bit of `kind` if it is one of this plugin's kinds (nothing is recorded).
    pub fn bit_of(&self, marks: &MarkIndex, kind: &str) -> Option<MarkerMask> {
        marks.bit(self.owner, kind).map(|b| 1 << b)
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
