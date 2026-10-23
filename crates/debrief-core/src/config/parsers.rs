//! Choosing a parser for a file: the built-in formats plus every
//! [`ConfigParser`] contributed to [`PARSERS_POINT`]. Contributed parsers run
//! under their plugin's guard; built-in ones under [`crate::guard::catch`].

use super::formats::{IniParser, JsonParser, TomlParser, XmlParser, YamlParser};
use super::{dto::FormatInfo, ConfigError, ConfigParser, ConfigTree, Limits, PARSERS_POINT};
use crate::plugin::{PluginRegistry, Service};
use std::path::Path;
use std::sync::Arc;

enum Entry {
    Builtin(Arc<dyn ConfigParser>),
    Contributed(Service<dyn ConfigParser>),
}

impl Entry {
    fn call<R>(&self, f: impl FnOnce(&dyn ConfigParser) -> Result<R, ConfigError>) -> Result<R, ConfigError> {
        match self {
            Entry::Builtin(p) => crate::guard::catch("Debrief", || Ok(f(p.as_ref())))
                .unwrap_or_else(|e| Err(ConfigError::Plugin(format!("{e:#} (please report this file)")))),
            Entry::Contributed(s) => s.call(|p| f(p)).unwrap_or_else(|e| Err(ConfigError::Plugin(format!("{e:#}")))),
        }
    }
}

/// The parsers available to a workspace.
#[derive(Clone)]
pub struct ConfigParsers {
    entries: Arc<Vec<Entry>>,
}

impl std::fmt::Debug for ConfigParsers {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ConfigParsers").field("count", &self.entries.len()).finish()
    }
}

impl Default for ConfigParsers {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ConfigParsers {
    /// JSON, XML, INI, TOML and YAML.
    pub fn builtin() -> ConfigParsers {
        ConfigParsers {
            entries: Arc::new(builtins()),
        }
    }

    /// The built-in parsers and those plugins contributed to `registry`.
    pub fn with_registry(registry: &PluginRegistry) -> ConfigParsers {
        let mut entries = builtins();
        entries.extend(
            registry
                .contributions::<dyn ConfigParser>(PARSERS_POINT)
                .into_iter()
                .map(Entry::Contributed),
        );
        ConfigParsers {
            entries: Arc::new(entries),
        }
    }

    /// Supported formats, for the UI.
    pub fn formats(&self) -> Vec<FormatInfo> {
        self.entries
            .iter()
            .filter_map(|e| {
                e.call(|p| {
                    Ok(FormatInfo {
                        id: p.id().into(),
                        name: p.name().into(),
                        extensions: p.extensions(),
                    })
                })
                .ok()
            })
            .collect()
    }

    /// Parses text with the parser most confident about it.
    pub fn parse(&self, file_name: &str, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError> {
        let head: String = content.chars().take(4096).collect();
        let best = self
            .entries
            .iter()
            .filter_map(|e| e.call(|p| Ok(p.sniff(file_name, &head))).ok().map(|score| (score, e)))
            .filter(|(score, _)| *score > 0)
            // Highest score; the first registered wins a tie.
            .fold(None::<(u8, &Entry)>, |best, (score, e)| match best {
                Some((s, _)) if s >= score => best,
                _ => Some((score, e)),
            });
        match best {
            Some((_, e)) => e.call(|p| p.parse(content, limits)),
            None => Err(ConfigError::Unrecognised {
                supported: self.formats().iter().map(|f| f.name.clone()).collect::<Vec<_>>().join(", "),
            }),
        }
    }

    /// Whether some parser recognises a file (by name and first bytes).
    pub fn recognises(&self, file_name: &str, head: &str) -> bool {
        self.entries
            .iter()
            .any(|e| e.call(|p| Ok(p.sniff(file_name, head) > 0)).unwrap_or(false))
    }
}

fn builtins() -> Vec<Entry> {
    let parsers: [Arc<dyn ConfigParser>; 5] = [
        Arc::new(JsonParser),
        Arc::new(XmlParser),
        Arc::new(TomlParser),
        Arc::new(YamlParser),
        Arc::new(IniParser),
    ];
    parsers.into_iter().map(Entry::Builtin).collect()
}

/// Reads a file (size-limited, any text encoding Debrief reads) and parses it.
pub fn read_config(parsers: &ConfigParsers, path: &Path, limits: &Limits) -> Result<ConfigTree, ConfigError> {
    let meta = std::fs::metadata(path).map_err(|e| ConfigError::Io(format!("{}: {e}", path.display())))?;
    if !meta.is_file() {
        return Err(ConfigError::Io(format!("{} is not a file", path.display())));
    }
    if meta.len() > limits.max_bytes as u64 {
        return Err(ConfigError::TooLarge {
            bytes: meta.len(),
            max: limits.max_bytes,
        });
    }
    let bytes = std::fs::read(path).map_err(|e| ConfigError::Io(format!("{}: {e}", path.display())))?;
    let text = crate::importers::decode(&bytes).ok_or(ConfigError::Binary)?;
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    parsers.parse(&name, &text, limits)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{ConfigValue, PathSeg, TreeBuilder};
    use crate::{Plugin, PluginRegistry};

    /// A plugin format: `k -> v` lines in `.arrow` files.
    struct ArrowParser;
    impl ConfigParser for ArrowParser {
        fn id(&self) -> &str {
            "arrow"
        }
        fn name(&self) -> &str {
            "Arrow"
        }
        fn extensions(&self) -> Vec<String> {
            vec!["arrow".into()]
        }
        fn sniff(&self, file_name: &str, _head: &str) -> u8 {
            if file_name.ends_with(".arrow") {
                90
            } else {
                0
            }
        }
        fn parse(&self, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError> {
            let mut b = TreeBuilder::new("arrow", limits);
            for line in content.lines() {
                let (k, v) = line.split_once("->").ok_or_else(|| ConfigError::syntax("Arrow", "no arrow"))?;
                b.push(vec![PathSeg::key(k.trim())], ConfigValue::String(v.trim().into()))?;
            }
            Ok(b.finish())
        }
    }

    /// Panics on everything.
    struct Crashing;
    impl ConfigParser for Crashing {
        fn id(&self) -> &str {
            "crash"
        }
        fn name(&self) -> &str {
            panic!("no name")
        }
        fn extensions(&self) -> Vec<String> {
            Vec::new()
        }
        fn sniff(&self, file_name: &str, _head: &str) -> u8 {
            match file_name.rsplit('.').next() {
                Some("crash") => 100,
                Some("arrow") => 0,
                _ => panic!("sniff crashed"),
            }
        }
        fn parse(&self, _content: &str, _limits: &Limits) -> Result<ConfigTree, ConfigError> {
            panic!("parse crashed")
        }
    }

    struct FormatsPlugin;
    impl Plugin for FormatsPlugin {
        fn id(&self) -> &'static str {
            "test.formats"
        }
        fn name(&self) -> &'static str {
            "Formats"
        }
        fn version(&self) -> &'static str {
            "1.0.0"
        }
        fn register(&self, registry: &mut PluginRegistry) -> anyhow::Result<()> {
            registry.contribute::<dyn ConfigParser>(PARSERS_POINT, Arc::new(ArrowParser));
            registry.contribute::<dyn ConfigParser>(PARSERS_POINT, Arc::new(Crashing));
            Ok(())
        }
    }

