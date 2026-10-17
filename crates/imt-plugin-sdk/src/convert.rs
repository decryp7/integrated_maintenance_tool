//! Conversions between the `imt.plugin.v1` messages and IMT's Rust types. Untrusted input (anything a
//! plugin or the host sends) is parsed here once, into typed values, with errors that name the field.

use crate::pb;
use anyhow::{anyhow, bail, Context, Result};
use imt_core::api::Severity;
use imt_core::condition::{CondValue, ConditionCfg};
use imt_core::plugin::annotate::{Annotation, FieldMatch};
use imt_core::plugin::markers::{MarkerRole, PluginMarkerKind};
use imt_core::settings::{EnumOption, FieldKind, SectionOwner, SettingField, SettingsSection};
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Most capabilities, dependencies, permissions, marker kinds or settings fields one manifest or reply
/// may list.
pub const MAX_LIST: usize = 1000;
/// Longest JSON text accepted in a control message (conditions, defaults, queries).
pub const MAX_JSON: usize = 1 << 20;

fn bounded<T>(what: &str, items: &[T]) -> Result<()> {
    if items.len() > MAX_LIST {
        bail!("{what}: more than {MAX_LIST} entries");
    }
    Ok(())
}

fn json<T: serde::de::DeserializeOwned>(what: &str, text: &str) -> Result<T> {
    if text.len() > MAX_JSON {
        bail!("{what}: longer than {MAX_JSON} bytes");
    }
    serde_json::from_str(text).with_context(|| format!("{what} is not valid JSON"))
}

/// Something a plugin may do beyond reading the bytes IMT gives it. Granted by the user per plugin.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Permission {
    /// Receive the paths of files the user opened (and read them directly).
    ReadOpenedFiles,
    /// Connect to `host` (empty: any host).
    Network { host: String },
}

impl Permission {
    /// A stable key (`read-opened-files`, `network:example.com`), used to store grants.
    pub fn key(&self) -> String {
        match self {
            Permission::ReadOpenedFiles => "read-opened-files".into(),
            Permission::Network { host } if host.is_empty() => "network:*".into(),
            Permission::Network { host } => format!("network:{host}"),
        }
    }

    /// How the user sees it.
    pub fn describe(&self) -> String {
        match self {
            Permission::ReadOpenedFiles => "Read the files you open (it receives their paths)".into(),
            Permission::Network { host } if host.is_empty() => "Connect to any server on the network".into(),
            Permission::Network { host } => format!("Connect to {host}"),
        }
    }
}

/// A permission a plugin asks for, with its reason.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PermissionRequest {
    pub permission: Permission,
    pub reason: String,
}

pub fn permission_from(p: &pb::Permission) -> Result<PermissionRequest> {
    let permission = match pb::PermissionKind::try_from(p.kind) {
        Ok(pb::PermissionKind::ReadOpenedFiles) => Permission::ReadOpenedFiles,
        Ok(pb::PermissionKind::Network) => {
            if p.target.len() > 253 || !p.target.chars().all(|c| c.is_ascii_alphanumeric() || ".-*".contains(c)) {
                bail!("permission: \"{}\" is not a host name", p.target);
            }
            Permission::Network {
                host: p.target.trim_start_matches('*').trim_start_matches('.').to_string(),
            }
        }
        _ => bail!("permission: unknown kind {}", p.kind),
    };
    Ok(PermissionRequest {
        permission,
        reason: p.reason.chars().take(500).collect(),
    })
}

pub fn permission_to(p: &PermissionRequest) -> pb::Permission {
    match &p.permission {
        Permission::ReadOpenedFiles => pb::Permission {
            kind: pb::PermissionKind::ReadOpenedFiles as i32,
            target: String::new(),
            reason: p.reason.clone(),
        },
        Permission::Network { host } => pb::Permission {
            kind: pb::PermissionKind::Network as i32,
            target: host.clone(),
            reason: p.reason.clone(),
        },
    }
}

