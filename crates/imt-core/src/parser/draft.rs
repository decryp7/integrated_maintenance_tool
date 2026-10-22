//! Parsers composed for the format wizard (ADR 0026 §5): a new parser for a file IMT did not recognise (from
//! a regex or a Serilog output template), or a changed copy of a source's parser (a timestamp format, a
//! regex or a template added first). Nothing is saved: the wizard previews the draft and saves it.

use super::{Parser, ParserError, ParserVersion};
use serde::{Deserialize, Serialize};

/// What the wizard asks for (`parser_draft`).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DraftRequest {
    /// The file the wizard was opened for (names a new parser).
    pub file_name: String,
    /// The parser to change; `None` for a new parser.
    pub base: Option<String>,
    /// A regex with named groups, added first.
    pub regex: Option<String>,
    /// A Serilog output template, added first.
    pub template: Option<String>,
    /// A timestamp format, added first.
    pub timestamp_format: Option<String>,
}

fn given(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// The id of a new parser for `file_name`: `custom-<stem>` in parser-id characters.
pub fn id_for(file_name: &str) -> String {
    let stem = std::path::Path::new(file_name)
        .file_stem()
        .map(|s| s.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let mut slug = String::new();
    for c in stem.chars() {
        if c.is_ascii_alphanumeric() {
            slug.push(c);
        } else if !slug.ends_with('-') {
            slug.push('-');
        }
    }
    let slug: String = slug.trim_matches('-').chars().take(56).collect();
    if slug.is_empty() {
        "custom".into()
    } else {
        format!("custom-{slug}")
    }
}

/// Composes the draft: `base` (the parser to change) with the request's additions and the next patch
/// version, or a new text parser. Returns its TOML, checked like any parser file.
pub fn compose(req: &DraftRequest, base: Option<&Parser>) -> Result<String, ParserError> {
    let (regex, template, ts) = (given(&req.regex), given(&req.template), given(&req.timestamp_format));
    if regex.is_none() && template.is_none() && ts.is_none() {
        return Err(ParserError::new(
            "nothing to add: give a regex, an output template or a timestamp format",
        ));
    }
    let mut parser = match base {
        Some(b) => {
            let mut p = b.clone();
            p.version = p.version.bump_patch();
            p
        }
        None => {
            let name = std::path::Path::new(&req.file_name)
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let text = format!(
                "schema = 1\nid = \"{}\"\nname = {}\nversion = \"1.0.0\"\ncategory = \"Custom\"\n[format]\nkind = \"text\"\n",
                id_for(&req.file_name),
                toml::Value::String(format!("Custom: {}", if name.is_empty() { "log" } else { &name })),
            );
            let mut p = Parser::parse_unchecked(&text)?;
            p.description = format!(
                "Made with the format wizard from {}.",
                if name.is_empty() { "a sample" } else { &name }
            );
            p.version = ParserVersion::new(1, 0, 0);
            p
        }
    };
    let format = &mut parser.format;
    if let Some(r) = regex {
        format.regexes.retain(|x| x != r);
        format.regexes.insert(0, r.to_string());
    }
    if let Some(t) = template {
        format.output_templates.retain(|x| x != t);
        format.output_templates.insert(0, t.to_string());
    }
    if let Some(t) = ts {
        format.timestamp_formats.retain(|x| x != t);
        format.timestamp_formats.insert(0, t.to_string());
    }
    // A base of kind `auto` reads text lines with the added format; JSON, CSV and plugin kinds keep theirs.
    let text = parser.canonical_toml();
    Parser::from_toml(&text)?;
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::builtin;

    fn req(file: &str) -> DraftRequest {
        DraftRequest {
            file_name: file.into(),
            ..Default::default()
        }
    }

    #[test]
    fn ids_for_file_names() {
        assert_eq!(id_for("Orders_2025-11-08.log"), "custom-orders-2025-11-08");
        assert_eq!(id_for("C:/logs/app.txt"), "custom-app");
        assert_eq!(id_for("***.log"), "custom");
        assert_eq!(id_for(""), "custom");
        assert!(id_for(&format!("{}.log", "a".repeat(100))).len() <= 64);
    }

    #[test]
    fn a_new_parser_from_a_regex_or_a_template() {
        let r = DraftRequest {
            regex: Some("^(?P<timestamp>\\S+ \\S+) (?P<level>\\w+) (?P<message>.*)$".into()),
            ..req("orders.log")
        };
        let p = Parser::from_toml(&compose(&r, None).unwrap()).unwrap();
        assert_eq!(
            (p.id.as_str(), p.name.as_str(), p.version.to_string().as_str()),
            ("custom-orders", "Custom: orders.log", "1.0.0")
        );
        assert_eq!((p.format.kind.as_str(), p.format.regexes.len()), ("text", 1));
        let t = DraftRequest {
            template: Some("{Timestamp:HH:mm:ss} {Message}".into()),
            timestamp_format: Some("%H:%M:%S".into()),
            ..req("x.log")
        };
        let p = Parser::from_toml(&compose(&t, None).unwrap()).unwrap();
        assert_eq!(p.format.output_templates, ["{Timestamp:HH:mm:ss} {Message}"]);
        assert_eq!(p.format.timestamp_formats, ["%H:%M:%S"]);
        assert!(compose(&req("x.log"), None).unwrap_err().message.contains("nothing to add"));
        let bad = DraftRequest {
            regex: Some("(".into()),
            ..req("x.log")
        };
        assert!(compose(&bad, None).is_err());
    }

    #[test]
    fn a_changed_copy_of_a_parser_gets_the_next_version() {
        let base = builtin::find("log4net-csv").unwrap();
        let r = DraftRequest {
            base: Some("log4net-csv".into()),
            timestamp_format: Some("%Y/%m/%d,%H:%M:%S%.f%:z".into()),
            ..req("orders.csv")
        };
        let p = Parser::from_toml(&compose(&r, Some(base)).unwrap()).unwrap();
        assert_eq!((p.id.as_str(), p.version.to_string().as_str()), ("log4net-csv", "1.0.1"));
        assert_eq!(p.format.timestamp_formats, ["%Y/%m/%d,%H:%M:%S%.f%:z"]);
        assert_eq!(p.format.csv, base.format.csv);
        // Adding a format already there moves it first instead of repeating it.
        let again = Parser::from_toml(&compose(&r, Some(&p)).unwrap()).unwrap();
        assert_eq!(again.format.timestamp_formats.len(), 1);
        assert_eq!(again.version.to_string(), "1.0.2");
    }
}
