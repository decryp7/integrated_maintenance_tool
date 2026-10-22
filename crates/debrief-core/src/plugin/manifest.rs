//! Plugin manifests: who a plugin is, which plugin API it targets, which
//! plugins it needs and which services it provides.
//!
//! A manifest is built with a small builder so that a plugin's `manifest()`
//! stays infallible: text that does not parse (a version or a requirement) is
//! recorded as a problem and [`Manifest::check`] rejects the plugin at install
//! with a message that names the bad value.

use anyhow::{anyhow, Result};
use semver::{Version, VersionReq};
use serde::Serialize;
use std::collections::HashSet;

/// A dependency on another plugin, by plugin id and semver requirement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Dep {
    pub id: String,
    pub req: VersionReq,
    /// An optional dependency only reduces features when it is unavailable.
    pub optional: bool,
}

impl Dep {
    /// The plugin cannot work without `id` in a version matching `req`.
    pub fn required(id: impl Into<String>, req: VersionReq) -> Dep {
        Dep {
            id: id.into(),
            req,
            optional: false,
        }
    }

    /// The plugin uses `id` when it is available and works without it.
    pub fn optional(id: impl Into<String>, req: VersionReq) -> Dep {
        Dep {
            id: id.into(),
            req,
            optional: true,
        }
    }
}

/// A service a plugin provides to others (see `PluginRegistry::provide`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceDecl {
    pub id: String,
    pub version: Version,
}

/// What a plugin declares about itself before it is registered.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub id: String,
    pub version: Version,
    pub api_version: u32,
    pub requires: Vec<Dep>,
    pub provides: Vec<ServiceDecl>,
    /// Values that did not parse; reported by [`Manifest::check`].
    problems: Vec<String>,
}

impl Manifest {
    /// A manifest without dependencies. `version` is semver; a missing minor
    /// or patch number is read as 0 (`"1"` is 1.0.0, `"1.2"` is 1.2.0).
    pub fn new(id: impl Into<String>, version: &str) -> Manifest {
        let id = id.into();
        let (version, problems) = match parse_version(version) {
            Ok(v) => (v, Vec::new()),
            Err(e) => (Version::new(0, 0, 0), vec![format!("version: {e}")]),
        };
        Manifest {
            id,
            version,
            api_version: crate::plugin::API_VERSION,
            requires: Vec::new(),
            provides: Vec::new(),
            problems,
        }
    }

    pub fn api_version(mut self, api_version: u32) -> Self {
        self.api_version = api_version;
        self
    }

    /// Adds a required dependency (`req` like `"^1.0"`, `">=0.3, <0.5"`).
    pub fn requires(self, id: impl Into<String>, req: &str) -> Self {
        self.dep(id.into(), req, false)
    }

    /// Adds an optional dependency.
    pub fn optionally(self, id: impl Into<String>, req: &str) -> Self {
        self.dep(id.into(), req, true)
    }

    /// Adds an already typed dependency.
    pub fn depends(mut self, dep: Dep) -> Self {
        self.requires.push(dep);
        self
    }

    /// Declares a service this plugin provides (`version` is semver).
    pub fn provides(mut self, id: impl Into<String>, version: &str) -> Self {
        let id = id.into();
        match parse_version(version) {
            Ok(version) => self.provides.push(ServiceDecl { id, version }),
            Err(e) => self.problems.push(format!("service \"{id}\" version: {e}")),
        }
        self
    }

    fn dep(mut self, id: String, req: &str, optional: bool) -> Self {
        match VersionReq::parse(req.trim()) {
            Ok(req) => self.requires.push(Dep { id, req, optional }),
            Err(e) => self
                .problems
                .push(format!("dependency \"{id}\": version requirement \"{req}\" is not valid ({e})")),
        }
        self
    }

    /// Rejects unparsable values, empty ids, self-dependencies and duplicates.
    pub fn check(&self) -> Result<()> {
        let mut problems = self.problems.clone();
        if self.id.trim().is_empty() {
            problems.push("the plugin id is empty".into());
        }
        let mut seen = HashSet::new();
        for d in &self.requires {
            if d.id == self.id {
                problems.push("the plugin depends on itself".into());
            } else if d.id.trim().is_empty() {
                problems.push("a dependency has an empty id".into());
            } else if !seen.insert(d.id.as_str()) {
                problems.push(format!("dependency \"{}\" is listed twice", d.id));
            }
        }
        let mut seen = HashSet::new();
        for s in &self.provides {
            if s.id.trim().is_empty() {
                problems.push("a provided service has an empty id".into());
            } else if !seen.insert(s.id.as_str()) {
                problems.push(format!("service \"{}\" is declared twice", s.id));
            }
        }
        if problems.is_empty() {
            Ok(())
        } else {
            Err(anyhow!("invalid manifest: {}", problems.join("; ")))
        }
    }
}