// ---- Conditions ----

pub fn condition_to(c: &ConditionCfg) -> pb::Condition {
    pb::Condition {
        field: c.field.clone(),
        op: c.op.clone(),
        value_json: c.value.as_ref().and_then(|v| serde_json::to_string(v).ok()).unwrap_or_default(),
    }
}

pub fn condition_from(c: &pb::Condition) -> Result<ConditionCfg> {
    let value = if c.value_json.is_empty() {
        None
    } else {
        Some(json::<CondValue>("condition value", &c.value_json)?)
    };
    Ok(ConditionCfg {
        field: c.field.clone(),
        op: c.op.clone(),
        value,
    })
}

// ---- Marker kinds ----

pub fn marker_kind_to(k: &PluginMarkerKind) -> pb::MarkerKind {
    pb::MarkerKind {
        kind: k.kind.clone(),
        label: k.label.clone(),
        description: k.description.clone().unwrap_or_default(),
        color: k.color.clone().unwrap_or_default(),
        icon: k.icon.clone().unwrap_or_default(),
        badge: k.badge.clone().unwrap_or_default(),
        shape: k.shape.clone().unwrap_or_default(),
        role: match k.role {
            None => pb::MarkerRole::RoleNone,
            Some(MarkerRole::Start) => pb::MarkerRole::RoleStart,
            Some(MarkerRole::Stop) => pb::MarkerRole::RoleStop,
            Some(MarkerRole::Crash) => pb::MarkerRole::RoleCrash,
        } as i32,
        when: k.when.iter().map(condition_to).collect(),
    }
}

fn non_empty(s: &str) -> Option<String> {
    (!s.is_empty()).then(|| s.to_string())
}

/// A marker kind from a plugin (the registry checks its name and rules when it is added).
pub fn marker_kind_from(k: &pb::MarkerKind) -> Result<PluginMarkerKind> {
    bounded("marker kind conditions", &k.when)?;
    let mut out = PluginMarkerKind::new(&k.kind, &k.label);
    out.description = non_empty(&k.description);
    out.color = non_empty(&k.color);
    out.icon = non_empty(&k.icon);
    out.badge = non_empty(&k.badge);
    out.shape = non_empty(&k.shape);
    out.role = match pb::MarkerRole::try_from(k.role) {
        Ok(pb::MarkerRole::RoleStart) => Some(MarkerRole::Start),
        Ok(pb::MarkerRole::RoleStop) => Some(MarkerRole::Stop),
        Ok(pb::MarkerRole::RoleCrash) => Some(MarkerRole::Crash),
        // Unknown roles of a newer minor version are tolerated as "no role".
        _ => None,
    };
    out.when = k.when.iter().map(condition_from).collect::<Result<_>>()?;
    Ok(out)
}

// ---- Settings ----

fn kind_to(k: &FieldKind) -> pb::FieldKind {
    use pb::field_kind::Kind;
    let kind = match k {
        FieldKind::Bool => Kind::Bool(pb::BoolKind {}),
        FieldKind::Int { min, max, step, unit } => Kind::Int(pb::IntKind {
            min: *min,
            max: *max,
            step: *step,
            unit: unit.clone().unwrap_or_default(),
        }),
        FieldKind::Float { min, max, step, unit } => Kind::Float(pb::FloatKind {
            min: *min,
            max: *max,
            step: *step,
            unit: unit.clone().unwrap_or_default(),
        }),
        FieldKind::Text { max_len, pattern } => Kind::Text(pb::TextKind {
            max_len: *max_len as u64,
            pattern: pattern.clone().unwrap_or_default(),
        }),
        FieldKind::Enum { options } => Kind::Enum(pb::EnumKind {
            options: options
                .iter()
                .map(|o| pb::EnumOption {
                    value: o.value.clone(),
                    label: o.label.clone(),
                })
                .collect(),
        }),
        FieldKind::Path { dir } => Kind::Path(pb::PathKind { dir: *dir }),
        FieldKind::Duration { min_ms, max_ms } => Kind::Duration(pb::DurationKind {
            min_ms: *min_ms,
            max_ms: *max_ms,
        }),
        FieldKind::Size { min_bytes, max_bytes } => Kind::Size(pb::SizeKind {
            min_bytes: *min_bytes,
            max_bytes: *max_bytes,
        }),
        FieldKind::StringList { max_items } => Kind::StringList(pb::StringListKind {
            max_items: *max_items as u64,
        }),
    };
    pb::FieldKind { kind: Some(kind) }
}

