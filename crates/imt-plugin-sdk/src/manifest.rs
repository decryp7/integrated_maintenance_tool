//! The handshake reply, parsed and checked once: what a plugin process is, needs and can do.

use crate::convert::{condition_from, permission_from, PermissionRequest};
use crate::pb;
use anyhow::{anyhow, bail, Result};
use imt_core::condition::ConditionCfg;
use std::collections::HashSet;

/// An importer a plugin offers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImporterCap {
    pub id: String,
    pub binary: bool,
    pub tail: bool,
    pub multiline: bool,
    /// Fields that identify who wrote an event (ADR 0020), at most [`MAX_IDENTITY_FIELDS`] valid names.
    pub identity_fields: Vec<String>,
}

/// Most identity fields an importer may declare.
pub const MAX_IDENTITY_FIELDS: usize = 8;
/// Longest identity field name, in characters.
const MAX_IDENTITY_FIELD_LEN: usize = 64;

/// The declared identity fields that are usable names (non-empty, at most 64 characters, no control
/// characters), at most [`MAX_IDENTITY_FIELDS`], without duplicates. Untrusted input: the rest is dropped.
pub fn identity_fields(declared: &[String]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in declared {
        let ok = !name.trim().is_empty() && name.chars().count() <= MAX_IDENTITY_FIELD_LEN && !name.chars().any(char::is_control);
        if ok && !out.contains(name) && out.len() < MAX_IDENTITY_FIELDS {
            out.push(name.clone());
        }
    }
    out
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExporterCap {
    pub id: String,
    pub name: String,
    pub extension: String,
}

#[derive(Debug, Clone)]
pub struct AnnotatorCap {
    pub id: String,
    /// Records IMT sends (all must match; empty: every record).
    pub interest: Vec<ConditionCfg>,
    /// At most this many (capped by IMT at its view limit).
    pub limit: usize,
}

/// Everything a plugin implements; what is absent is simply not called.
#[derive(Debug, Clone, Default)]
pub struct Capabilities {
    pub importers: Vec<ImporterCap>,
    pub transforms: Vec<String>,
    pub exporters: Vec<ExporterCap>,
    pub marker_kinds: bool,
    pub annotators: Vec<AnnotatorCap>,
    pub settings: bool,
    pub services: Vec<(String, String)>,
    pub events: Vec<String>,
}

/// A dependency on another plugin.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DepDecl {
    pub id: String,
    pub req: String,
    pub optional: bool,
}

/// A plugin's manifest, as checked by IMT.
#[derive(Debug, Clone)]
pub struct RemoteManifest {
    pub id: String,
    pub name: String,
    pub version: String,
    pub contract: (u32, u32),
    pub requires: Vec<DepDecl>,
    pub provides: Vec<(String, String)>,
    pub permissions: Vec<PermissionRequest>,
    pub capabilities: Capabilities,
}

fn check_id(what: &str, id: &str) -> Result<()> {
    if id.is_empty() || id.len() > 128 || !id.chars().all(|c| c.is_ascii_alphanumeric() || "._-/".contains(c)) {
        bail!("{what} \"{id}\" is not a valid id (1–128 characters from A–Z, a–z, 0–9, '.', '_', '-', '/')");
    }
    Ok(())
}

fn unique<'a>(what: &str, ids: impl Iterator<Item = &'a str>) -> Result<()> {
    let mut seen = HashSet::new();
    for id in ids {
        check_id(what, id)?;
        if !seen.insert(id) {
            bail!("{what} \"{id}\" is declared twice");
        }
    }
    Ok(())
}

