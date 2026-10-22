//! `cargo debrief-plugin new`: a new plugin project from one of the example plugins (`examples/plugins/<kind>-example`),
//! which are built and tested in Debrief's CI, so a new project starts from code that compiles and passes the harness.

use anyhow::{bail, Context, Result};
use std::path::{Path, PathBuf};

/// The kinds of plugin a project can start as.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// Reads a file format.
    Importer,
    /// A pipeline step.
    Transform,
    /// Operations the UI calls.
    Tool,
    /// A tool that runs in the background and collects data.
    Collector,
    /// Marks ranges and points on the timeline.
    Annotator,
}

impl Kind {
    /// Every kind.
    pub const ALL: [Kind; 5] = [Kind::Importer, Kind::Transform, Kind::Tool, Kind::Collector, Kind::Annotator];

    /// `importer`, `transform`, ...
    pub fn name(self) -> &'static str {
        match self {
            Kind::Importer => "importer",
            Kind::Transform => "transform",
            Kind::Tool => "tool",
            Kind::Collector => "collector",
            Kind::Annotator => "annotator",
        }
    }

    /// The kind called `s`.
    pub fn parse(s: &str) -> Result<Kind> {
        Kind::ALL
            .into_iter()
            .find(|k| k.name() == s)
            .with_context(|| format!("unknown kind \"{s}\"; use one of importer, transform, tool, collector, annotator"))
    }

    /// The example's plugin id, replaced by the new plugin's.
    fn example_id(self) -> &'static str {
        match self {
            Kind::Importer => "net.decryptology.example.kv-lines",
            Kind::Transform => "net.decryptology.example.transform",
            Kind::Tool => "net.decryptology.example.tool",
            Kind::Collector => "net.decryptology.example.collector",
            Kind::Annotator => "net.decryptology.example.annotator",
        }
    }

    /// The example's files (path in the project, text).
    fn files(self) -> Vec<(&'static str, &'static str)> {
        macro_rules! example {
            ($kind:literal, $($path:literal),+) => {
                vec![$(($path, include_str!(concat!("../../../examples/plugins/", $kind, "-example/", $path)))),+]
            };
        }
        match self {
            Kind::Importer => example!(
                "importer",
                "Cargo.toml",
                "src/lib.rs",
                "src/main.rs",
                "tests/plugin.rs",
                "tests/samples/app.log",
                "tests/golden/app.json"
            ),
            Kind::Transform => example!("transform", "Cargo.toml", "src/lib.rs", "src/main.rs", "tests/plugin.rs"),
            Kind::Tool => example!("tool", "Cargo.toml", "src/lib.rs", "src/main.rs", "tests/plugin.rs"),
            Kind::Collector => example!("collector", "Cargo.toml", "src/lib.rs", "src/main.rs", "tests/plugin.rs"),
            Kind::Annotator => example!("annotator", "Cargo.toml", "src/lib.rs", "src/main.rs", "tests/plugin.rs"),
        }
    }
}

/// Where a new project gets the SDK from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Sdk {
    /// Debrief's repository (optionally at a tag or revision).
    Git {
        /// The repository URL.
        url: String,
        /// `tag = "..."` or `rev = "..."`, or nothing for the default branch.
        pin: Option<(String, String)>,
    },
    /// A local checkout of Debrief (its root folder).
    Path(PathBuf),
}

/// Debrief's repository.
pub const REPOSITORY: &str = "https://dev.decryptology.net/decryp7/debrief";

impl Sdk {
    /// The dependency line value for crate `krate` (`debrief-plugin-api` or `debrief-plugin-test`).
    fn dependency(&self, krate: &str) -> String {
        match self {
            Sdk::Git { url, pin } => match pin {
                Some((k, v)) => format!("{{ git = \"{url}\", {k} = \"{v}\" }}"),
                None => format!("{{ git = \"{url}\" }}"),
            },
            Sdk::Path(root) => {
                let p = root.join("crates").join(krate);
                format!("{{ path = {:?} }}", p.to_string_lossy().replace('\\', "/"))
            }
        }
    }
}

/// What `new` creates.
#[derive(Debug, Clone)]
pub struct NewProject {
    /// The crate name (`kebab-case`), also the program's name.
    pub name: String,
    /// The plugin id (default `com.example.<name>`).
    pub id: String,
    /// What kind of plugin.
    pub kind: Kind,
    /// Where the SDK comes from.
    pub sdk: Sdk,
}

