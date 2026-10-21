//! `cargo imt-plugin <command>`: see the crate docs (`cargo imt-plugin help`).

use anyhow::{bail, Context, Result};
use cargo_imt_plugin::templates::{self, Kind, NewProject, Sdk};
use cargo_imt_plugin::{build_program, pack, samples, PluginMeta};
use imt_plugin_package::manifest::host_target;
use imt_plugin_package::{Package, SecretKey, TrustSettings};
use std::path::{Path, PathBuf};

const HELP: &str = "cargo imt-plugin — build IMT plugins

USAGE:
    cargo imt-plugin new <name> --kind <importer|transform|tool|collector|annotator>
                         [--id <com.example.name>] [--dir <folder>] [--sdk-path <IMT checkout>]
                         [--sdk-git <url>] [--sdk-tag <tag> | --sdk-rev <rev>]
    cargo imt-plugin test [--manifest-path <Cargo.toml>]
    cargo imt-plugin package [--manifest-path <Cargo.toml>] [--sign <secret key file>]
                             [--target <triple>]... [--out <file.imtplugin>]
    cargo imt-plugin keygen --out <secret key file>
    cargo imt-plugin inspect <file.imtplugin>

new      a plugin project from IMT's example of that kind (it builds and passes the harness as it is)
test     builds the plugin, runs IMT's conformance checks against its program in IMT's sandbox, then cargo test
package  builds release programs (this computer's, plus each --target) and writes a .imtplugin, signed with --sign
keygen   a publisher key pair: keep the secret key file private; give users the public key it prints
inspect  what a package holds and whether its signature is valid
";

/// The value after flag `name` (`--name value`), removed from `args`.
fn flag(args: &mut Vec<String>, name: &str) -> Result<Option<String>> {
    let Some(i) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    if i + 1 >= args.len() {
        bail!("{name} needs a value");
    }
    let v = args.remove(i + 1);
    args.remove(i);
    Ok(Some(v))
}

/// Every value of a repeatable flag.
fn flags(args: &mut Vec<String>, name: &str) -> Result<Vec<String>> {
    let mut out = Vec::new();
    while let Some(v) = flag(args, name)? {
        out.push(v);
    }
    Ok(out)
}

fn manifest_path(args: &mut Vec<String>) -> Result<PathBuf> {
    let p = PathBuf::from(flag(args, "--manifest-path")?.unwrap_or_else(|| "Cargo.toml".into()));
    if !p.is_file() {
        bail!("no {} here; run in the plugin's folder or pass --manifest-path", p.display());
    }
    Ok(p)
}

fn no_more(args: &[String]) -> Result<()> {
    if let Some(a) = args.first() {
        bail!("unexpected argument \"{a}\" (see cargo imt-plugin help)");
    }
    Ok(())
}

fn new(mut args: Vec<String>) -> Result<()> {
    let kind = Kind::parse(&flag(&mut args, "--kind")?.context("--kind is required")?)?;
    let dir = PathBuf::from(flag(&mut args, "--dir")?.unwrap_or_else(|| ".".into()));
    let id = flag(&mut args, "--id")?;
    let sdk = match flag(&mut args, "--sdk-path")? {
        Some(p) => {
            let full = std::fs::canonicalize(&p).with_context(|| format!("no IMT checkout at {p}"))?;
            // Cargo does not read Windows' verbatim `\\?\` paths.
            let text = full.to_string_lossy().into_owned();
            Sdk::Path(PathBuf::from(text.strip_prefix(r"\\?\").unwrap_or(&text)))
        }
        None => {
            let url = flag(&mut args, "--sdk-git")?.unwrap_or_else(|| templates::REPOSITORY.into());
            let pin = match (flag(&mut args, "--sdk-tag")?, flag(&mut args, "--sdk-rev")?) {
                (Some(t), None) => Some(("tag".to_string(), t)),
                (None, Some(r)) => Some(("rev".to_string(), r)),
                (None, None) => None,
                (Some(_), Some(_)) => bail!("pass --sdk-tag or --sdk-rev, not both"),
            };
            Sdk::Git { url, pin }
        }
    };
    let name = if args.is_empty() {
        bail!("name the plugin: cargo imt-plugin new <name> --kind <kind>")
    } else {
        args.remove(0)
    };
    no_more(&args)?;
    let id = id.unwrap_or_else(|| format!("com.example.{name}"));
    let root = templates::create(&NewProject { name, id, kind, sdk }, &dir)?;
    println!(
        "Created {} ({} plugin). Next: cd {} && cargo imt-plugin test",
        root.display(),
        kind.name(),
        root.display()
    );
    Ok(())
}

fn test(mut args: Vec<String>) -> Result<()> {
    let manifest = manifest_path(&mut args)?;
    no_more(&args)?;
    let meta = PluginMeta::read(&manifest)?;
    let exe = build_program(&manifest, &meta, false, None)?;
    let project = manifest.parent().map(Path::to_path_buf).unwrap_or_default();
    let plugin = imt_plugin_test::PluginUnderTest::launch(&exe)?;
    let report = plugin.conformance_with(&samples(&project));
    print!("{report}");
    drop(plugin);
    if !report.passed() {
        bail!("{} conformance check(s) failed", report.failures().len());
    }
    // A cargo of the author's: the plugin's own tests.
    #[allow(clippy::disallowed_methods)]
    let mut cmd = std::process::Command::new(std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into()));
    cmd.arg("test").arg("--manifest-path").arg(&manifest);
    let status = cmd.status().context("cannot run cargo test")?;
    if !status.success() {
        bail!("cargo test failed");
    }
    Ok(())
}

fn package(mut args: Vec<String>) -> Result<()> {
    let manifest = manifest_path(&mut args)?;
    let sign = flag(&mut args, "--sign")?;
    let out = flag(&mut args, "--out")?;
    let mut targets = flags(&mut args, "--target")?;
    no_more(&args)?;
    let meta = PluginMeta::read(&manifest)?;
    let key = match &sign {
        Some(f) => Some(SecretKey::parse(
            &std::fs::read_to_string(f).with_context(|| format!("cannot read {f}"))?,
        )?),
        None => None,
    };
    targets.retain(|t| t != host_target());
    let mut programs = vec![(host_target().to_string(), build_program(&manifest, &meta, true, None)?)];
    for t in targets {
        let exe = build_program(&manifest, &meta, true, Some(&t))?;
        programs.push((t, exe));
    }
    let out = PathBuf::from(out.unwrap_or_else(|| format!("{}-{}.imtplugin", meta.name, meta.version)));
    let digest = pack(&meta, &programs, key.as_ref(), &out)?;
    println!(
        "Wrote {} ({}, digest {digest}){}",
        out.display(),
        programs.iter().map(|(t, _)| t.as_str()).collect::<Vec<_>>().join(", "),
        if key.is_some() {
            ""
        } else {
            "; unsigned: IMT installs it only in developer mode"
        }
    );
    Ok(())
}

fn keygen(mut args: Vec<String>) -> Result<()> {
    let out = PathBuf::from(flag(&mut args, "--out")?.context("--out <file> is required")?);
    no_more(&args)?;
    if out.exists() {
        bail!("{} exists; never overwrite a key", out.display());
    }
    let key = SecretKey::generate()?;
    std::fs::write(&out, format!("{}\n", key.to_text())).with_context(|| format!("cannot write {}", out.display()))?;
    println!("Secret key: {} (keep it private; anyone with it can sign as you)", out.display());
    println!(
        "Public key (users add it in IMT: Settings → Plugin trust):\n{}",
        key.public().to_text()
    );
    println!("Key id: {}", key.public().id());
    Ok(())
}

fn inspect(mut args: Vec<String>) -> Result<()> {
    if args.is_empty() {
        bail!("name the package: cargo imt-plugin inspect <file.imtplugin>");
    }
    let path = PathBuf::from(args.remove(0));
    no_more(&args)?;
    let p = Package::open(&path)?;
    let m = &p.manifest;
    println!("{} {} — {} ({})", m.name, m.version, m.id, m.publisher);
    println!("protocol {}; targets: {}", m.protocol, m.targets.join(", "));
    for r in &m.permissions {
        println!("permission {:?}: {}", r.permission, r.reason);
    }
    let status = TrustSettings::default().status(&p.signature);
    println!(
        "signature: {} ({})",
        status.code(),
        match &status {
            imt_plugin_package::SignatureStatus::Trusted(n) => n.clone(),
            imt_plugin_package::SignatureStatus::UnknownKey(k) => format!("key {k}"),
            imt_plugin_package::SignatureStatus::Unsigned => "not signed".into(),
        }
    );
    println!("digest: {}", p.digest());
    Ok(())
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    // `cargo imt-plugin x` runs `cargo-imt-plugin imt-plugin x`.
    if args.first().map(String::as_str) == Some("imt-plugin") {
        args.remove(0);
    }
    let command = if args.is_empty() { "help".to_string() } else { args.remove(0) };
    let r = match command.as_str() {
        "new" => new(args),
        "test" => test(args),
        "package" => package(args),
        "keygen" => keygen(args),
        "inspect" => inspect(args),
        "help" | "--help" | "-h" => {
            print!("{HELP}");
            Ok(())
        }
        other => Err(anyhow::anyhow!("unknown command \"{other}\"\n\n{HELP}")),
    };
    if let Err(e) = r {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}
