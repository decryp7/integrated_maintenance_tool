//! Built-in configuration formats: JSON, XML, INI, TOML and YAML.

mod ini;
mod json;
mod toml;
mod xml;
mod yaml;

pub use self::toml::TomlParser;
pub use ini::IniParser;
pub use json::JsonParser;
pub use xml::XmlParser;
pub use yaml::YamlParser;

use super::{extension_of, ConfigError};

/// Score from the file extension: 80 when it is one of `exts`, else 0.
fn by_extension(file_name: &str, exts: &[&str]) -> u8 {
    if exts.contains(&extension_of(file_name).as_str()) {
        80
    } else {
        0
    }
}

/// The text without a leading byte-order mark and whitespace.
fn head_of(head: &str) -> &str {
    head.trim_start_matches('\u{feff}').trim_start()
}

/// Deepest nesting of `[`/`{` outside quoted strings and `#` comments, so a
/// recursive parser is never handed input deeper than the limit.
fn bracket_depth(content: &str) -> usize {
    let (mut depth, mut max) = (0usize, 0usize);
    let mut quote: Option<char> = None;
    let mut comment = false;
    let mut escaped = false;
    for c in content.chars() {
        if comment {
            comment = c != '\n';
            continue;
        }
        if let Some(q) = quote {
            if escaped {
                escaped = false;
            } else if c == '\\' && q == '"' {
                escaped = true;
            } else if c == q || c == '\n' {
                quote = None;
            }
            continue;
        }
        match c {
            '"' | '\'' => quote = Some(c),
            '#' => comment = true,
            '[' | '{' => {
                depth += 1;
                max = max.max(depth);
            }
            ']' | '}' => depth = depth.saturating_sub(1),
            _ => {}
        }
    }
    max
}

fn check_nesting(content: &str, max: usize) -> Result<(), ConfigError> {
    if bracket_depth(content) > max {
        return Err(ConfigError::TooDeep { max });
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nesting_ignores_strings_and_comments() {
        assert_eq!(bracket_depth("[[1, \"[[[\", '{{'], {a = 1}] # [[[[["), 2);
        assert_eq!(bracket_depth("a = \"x\\\"[\" ]]] [ "), 1);
        assert_eq!(bracket_depth("'unterminated\n[["), 2);
        assert!(check_nesting(&"[".repeat(5), 4).is_err());
        assert!(check_nesting("[]", 4).is_ok());
        assert_eq!(by_extension("A.JSON", &["json"]), 80);
        assert_eq!(by_extension("a", &["json"]), 0);
        assert_eq!(head_of("\u{feff}  x"), "x");
    }
}