/// Whether `name` is a valid crate and program name: 1–64 characters from `a–z 0–9 -`, starting with a letter.
pub fn valid_name(name: &str) -> bool {
    (1..=64).contains(&name.len())
        && name.starts_with(|c: char| c.is_ascii_lowercase())
        && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

/// The project's files (path, text) for `p`.
pub fn render(p: &NewProject) -> Result<Vec<(String, String)>> {
    if !valid_name(&p.name) {
        bail!(
            "\"{}\" is not a valid name: use 1–64 characters from a–z, 0–9 and '-', starting with a letter",
            p.name
        );
    }
    if !debrief_plugin_package::manifest::valid_id(&p.id) {
        bail!(
            "\"{}\" is not a valid plugin id: use reverse-DNS style, e.g. com.example.{}",
            p.id,
            p.name
        );
    }
    let kind = p.kind.name();
    let example_crate = format!("debrief-example-{kind}");
    let snake = p.name.replace('-', "_");
    let mut out = Vec::new();
    for (path, text) in p.kind.files() {
        let mut t = text
            .replace(p.kind.example_id(), &p.id)
            .replace(&example_crate, &p.name)
            .replace(&example_crate.replace('-', "_"), &snake);
        if path == "Cargo.toml" {
            for krate in ["debrief-plugin-api", "debrief-plugin-test"] {
                t = t.replace(&format!("{{ path = \"../../../crates/{krate}\" }}"), &p.sdk.dependency(krate));
            }
            t = t.replace("publisher = \"Debrief examples\"", "publisher = \"Your name\"");
            // A project of its own, not a member of whatever workspace it is created in.
            t.push_str("\n[workspace]\n");
        }
        out.push((path.to_string(), t));
    }
    out.push(("README.md".into(), readme(p)));
    out.push((".gitignore".into(), "/target\n*.debriefplugin\n*.secret-key\n".into()));
    Ok(out)
}

fn readme(p: &NewProject) -> String {
    format!(
        "# {name}\n\nAn Debrief {kind} plugin (`{id}`), started from Debrief's {kind} example.\n\n\
         ```sh\n\
         cargo debrief-plugin test                       # conformance checks in Debrief's sandbox, then cargo test\n\
         DEBRIEF_BLESS=1 cargo test                      # accept new golden output (review the diff)\n\
         cargo debrief-plugin keygen --out me.secret-key  # once: your publisher key (keep it private)\n\
         cargo debrief-plugin package --sign me.secret-key\n\
         ```\n\n\
         Install the `.debriefplugin` file in Debrief: Plugins → Install from file. Users who trust your public key (printed by \
         `keygen`) install it without developer mode.\n\n\
         The walkthrough, the API reference and the rules for permissions and the sandbox: Debrief's `docs/plugin-sdk.md`.\n",
        name = p.name,
        kind = p.kind.name(),
        id = p.id
    )
}

/// Writes the project into `dir/<name>`, which must not exist yet. Returns the folder.
pub fn create(p: &NewProject, dir: &Path) -> Result<PathBuf> {
    let root = dir.join(&p.name);
    if root.exists() {
        bail!("{} already exists", root.display());
    }
    for (path, text) in render(p)? {
        let file = root.join(path);
        if let Some(parent) = file.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("cannot create {}", parent.display()))?;
        }
        std::fs::write(&file, text).with_context(|| format!("cannot write {}", file.display()))?;
    }
    Ok(root)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_kind_renders_a_project_with_its_names_and_sdk() {
        for kind in Kind::ALL {
            let p = NewProject {
                name: "my-plugin".into(),
                id: "com.example.my-plugin".into(),
                kind,
                sdk: Sdk::Path("/src/debrief".into()),
            };
            let files = render(&p).unwrap();
            let get = |n: &str| files.iter().find(|(p, _)| p == n).map(|(_, t)| t.clone()).unwrap();
            let cargo = get("Cargo.toml");
            assert!(cargo.contains("name = \"my-plugin\""), "{kind:?}: {cargo}");
            assert!(cargo.contains("id = \"com.example.my-plugin\""));
            assert!(
                cargo.contains(r#"debrief-plugin-api = { path = "/src/debrief/crates/debrief-plugin-api" }"#),
                "{cargo}"
            );
            assert!(cargo.contains(r#"debrief-plugin-test = { path = "/src/debrief/crates/debrief-plugin-test" }"#));
            assert!(cargo.trim_end().ends_with("[workspace]"));
            assert!(get("src/main.rs").contains("my_plugin::"), "{kind:?}");
            assert!(get("src/lib.rs").contains("\"com.example.my-plugin\""), "{kind:?}");
            assert!(get("tests/plugin.rs").contains("CARGO_BIN_EXE_my-plugin"));
            for (path, text) in &files {
                assert!(
                    !text.contains("debrief-example") && !text.contains("debrief_example"),
                    "{kind:?} {path}"
                );
                assert!(!text.contains("net.decryptology.example"), "{kind:?} {path}");
            }
            assert!(get("README.md").contains(kind.name()));
        }
        assert_eq!(Kind::parse("collector").unwrap(), Kind::Collector);
        assert!(Kind::parse("exporter").is_err());
    }

    #[test]
    fn names_ids_and_folders_are_checked() {
        let p = |name: &str, id: &str| NewProject {
            name: name.into(),
            id: id.into(),
            kind: Kind::Tool,
            sdk: Sdk::Git {
                url: REPOSITORY.into(),
                pin: Some(("tag".into(), "v1.0.0".into())),
            },
        };
        assert!(render(&p("My Plugin", "com.example.x")).is_err());
        assert!(render(&p("1st", "com.example.x")).is_err());
        assert!(render(&p("ok", "nodots")).is_err());
        let cargo = render(&p("ok", "com.example.ok")).unwrap()[0].1.clone();
        assert!(cargo.contains(&format!(r#"{{ git = "{REPOSITORY}", tag = "v1.0.0" }}"#)), "{cargo}");
        let t = tempfile::tempdir().unwrap();
        let root = create(&p("ok", "com.example.ok"), t.path()).unwrap();
        assert!(root.join("src/lib.rs").is_file() && root.join("README.md").is_file());
        assert!(create(&p("ok", "com.example.ok"), t.path()).is_err(), "never overwrites");
    }
}