    fn registry() -> PluginRegistry {
        let mut r = PluginRegistry::new();
        r.install(&FormatsPlugin).unwrap();
        r.resolve();
        r
    }

    #[test]
    fn picks_the_most_confident_parser() {
        let p = ConfigParsers::builtin();
        let l = Limits::default();
        assert_eq!(p.parse("a.json", "{}", &l).unwrap().format, "json");
        assert_eq!(p.parse("a.txt", "<?xml version=\"1.0\"?><r/>", &l).unwrap().format, "xml");
        assert_eq!(p.parse("a.yml", "a: 1", &l).unwrap().format, "yaml");
        assert_eq!(p.parse("a.ini", "a=1", &l).unwrap().format, "ini");
        assert_eq!(p.parse("a.toml", "a=1", &l).unwrap().format, "toml");
        let e = p.parse("a.txt", "plain words", &l).unwrap_err();
        assert_eq!(
            e.to_string(),
            "no configuration parser recognises this file (supported: JSON, XML, TOML, YAML, INI)"
        );
        assert!(p.recognises("x.json", ""));
        assert!(!p.recognises("x.log", "hello"));
        assert_eq!(format!("{:?}", ConfigParsers::default()), "ConfigParsers { count: 5 }");
    }

    #[test]
    fn plugin_parsers_are_used_and_contained() {
        let p = ConfigParsers::with_registry(&registry());
        let l = Limits::default();
        let t = p.parse("x.arrow", "a -> 1\nb -> 2", &l).unwrap();
        assert_eq!((t.format.as_str(), t.nodes.len()), ("arrow", 2));
        assert!(matches!(p.parse("x.arrow", "nope", &l), Err(ConfigError::Syntax { .. })));
        // The crashing parser's name panics: it is left out of the format list.
        let ids: Vec<String> = p.formats().into_iter().map(|f| f.id).collect();
        assert!(ids.contains(&"arrow".to_string()) && !ids.contains(&"crash".to_string()));
        // Its parse panics: an error attributed to the plugin.
        let e = p.parse("x.crash", "", &l).unwrap_err();
        assert!(matches!(e, ConfigError::Plugin(ref m) if m.contains("test.formats")), "{e}");
        // Its sniff panics on other files (the third crash quarantines it): skipped, others still work.
        assert_eq!(p.parse("a.json", "[1]", &l).unwrap().format, "json");
        assert!(p.parse("x.crash", "", &l).is_err());
    }

    #[test]
    fn builtin_panics_are_contained() {
        assert!(Crashing.extensions().is_empty());
        let e = Entry::Builtin(Arc::new(Crashing));
        let r: Result<(), ConfigError> = e.call(|p| p.parse("", &Limits::default()).map(|_| ()));
        assert!(matches!(r, Err(ConfigError::Plugin(m)) if m.contains("parse crashed")));
    }

    #[test]
    fn reads_files_with_limits() {
        let dir = std::env::temp_dir().join(format!("debrief-config-read-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let p = ConfigParsers::builtin();
        let l = Limits::default();
        let json = dir.join("a.json");
        // UTF-16 LE with BOM, as Windows tools write.
        let mut bytes = vec![0xFF, 0xFE];
        bytes.extend("{\"a\": 1}".encode_utf16().flat_map(|u| u.to_le_bytes()));
        std::fs::write(&json, bytes).unwrap();
        assert_eq!(read_config(&p, &json, &l).unwrap().nodes.len(), 1);
        let tiny = Limits { max_bytes: 3, ..l };
        assert!(matches!(read_config(&p, &json, &tiny), Err(ConfigError::TooLarge { max: 3, .. })));
        let bin = dir.join("b.json");
        std::fs::write(&bin, [0u8, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 1, 2, 3, 4, 5]).unwrap();
        assert_eq!(read_config(&p, &bin, &l), Err(ConfigError::Binary));
        assert!(matches!(read_config(&p, &dir.join("missing.json"), &l), Err(ConfigError::Io(_))));
        assert!(read_config(&p, &dir, &l).unwrap_err().to_string().contains("is not a file"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
