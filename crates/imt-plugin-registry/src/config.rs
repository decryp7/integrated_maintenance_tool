//! `.reg` files as configuration trees for the diff view: `HIVE/key/.../value`
//! paths compared without regard to case (as Windows does), typed values
//! (strings, numbers, string lists, bytes). Deleted keys and values (`[-...]`,
//! `"name"=-`) are null.

use crate::{parse_reg, RegData, RegEntry};
use imt_plugin_api::config::{ConfigError, ConfigParser, ConfigTree, ConfigValue, Limits, PathSeg, TreeBuilder};

pub struct RegConfigParser;

impl ConfigParser for RegConfigParser {
    fn id(&self) -> &str {
        "registry"
    }

    fn name(&self) -> &str {
        "Registry export"
    }

    fn extensions(&self) -> Vec<String> {
        vec!["reg".into()]
    }

    fn sniff(&self, file_name: &str, head: &str) -> u8 {
        let first = head.trim_start_matches('\u{feff}').trim_start().lines().next().unwrap_or("").trim();
        if first.starts_with("Windows Registry Editor Version") || first == "REGEDIT4" {
            100
        } else if imt_plugin_api::config::extension_of(file_name) == "reg" {
            80
        } else {
            0
        }
    }

    fn parse(&self, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError> {
        let mut b = TreeBuilder::new(self.id(), limits).case_insensitive(true);
        for entry in parse_reg(content).entries {
            if b.is_full() {
                b.mark_truncated();
                break;
            }
            match entry {
                RegEntry::Key { hive, path, deleted, .. } => {
                    let value = if deleted { ConfigValue::Null } else { ConfigValue::Section };
                    b.push(key_path(&hive, &path), value)?;
                }
                RegEntry::Value {
                    hive, path, name, data, ..
                } => {
                    let mut p = key_path(&hive, &path);
                    p.push(PathSeg::Key(name));
                    b.push(p, value(data))?;
                }
            }
        }
        Ok(b.finish())
    }
}

fn key_path(hive: &str, path: &str) -> Vec<PathSeg> {
    std::iter::once(hive)
        .chain(path.split('\\').filter(|s| !s.is_empty()))
        .map(PathSeg::key)
        .collect()
}

fn value(data: RegData) -> ConfigValue {
    match data {
        RegData::Text(s) => ConfigValue::String(s),
        RegData::Number(n) => i64::try_from(n).map_or_else(|_| ConfigValue::String(n.to_string()), ConfigValue::Integer),
        RegData::Multi(items) => ConfigValue::List(items),
        RegData::Bytes(b) => ConfigValue::Binary(b),
        RegData::Delete => ConfigValue::Null,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const LEFT: &str = "Windows Registry Editor Version 5.00\r\n\r\n\
        [HKEY_LOCAL_MACHINE\\SOFTWARE\\Contoso]\r\n\
        \"Timeout\"=dword:00001388\r\n\
        \"Path\"=\"C:\\\\App\"\r\n\
        \"Hosts\"=hex(7):61,00,00,00,62,00,00,00,00,00\r\n\
        \"Blob\"=hex:de,ad\r\n\
        \"Huge\"=hex(b):ff,ff,ff,ff,ff,ff,ff,ff\r\n\
        \"InstallDate\"=dword:00000001\r\n\
        [-HKEY_CURRENT_USER\\Software\\Old]\r\n\
        \"Gone\"=-\r\n";

    #[test]
    fn reads_keys_and_typed_values() {
        let t = RegConfigParser.parse(LEFT, &Limits::default()).unwrap();
        assert!(t.case_insensitive);
        let get = |p: &str| {
            t.nodes
                .iter()
                .find(|n| n.path.iter().map(PathSeg::label).collect::<Vec<_>>().join("/") == p)
                .map(|n| n.value.clone())
        };
        assert_eq!(get("HKEY_LOCAL_MACHINE/SOFTWARE/Contoso"), Some(ConfigValue::Section));
        assert_eq!(get("HKEY_LOCAL_MACHINE/SOFTWARE/Contoso/Timeout"), Some(ConfigValue::Integer(5000)));
        assert_eq!(
            get("HKEY_LOCAL_MACHINE/SOFTWARE/Contoso/Path"),
            Some(ConfigValue::String("C:\\App".into()))
        );
        assert_eq!(
            get("HKEY_LOCAL_MACHINE/SOFTWARE/Contoso/Hosts"),
            Some(ConfigValue::List(vec!["a".into(), "b".into()]))
        );
        assert_eq!(
            get("HKEY_LOCAL_MACHINE/SOFTWARE/Contoso/Blob"),
            Some(ConfigValue::Binary(vec![0xde, 0xad]))
        );
        assert_eq!(
            get("HKEY_LOCAL_MACHINE/SOFTWARE/Contoso/Huge"),
            Some(ConfigValue::String(u64::MAX.to_string()))
        );
        assert_eq!(get("HKEY_CURRENT_USER/Software/Old"), Some(ConfigValue::Null));
        assert_eq!(value(RegData::Delete), ConfigValue::Null);
    }

    #[test]
    fn sniffs_and_truncates() {
        assert_eq!(RegConfigParser.sniff("x", "\u{feff}REGEDIT4\n"), 100);
        assert_eq!(RegConfigParser.sniff("x.REG", "junk"), 80);
        assert_eq!(RegConfigParser.sniff("x.txt", "junk"), 0);
        assert_eq!(
            (RegConfigParser.id(), RegConfigParser.name(), RegConfigParser.extensions()),
            ("registry", "Registry export", vec!["reg".to_string()])
        );
        let limits = Limits {
            max_nodes: 2,
            ..Limits::default()
        };
        let t = RegConfigParser.parse(LEFT, &limits).unwrap();
        assert!(t.truncated);
        assert_eq!(t.nodes.len(), 2);
        let deep = format!("REGEDIT4\n[HKLM{}]\n", "\\k".repeat(200));
        assert!(matches!(
            RegConfigParser.parse(&deep, &Limits::default()),
            Err(ConfigError::TooDeep { .. })
        ));
    }
}