fn kind_from(k: Option<&pb::FieldKind>) -> Result<FieldKind> {
    use pb::field_kind::Kind;
    let usize_of = |v: u64| usize::try_from(v).unwrap_or(usize::MAX);
    Ok(match k.and_then(|k| k.kind.as_ref()) {
        Some(Kind::Bool(_)) => FieldKind::Bool,
        Some(Kind::Int(i)) => FieldKind::Int {
            min: i.min,
            max: i.max,
            step: i.step,
            unit: non_empty(&i.unit),
        },
        Some(Kind::Float(f)) => FieldKind::Float {
            min: f.min,
            max: f.max,
            step: f.step,
            unit: non_empty(&f.unit),
        },
        Some(Kind::Text(t)) => FieldKind::Text {
            max_len: usize_of(t.max_len),
            pattern: non_empty(&t.pattern),
        },
        Some(Kind::Enum(e)) => {
            bounded("enum options", &e.options)?;
            FieldKind::Enum {
                options: e.options.iter().map(|o| EnumOption::new(&o.value, &o.label)).collect(),
            }
        }
        Some(Kind::Path(p)) => FieldKind::Path { dir: p.dir },
        Some(Kind::Duration(d)) => FieldKind::Duration {
            min_ms: d.min_ms,
            max_ms: d.max_ms,
        },
        Some(Kind::Size(s)) => FieldKind::Size {
            min_bytes: s.min_bytes,
            max_bytes: s.max_bytes,
        },
        Some(Kind::StringList(l)) => FieldKind::StringList {
            max_items: usize_of(l.max_items),
        },
        None => bail!("a settings field has no (or an unknown) kind"),
    })
}

pub fn section_to(s: &SettingsSection) -> pb::SettingsSection {
    pb::SettingsSection {
        title: s.title.clone(),
        description: s.description.clone(),
        fields: s
            .fields
            .iter()
            .map(|f| pb::SettingField {
                key: f.key.clone(),
                label: f.label.clone(),
                description: f.description.clone(),
                kind: Some(kind_to(&f.kind)),
                default_json: f.default.to_string(),
                restart_required: f.restart_required,
                advanced: f.advanced,
            })
            .collect(),
    }
}

/// The section of plugin `id`, checked like an in-process plugin's.
pub fn section_from(id: &str, s: &pb::SettingsSection) -> Result<SettingsSection> {
    bounded("settings fields", &s.fields)?;
    let mut out = SettingsSection::new(id, &s.title, SectionOwner::Plugin { id: id.into() }).describe(&s.description);
    for f in &s.fields {
        let default: Value = json(&format!("default of setting \"{}\"", f.key), &f.default_json)?;
        let mut field = SettingField::new(&f.key, &f.label, kind_from(f.kind.as_ref())?, default).describe(&f.description);
        if f.restart_required {
            field = field.needs_restart();
        }
        if f.advanced {
            field = field.as_advanced();
        }
        out = out.field(field);
    }
    out.check().map_err(|e| anyhow!("invalid settings schema: {e}"))?;
    Ok(out)
}

// ---- Annotations ----

