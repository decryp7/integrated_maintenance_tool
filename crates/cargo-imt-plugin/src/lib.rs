//! `cargo imt-plugin`: the plugin author's tool (ADR 0027).
//!
//! - `new <name> --kind importer|transform|tool|collector|annotator` ([`templates`]);
//! - `test`: builds the plugin, runs the harness's conformance checks against its program in IMT's sandbox, then
//!   `cargo test`;
//! - `package [--sign <key file>] [--target <triple>]...`: builds release programs and writes a `.imtplugin`
//!   ([`pack`]);
//! - `keygen --out <file>`: a publisher key pair; `inspect <file.imtplugin>`: what a package holds.

#![deny(missing_docs)]

pub mod templates;

use anyhow::{anyhow, bail, Context, Result};
use imt_plugin_api::manifest::Manifest;
use imt_plugin_package::archive::{build, Build};
use imt_plugin_package::manifest::{host_target, PackageManifest};
use imt_plugin_package::SecretKey;
use serde::Deserialize;
use std::path::{Path, PathBuf};

/// What `Cargo.toml` says about the plugin (`[package]` and `[package.metadata.imt-plugin]`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginMeta {
    /// The crate name, also the program's name.
    pub name: String,
    /// The crate version, also the plugin's.
    pub version: semver::Version,
    /// The crate description.
    pub description: String,
    /// The plugin id (`metadata.imt-plugin.id`).
    pub id: String,
    /// The publisher shown to users (`metadata.imt-plugin.publisher`).
    pub publisher: String,
    /// The display name (`metadata.imt-plugin.name`, else the plugin's own name when packaged).
    pub display_name: Option<String>,
}

#[derive(Deserialize)]
struct CargoToml {
    package: CargoPackage,
}

#[derive(Deserialize)]
struct CargoPackage {
    name: String,
    version: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    metadata: Option<CargoMetadata>,
}

#[derive(Deserialize)]
struct CargoMetadata {
    #[serde(rename = "imt-plugin")]
    imt_plugin: Option<PluginTable>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PluginTable {
    id: String,
    publisher: String,
    #[serde(default)]
    name: Option<String>,
}

impl PluginMeta {
    /// Reads `Cargo.toml` (with explicit `name`, `version` and a `[package.metadata.imt-plugin]` table).
    pub fn parse(text: &str) -> Result<PluginMeta> {
        let c: CargoToml = toml::from_str(text).map_err(|e| anyhow!("Cargo.toml: {}", e.message()))?;
        let table = c
            .package
            .metadata
            .and_then(|m| m.imt_plugin)
            .context("Cargo.toml has no [package.metadata.imt-plugin] table with id and publisher")?;
        Ok(PluginMeta {
            version: semver::Version::parse(&c.package.version)
                .with_context(|| format!("Cargo.toml: version \"{}\" is not semver", c.package.version))?,
            name: c.package.name,
            description: c.package.description,
            id: table.id,
            publisher: table.publisher,
            display_name: table.name,
        })
    }

    /// Reads the `Cargo.toml` at `path`.
    pub fn read(path: &Path) -> Result<PluginMeta> {
        PluginMeta::parse(&std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?)
    }
}

/// The program's own manifest: it is started under the harness (contained, as IMT would) and asked.
pub fn manifest_of(program: &Path) -> Result<Manifest> {
    let p = imt_plugin_test::PluginUnderTest::launch(program)?;
    Ok(p.manifest().clone())
}

/// Writes a package for `meta` with `programs` (target triple, program) to `out`, signed with `sign`. The program for
/// this computer (or, if there is none, packaging fails) is asked for its manifest: the package declares exactly the
/// permissions and protocol the program does, and its id and version must match `Cargo.toml`. Returns the digest.
pub fn pack(meta: &PluginMeta, programs: &[(String, PathBuf)], sign: Option<&SecretKey>, out: &Path) -> Result<String> {
    let Some((_, here)) = programs.iter().find(|(t, _)| t == host_target()) else {
        bail!(
            "include a program for this computer ({}): IMT's packager asks it to describe itself",
            host_target()
        );
    };
    let m = manifest_of(here)?;
    if *m.id != meta.id {
        bail!(
            "the program says it is \"{}\" but Cargo.toml's metadata.imt-plugin.id is \"{}\"",
            m.id,
            meta.id
        );
    }
    if *m.version != meta.version.to_string() {
        bail!(
            "the program says it is version {} but Cargo.toml says {}; return env!(\"CARGO_PKG_VERSION\") from Plugin::version",
            m.version,
            meta.version
        );
    }
    let manifest = PackageManifest {
        id: meta.id.clone(),
        name: meta.display_name.clone().unwrap_or_else(|| m.name.to_string()),
        version: meta.version.clone(),
        publisher: meta.publisher.clone(),
        description: meta.description.clone(),
        protocol: m.protocol,
        binary: meta.name.clone(),
        targets: programs.iter().map(|(t, _)| t.clone()).collect(),
        key: None,
        permissions: m.permissions.clone(),
    };
    let programs: Vec<(String, &Path)> = programs.iter().map(|(t, p)| (t.clone(), p.as_path())).collect();
    Ok(build(Build { manifest, programs, sign }, out)?)
}

/// The cargo program to run (`$CARGO`, else `cargo`).
fn cargo() -> std::process::Command {
    // A developer tool on the author's machine: it runs their cargo with their environment.
    #[allow(clippy::disallowed_methods)]
    std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()))
}