/// Parses a semver version, accepting `"1"` and `"1.2"` (missing parts are 0)
/// and a leading `v`.
pub fn parse_version(text: &str) -> Result<Version> {
    let t = text.trim();
    let t = t.strip_prefix('v').unwrap_or(t);
    if let Ok(v) = Version::parse(t) {
        return Ok(v);
    }
    let core_end = t.find(['-', '+']).unwrap_or(t.len());
    let (core, rest) = t.split_at(core_end);
    let padded = match core.split('.').count() {
        1 => format!("{core}.0.0{rest}"),
        2 => format!("{core}.0{rest}"),
        _ => String::new(),
    };
    Version::parse(&padded).map_err(|_| anyhow!("\"{text}\" is not a semver version (major.minor.patch)"))
}

/// A dependency as listed in [`crate::plugin::PluginInfo`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DepInfo {
    pub id: String,
    /// The semver requirement, e.g. `^1.0`.
    pub req: String,
    pub optional: bool,
}

impl From<&Dep> for DepInfo {
    fn from(d: &Dep) -> Self {
        DepInfo {
            id: d.id.clone(),
            req: d.req.to_string(),
            optional: d.optional,
        }
    }
}

/// A provided service as listed in [`crate::plugin::PluginInfo`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServiceInfo {
    pub id: String,
    pub version: String,
}

impl From<&ServiceDecl> for ServiceInfo {
    fn from(s: &ServiceDecl) -> Self {
        ServiceInfo {
            id: s.id.clone(),
            version: s.version.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versions_are_parsed_leniently() {
        assert_eq!(parse_version("1").unwrap(), Version::new(1, 0, 0));
        assert_eq!(parse_version("1.2").unwrap(), Version::new(1, 2, 0));
        assert_eq!(parse_version(" v1.2.3 ").unwrap(), Version::new(1, 2, 3));
        assert_eq!(parse_version("1.2-beta").unwrap().to_string(), "1.2.0-beta");
        for bad in ["", "x", "1.2.3.4", "1..2"] {
            assert!(parse_version(bad).unwrap_err().to_string().contains("semver"), "{bad}");
        }
    }

    #[test]
    fn builder_collects_problems_for_check() {
        let m = Manifest::new("a", "1.0.0")
            .requires("b", "^1.0")
            .optionally("c", ">=0.3, <0.5")
            .depends(Dep::required("d", VersionReq::STAR))
            .depends(Dep::optional("e", VersionReq::STAR))
            .provides("svc", "2.1")
            .api_version(1);
        m.check().unwrap();
        assert_eq!(m.requires.len(), 4);
        assert!(m.requires[1].optional && !m.requires[2].optional && m.requires[3].optional);
        assert_eq!(m.provides[0].version, Version::new(2, 1, 0));
        assert_eq!(DepInfo::from(&m.requires[0]).req, "^1.0");
        assert_eq!(ServiceInfo::from(&m.provides[0]).version, "2.1.0");

        let bad = Manifest::new(" ", "nope")
            .requires("b", "not a req")
            .requires("", "*")
            .requires("", "*")
            .provides("s", "x")
            .provides("t", "1")
            .provides("t", "1")
            .provides("", "1")
            .requires("c", "*")
            .requires("c", "*");
        let e = bad.check().unwrap_err().to_string();
        for part in [
            "version: \"nope\"",
            "\"b\": version requirement \"not a req\"",
            "service \"s\" version",
            "plugin id is empty",
            "dependency has an empty id",
            "\"c\" is listed twice",
            "\"t\" is declared twice",
            "provided service has an empty id",
        ] {
            assert!(e.contains(part), "{part}: {e}");
        }
        let own = Manifest::new("a", "1").requires("a", "*").check().unwrap_err();
        assert!(own.to_string().contains("depends on itself"));
    }
}