pub fn annotation_to(a: &Annotation) -> pb::Annotation {
    pb::Annotation {
        kind: a.kind.clone(),
        from: a.from,
        to: a.to,
        instance: a.instance.clone().unwrap_or_default(),
        on_field: a.on.as_ref().map(|o| o.field.clone()).unwrap_or_default(),
        on_value: a.on.as_ref().map(|o| o.value.clone()).unwrap_or_default(),
        label: a.label.clone(),
        severity: match a.severity {
            Severity::Info => pb::Severity::Info,
            Severity::Warning => pb::Severity::Warning,
            Severity::Critical => pb::Severity::Critical,
        } as i32,
        evidence_query_json: serde_json::to_string(&a.evidence_query).unwrap_or_default(),
    }
}

pub fn annotation_from(a: &pb::Annotation) -> Result<Annotation> {
    let evidence_query = if a.evidence_query_json.is_empty() {
        Default::default()
    } else {
        json("evidence query", &a.evidence_query_json)?
    };
    Ok(Annotation {
        kind: a.kind.clone(),
        from: a.from,
        to: a.to,
        instance: non_empty(&a.instance),
        on: (!a.on_field.is_empty()).then(|| FieldMatch {
            field: a.on_field.clone(),
            value: a.on_value.clone(),
        }),
        label: a.label.clone(),
        severity: match pb::Severity::try_from(a.severity) {
            Ok(pb::Severity::Critical) => Severity::Critical,
            Ok(pb::Severity::Warning) => Severity::Warning,
            _ => Severity::Info,
        },
        evidence_query,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::query::Query;
    use serde_json::json;

    #[test]
    fn permissions_round_trip_and_reject_bad_hosts() {
        for p in [
            PermissionRequest {
                permission: Permission::ReadOpenedFiles,
                reason: "r".into(),
            },
            PermissionRequest {
                permission: Permission::Network {
                    host: "example.com".into(),
                },
                reason: String::new(),
            },
        ] {
            assert_eq!(permission_from(&permission_to(&p)).unwrap(), p);
        }
        let bad = pb::Permission {
            kind: pb::PermissionKind::Network as i32,
            target: "evil host/x".into(),
            reason: String::new(),
        };
        assert!(permission_from(&bad).is_err());
        assert!(permission_from(&pb::Permission::default()).is_err());
        assert_eq!(Permission::ReadOpenedFiles.key(), "read-opened-files");
        assert_eq!(Permission::Network { host: String::new() }.key(), "network:*");
        assert_eq!(Permission::Network { host: "a.b".into() }.key(), "network:a.b");
        assert!(Permission::Network { host: String::new() }.describe().contains("any server"));
        assert!(Permission::Network { host: "a.b".into() }.describe().contains("a.b"));
        assert!(Permission::ReadOpenedFiles.describe().contains("files"));
        let wildcard = pb::Permission {
            kind: pb::PermissionKind::Network as i32,
            target: "*.example.com".into(),
            reason: String::new(),
        };
        assert_eq!(
            permission_from(&wildcard).unwrap().permission,
            Permission::Network {
                host: "example.com".into()
            }
        );
    }

    #[test]
    fn marker_kinds_round_trip() {
        for role in [None, Some(MarkerRole::Start), Some(MarkerRole::Stop), Some(MarkerRole::Crash)] {
            let mut k = PluginMarkerKind::new("k", "K")
                .describe("d")
                .color("#fff")
                .icon("bolt")
                .badge("X", "circle")
                .when("EventId", "eq", "41")
                .when_in("Provider", &["a", "b"]);
            k.role = role;
            let back = marker_kind_from(&marker_kind_to(&k)).unwrap();
            assert_eq!(format!("{back:?}"), format!("{k:?}"));
        }
        let plain = marker_kind_from(&marker_kind_to(&PluginMarkerKind::new("k", "K"))).unwrap();
        assert!(plain.color.is_none() && plain.when.is_empty());
        let bad = pb::MarkerKind {
            when: vec![pb::Condition {
                field: "f".into(),
                op: "eq".into(),
                value_json: "{".into(),
            }],
            ..Default::default()
        };
        assert!(marker_kind_from(&bad).is_err());
        let too_many = pb::MarkerKind {
            when: vec![pb::Condition::default(); MAX_LIST + 1],
            ..Default::default()
        };
        assert!(marker_kind_from(&too_many).is_err());
        assert!(json::<Value>("x", &" ".repeat(MAX_JSON + 1)).is_err());
    }

    #[test]
    fn settings_sections_round_trip_every_kind() {
        let s = SettingsSection::new("p", "Title", SectionOwner::Plugin { id: "p".into() })
            .describe("desc")
            .field(SettingField::new("b", "B", FieldKind::Bool, true).needs_restart())
            .field(
                SettingField::new(
                    "i",
                    "I",
                    FieldKind::Int {
                        min: 0,
                        max: 10,
                        step: 1,
                        unit: Some("x".into()),
                    },
                    3,
                )
                .as_advanced(),
            )
            .field(SettingField::new(
                "f",
                "F",
                FieldKind::Float {
                    min: 0.0,
                    max: 1.0,
                    step: 0.1,
                    unit: None,
                },
                0.5,
            ))
            .field(SettingField::new(
                "t",
                "T",
                FieldKind::Text {
                    max_len: 10,
                    pattern: Some("[a-z]*".into()),
                },
                "ab",
            ))
            .field(SettingField::new(
                "e",
                "E",
                FieldKind::Enum {
                    options: vec![EnumOption::new("a", "A"), EnumOption::new("b", "B")],
                },
                "a",
            ))
            .field(SettingField::new("p", "P", FieldKind::Path { dir: true }, ""))
            .field(SettingField::new("d", "D", FieldKind::Duration { min_ms: 1, max_ms: 1000 }, 10))
            .field(SettingField::new(
                "s",
                "S",
                FieldKind::Size {
                    min_bytes: 1,
                    max_bytes: 100,
                },
                10,
            ))
            .field(SettingField::new("l", "L", FieldKind::StringList { max_items: 3 }, json!(["x"])));
        let back = section_from("p", &section_to(&s)).unwrap();
        assert_eq!(back, s);
        let mut bad = section_to(&s);
        bad.fields[0].kind = None;
        assert!(section_from("p", &bad).is_err());
        let mut bad = section_to(&s);
        bad.fields[0].default_json = "nope".into();
        assert!(section_from("p", &bad).is_err());
        // The schema is checked like an in-process plugin's.
        let mut bad = section_to(&s);
        bad.fields[1].key = "b".into();
        assert!(section_from("p", &bad).unwrap_err().to_string().contains("invalid settings schema"));
    }

    #[test]
    fn annotations_round_trip() {
        for severity in [Severity::Info, Severity::Warning, Severity::Critical] {
            let a = Annotation {
                kind: "k".into(),
                from: 1,
                to: 2,
                instance: Some("i".into()),
                on: Some(FieldMatch {
                    field: "Computer".into(),
                    value: "W".into(),
                }),
                label: "L".into(),
                severity,
                evidence_query: Query {
                    from: Some(1),
                    ..Query::default()
                },
            };
            let back = annotation_from(&annotation_to(&a)).unwrap();
            assert_eq!(format!("{back:?}"), format!("{a:?}"));
        }
        let plain = annotation_from(&pb::Annotation::default()).unwrap();
        assert!(plain.instance.is_none() && plain.on.is_none());
        let bad = pb::Annotation {
            evidence_query_json: "[".into(),
            ..Default::default()
        };
        assert!(annotation_from(&bad).is_err());
    }

    #[test]
    fn conditions_without_values() {
        let c = ConditionCfg {
            field: "@level".into(),
            op: "exists".into(),
            value: None,
        };
        let back = condition_from(&condition_to(&c)).unwrap();
        assert!(back.value.is_none() && back.op == "exists");
    }
}