fn run(mut cmd: std::process::Command) -> Result<()> {
    let status = cmd.status().with_context(|| format!("cannot run {cmd:?}"))?;
    if !status.success() {
        bail!("{cmd:?} failed ({status})");
    }
    Ok(())
}

/// The target folder of the project at `manifest_path` (from `cargo metadata`).
pub fn target_dir(manifest_path: &Path) -> Result<PathBuf> {
    let mut cmd = cargo();
    cmd.args(["metadata", "--format-version", "1", "--no-deps", "--manifest-path"])
        .arg(manifest_path);
    let out = cmd.output().context("cannot run cargo metadata")?;
    if !out.status.success() {
        bail!("cargo metadata failed: {}", String::from_utf8_lossy(&out.stderr));
    }
    let v: serde_json::Value = serde_json::from_slice(&out.stdout).context("cargo metadata printed something else")?;
    v["target_directory"]
        .as_str()
        .map(PathBuf::from)
        .context("cargo metadata named no target folder")
}

/// Builds the plugin's program (`release` or debug, for `target` or this computer); returns its path.
pub fn build_program(manifest_path: &Path, meta: &PluginMeta, release: bool, target: Option<&str>) -> Result<PathBuf> {
    let mut cmd = cargo();
    cmd.arg("build")
        .arg("--manifest-path")
        .arg(manifest_path)
        .args(["--bin", &meta.name]);
    if release {
        cmd.arg("--release");
    }
    if let Some(t) = target {
        cmd.args(["--target", t]);
    }
    run(cmd)?;
    let mut dir = target_dir(manifest_path)?;
    if let Some(t) = target {
        dir = dir.join(t);
    }
    dir = dir.join(if release { "release" } else { "debug" });
    let windows = target.map_or(cfg!(windows), |t| t.contains("windows"));
    let exe = dir.join(if windows { format!("{}.exe", meta.name) } else { meta.name.clone() });
    if !exe.is_file() {
        bail!("cargo built no program at {}", exe.display());
    }
    Ok(exe)
}

/// The sample files of a project (`tests/samples/*`), for the conformance checks.
pub fn samples(project: &Path) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = std::fs::read_dir(project.join("tests").join("samples"))
        .map(|d| d.flatten().map(|e| e.path()).filter(|p| p.is_file()).collect())
        .unwrap_or_default();
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cargo_metadata_is_read_and_checked() {
        let text = r#"
[package]
name = "kv"
version = "1.2.3"
description = "Reads kv."

[package.metadata.imt-plugin]
id = "com.example.kv"
publisher = "Me"
"#;
        let m = PluginMeta::parse(text).unwrap();
        assert_eq!(
            (m.name.as_str(), m.id.as_str(), m.publisher.as_str()),
            ("kv", "com.example.kv", "Me")
        );
        assert_eq!(m.version, semver::Version::new(1, 2, 3));
        assert!(PluginMeta::parse("[package]\nname = \"kv\"\nversion = \"1.0.0\"\n")
            .unwrap_err()
            .to_string()
            .contains("metadata.imt-plugin"));
        assert!(PluginMeta::parse(&text.replace("1.2.3", "one")).is_err());
        assert!(PluginMeta::parse(&text.replace("publisher = \"Me\"", "publisher = \"Me\"\nextra = 1")).is_err());
        let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples/plugins");
        for kind in templates::Kind::ALL {
            let meta = PluginMeta::read(&examples.join(format!("{}-example/Cargo.toml", kind.name()))).unwrap();
            assert!(meta.id.starts_with("net.decryptology.example."), "{meta:?}");
        }
        assert_eq!(samples(&examples.join("importer-example")).len(), 1);
        assert!(samples(&examples.join("tool-example")).is_empty());
    }

    #[test]
    fn packing_needs_a_program_for_this_computer() {
        let t = tempfile::tempdir().unwrap();
        let meta = PluginMeta::parse(
            "[package]\nname = \"kv\"\nversion = \"1.0.0\"\n[package.metadata.imt-plugin]\nid = \"com.example.kv\"\npublisher = \"Me\"\n",
        )
        .unwrap();
        let e = pack(
            &meta,
            &[("aarch64-unknown-none-x".into(), t.path().join("kv"))],
            None,
            &t.path().join("kv.imtplugin"),
        )
        .unwrap_err();
        assert!(e.to_string().contains(host_target()), "{e}");
    }
}