impl RemoteManifest {
    /// Parses and checks a handshake reply against the contract major version IMT speaks.
    pub fn parse(m: &pb::Manifest, major: u32) -> Result<RemoteManifest> {
        check_id("plugin id", &m.id)?;
        if m.contract_major != major {
            bail!(
                "plugin \"{}\" implements imt.plugin.v{} but IMT speaks v{major}; update the plugin",
                m.id,
                m.contract_major
            );
        }
        semver::Version::parse(&m.version).map_err(|e| anyhow!("plugin \"{}\": version \"{}\" is not semver ({e})", m.id, m.version))?;
        let lists = [m.requires.len(), m.provides.len(), m.permissions.len(), m.capabilities.len()];
        if lists.iter().any(|n| *n > crate::convert::MAX_LIST) {
            bail!("plugin \"{}\": its manifest lists too many entries", m.id);
        }
        let mut caps = Capabilities::default();
        for c in &m.capabilities {
            use pb::capability::Kind;
            match &c.kind {
                Some(Kind::Importer(i)) => caps.importers.push(ImporterCap {
                    id: i.id.clone(),
                    binary: i.binary,
                    tail: i.tail && !i.binary,
                    multiline: i.multiline,
                    identity_fields: identity_fields(&i.identity_fields),
                }),
                Some(Kind::Transform(t)) => caps.transforms.push(t.id.clone()),
                Some(Kind::Exporter(e)) => {
                    if e.extension.is_empty() || e.extension.len() > 16 || !e.extension.chars().all(|c| c.is_ascii_alphanumeric()) {
                        bail!("exporter \"{}\": extension \"{}\" is not valid", e.id, e.extension);
                    }
                    caps.exporters.push(ExporterCap {
                        id: e.id.clone(),
                        name: if e.name.is_empty() {
                            e.id.clone()
                        } else {
                            e.name.chars().take(100).collect()
                        },
                        extension: e.extension.clone(),
                    })
                }
                Some(Kind::MarkerKinds(_)) => caps.marker_kinds = true,
                Some(Kind::Annotator(a)) => caps.annotators.push(AnnotatorCap {
                    id: a.id.clone(),
                    interest: a.interest.iter().map(condition_from).collect::<Result<_>>()?,
                    limit: if a.limit == 0 {
                        imt_core::plugin::annotate::MAX_VIEW_RECORDS
                    } else {
                        (a.limit as usize).min(imt_core::plugin::annotate::MAX_VIEW_RECORDS)
                    },
                }),
                Some(Kind::Settings(_)) => caps.settings = true,
                Some(Kind::Service(s)) => caps.services.push((s.id.clone(), s.version.clone())),
                Some(Kind::Events(e)) => caps.events.extend(e.kinds.iter().take(64).cloned()),
                // A capability of a newer minor version: tolerated, never called.
                None => {}
            }
        }
        unique("importer", caps.importers.iter().map(|i| i.id.as_str()))?;
        unique("transform", caps.transforms.iter().map(String::as_str))?;
        unique("exporter", caps.exporters.iter().map(|e| e.id.as_str()))?;
        unique("annotator", caps.annotators.iter().map(|a| a.id.as_str()))?;
        unique("service", caps.services.iter().map(|s| s.0.as_str()))?;
        let requires = m
            .requires
            .iter()
            .map(|d| {
                check_id("dependency", &d.id)?;
                Ok(DepDecl {
                    id: d.id.clone(),
                    req: d.req.clone(),
                    optional: d.optional,
                })
            })
            .collect::<Result<_>>()?;
        let provides = m.provides.iter().map(|s| (s.id.clone(), s.version.clone())).collect();
        let permissions = m.permissions.iter().map(permission_from).collect::<Result<_>>()?;
        Ok(RemoteManifest {
            id: m.id.clone(),
            name: if m.name.trim().is_empty() {
                m.id.clone()
            } else {
                m.name.chars().take(100).collect()
            },
            version: m.version.clone(),
            contract: (m.contract_major, m.contract_minor),
            requires,
            provides,
            permissions,
            capabilities: caps,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pb::capability::Kind;

    fn manifest(caps: Vec<Kind>) -> pb::Manifest {
        pb::Manifest {
            id: "a.b".into(),
            name: String::new(),
            version: "1.0.0".into(),
            contract_major: 1,
            contract_minor: 3,
            requires: vec![pb::Dependency {
                id: "c.d".into(),
                req: "^1".into(),
                optional: true,
            }],
            provides: vec![pb::ServiceDecl {
                id: "s".into(),
                version: "1.0.0".into(),
            }],
            permissions: Vec::new(),
            capabilities: caps.into_iter().map(|k| pb::Capability { kind: Some(k) }).collect(),
        }
    }

    #[test]
    fn identity_fields_are_capped() {
        let many: Vec<String> = (0..20).map(|i| format!("F{i}")).collect();
        assert_eq!(identity_fields(&many).len(), MAX_IDENTITY_FIELDS);
        assert!(identity_fields(&[" ".into()]).is_empty());
    }

    #[test]
    fn a_full_manifest_parses() {
        let mut m = manifest(vec![
            Kind::Importer(pb::ImporterCap {
                id: "bin".into(),
                binary: true,
                tail: true,
                multiline: false,
                identity_fields: vec![
                    "Computer".into(),
                    String::new(),
                    "Computer".into(),
                    char::from(7).to_string(),
                    "x".repeat(65),
                    "ProcessId".into(),
                ],
            }),
            Kind::Exporter(pb::ExporterCap {
                id: "x".into(),
                name: String::new(),
                extension: "txt".into(),
            }),
            Kind::Annotator(pb::AnnotatorCap {
                id: "a0".into(),
                interest: Vec::new(),
                limit: 0,
            }),
            Kind::Annotator(pb::AnnotatorCap {
                id: "a1".into(),
                interest: Vec::new(),
                limit: 5,
            }),
            Kind::Annotator(pb::AnnotatorCap {
                id: "a2".into(),
                interest: Vec::new(),
                limit: u32::MAX,
            }),
            Kind::Settings(pb::SettingsCap {}),
            Kind::Service(pb::ServiceCap {
                id: "s".into(),
                version: "1.0.0".into(),
            }),
            Kind::Events(pb::EventsCap {
                kinds: vec!["recordsAppended".into()],
            }),
        ]);
        m.capabilities.push(pb::Capability { kind: None });
        let r = RemoteManifest::parse(&m, 1).unwrap();
        assert_eq!(r.name, "a.b", "no name: the id");
        assert_eq!(r.contract, (1, 3));
        assert!(!r.capabilities.importers[0].tail, "binary importers have no tail");
        assert_eq!(
            r.capabilities.importers[0].identity_fields,
            ["Computer", "ProcessId"],
            "invalid, duplicate and over-long names are dropped"
        );
        assert_eq!(r.capabilities.exporters[0].name, "x");
        let limits: Vec<usize> = r.capabilities.annotators.iter().map(|a| a.limit).collect();
        let max = imt_core::plugin::annotate::MAX_VIEW_RECORDS;
        assert_eq!(limits, [max, 5, max]);
        assert!(r.capabilities.settings);
        assert_eq!(r.capabilities.services.len(), 1);
        assert_eq!(r.capabilities.events, ["recordsAppended"]);
        assert_eq!(
            r.requires[0],
            DepDecl {
                id: "c.d".into(),
                req: "^1".into(),
                optional: true
            }
        );
        assert_eq!(r.provides, [("s".to_string(), "1.0.0".to_string())]);
    }

    #[test]
    fn bad_manifests_are_refused_with_the_reason() {
        let check = |m: pb::Manifest, want: &str| {
            let e = RemoteManifest::parse(&m, 1).err().unwrap().to_string();
            assert!(e.contains(want), "{e} should mention {want}");
        };
        let mut m = manifest(vec![]);
        m.id = "bad id!".into();
        check(m, "not a valid id");
        let mut m = manifest(vec![]);
        m.contract_major = 2;
        check(m, "imt.plugin.v2");
        let mut m = manifest(vec![]);
        m.version = "one".into();
        check(m, "not semver");
        let mut m = manifest(vec![]);
        m.capabilities = vec![pb::Capability { kind: None }; crate::convert::MAX_LIST + 1];
        check(m, "too many");
        let ext = |e: &str| {
            manifest(vec![Kind::Exporter(pb::ExporterCap {
                id: "x".into(),
                name: "X".into(),
                extension: e.into(),
            })])
        };
        check(ext(""), "extension");
        check(ext("../x"), "extension");
        let imp = |id: &str| {
            Kind::Importer(pb::ImporterCap {
                id: id.into(),
                ..Default::default()
            })
        };
        check(manifest(vec![imp("i"), imp("i")]), "declared twice");
        check(manifest(vec![imp("")]), "not a valid id");
        let mut m = manifest(vec![]);
        m.requires[0].id = String::new();
        check(m, "dependency");
        let mut m = manifest(vec![]);
        m.permissions = vec![pb::Permission::default()];
        check(m, "permission");
        let bad_interest = manifest(vec![Kind::Annotator(pb::AnnotatorCap {
            id: "a".into(),
            interest: vec![pb::Condition {
                field: "f".into(),
                op: "eq".into(),
                value_json: "{".into(),
            }],
            limit: 1,
        })]);
        check(bad_interest, "JSON");
    }
}
