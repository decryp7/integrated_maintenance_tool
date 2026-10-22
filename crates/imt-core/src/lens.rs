//! Lenses: named view presets of a profile (`[[lens]]`, see `default_profile.toml`). A lens bundles
//! a query (levels, search text, filters, query language, marker kinds), the extra grid columns,
//! row highlights and the chart to bring forward. The UI applies one with a click; this module holds
//! the configuration and its validation, so a broken lens fails the profile with a message naming it.

use crate::model::Level;
use crate::query::{CompiledQuery, FieldFilter, Query};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};

/// Most lenses in a profile.
pub const MAX_LENSES: usize = 32;
/// Most filters, columns or highlights in one lens.
pub const MAX_LENS_ITEMS: usize = 16;
/// Longest search text or query-language expression of a lens, in bytes.
pub const MAX_LENS_TEXT: usize = 2_000;

/// Row highlight operators (what the grid's highlight rules can evaluate).
const HIGHLIGHT_OPS: [&str; 8] = ["eq", "ne", "contains", "notContains", "exists", "missing", "gt", "lt"];

/// `[[lens]]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensCfg {
    pub id: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub description: String,
    /// Only these levels; empty = all.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub levels: Vec<Level>,
    /// Search box text.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub text: String,
    /// The text is a regular expression.
    #[serde(default, skip_serializing_if = "is_false")]
    pub regex: bool,
    /// Query-language filter.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub ql: String,
    /// Only records carrying one of these marker kinds.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<String>,
    /// `[[lens.filter]]`: field filters.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub filter: Vec<FieldFilter>,
    /// Extra grid columns (field names), in order.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub columns: Vec<String>,
    /// `[[lens.highlight]]`: row colours.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub highlight: Vec<LensHighlight>,
    /// Id of the `[[chart]]` to show first.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub chart: Option<String>,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// `[[lens.highlight]]`: colour rows whose `field` satisfies `op` / `value`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LensHighlight {
    pub field: String,
    pub op: String,
    #[serde(default)]
    pub value: String,
    /// A `#rrggbb` colour.
    pub color: String,
}

impl LensCfg {
    /// The query part of the lens (what the UI puts into the query).
    pub fn query(&self) -> Query {
        Query {
            levels: self.levels.clone(),
            text: self.text.clone(),
            regex: self.regex,
            ql: self.ql.clone(),
            markers: self.markers.clone(),
            filters: self.filter.clone(),
            ..Query::default()
        }
    }

    /// Checks the lens; `charts` are the ids of the profile's charts.
    pub fn validate(&self, charts: &[String]) -> Result<()> {
        if self.id.trim().is_empty() {
            bail!("a lens needs an id");
        }
        if self.title.trim().is_empty() {
            bail!("lens \"{}\" needs a title", self.id);
        }
        self.check(charts).with_context(|| format!("lens \"{}\"", self.id))
    }

    fn check(&self, charts: &[String]) -> Result<()> {
        if self.text.len() > MAX_LENS_TEXT || self.ql.len() > MAX_LENS_TEXT {
            bail!("text and ql are limited to {MAX_LENS_TEXT} bytes");
        }
        for (what, n) in [
            ("filters", self.filter.len()),
            ("columns", self.columns.len()),
            ("highlights", self.highlight.len()),
        ] {
            if n > MAX_LENS_ITEMS {
                bail!("at most {MAX_LENS_ITEMS} {what}");
            }
        }
        if self.columns.iter().any(|c| c.trim().is_empty()) {
            bail!("a column name is empty");
        }
        // Compiling is the check: regexes, filter operators and the query language.
        let mut q = self.query();
        q.markers.clear();
        CompiledQuery::new(&q, &[], &[])?;
        for h in &self.highlight {
            if h.field.trim().is_empty() {
                bail!("a highlight needs a field");
            }
            if !HIGHLIGHT_OPS.contains(&h.op.as_str()) {
                bail!("highlight operator \"{}\" is unknown (use {})", h.op, HIGHLIGHT_OPS.join(", "));
            }
            if !is_hex_colour(&h.color) {
                bail!("highlight colour \"{}\" must look like #rrggbb", h.color);
            }
        }
        if let Some(chart) = &self.chart {
            if !charts.iter().any(|c| c == chart) {
                bail!("chart \"{chart}\" is not a [[chart]] id of this profile");
            }
        }
        Ok(())
    }
}

fn is_hex_colour(s: &str) -> bool {
    s.len() == 7 && s.starts_with('#') && s[1..].chars().all(|c| c.is_ascii_hexdigit())
}

/// Validates every lens of a profile: count, unique ids and each lens; `charts` are the chart ids.
pub fn validate_all(lenses: &[LensCfg], charts: &[String]) -> Result<()> {
    if lenses.len() > MAX_LENSES {
        bail!("at most {MAX_LENSES} [[lens]] sections");
    }
    let mut seen = std::collections::HashSet::new();
    for l in lenses {
        l.validate(charts)?;
        if !seen.insert(l.id.as_str()) {
            bail!("lens id \"{}\" is used twice", l.id);
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lens(id: &str) -> LensCfg {
        LensCfg {
            id: id.into(),
            title: "T".into(),
            description: String::new(),
            levels: vec![],
            text: String::new(),
            regex: false,
            ql: String::new(),
            markers: vec![],
            filter: vec![],
            columns: vec![],
            highlight: vec![],
            chart: None,
        }
    }

    fn err(l: &LensCfg, charts: &[String]) -> String {
        format!("{:#}", l.validate(charts).unwrap_err())
    }

    #[test]
    fn parses_from_toml_and_builds_its_query() {
        #[derive(Deserialize)]
        struct P {
            lens: Vec<LensCfg>,
        }
        let p: P = toml::from_str(
            r##"
[[lens]]
id = "errors"
title = "Errors only"
levels = ["Error", "Fatal"]
text = "timeout"
regex = true
ql = "level >= error"
markers = ["restart"]
columns = ["SourceContext"]
chart = "chart-1"
[[lens.filter]]
field = "SourceContext"
op = "contains"
value = "Http"
[[lens.highlight]]
field = "@level"
op = "eq"
value = "Error"
color = "#f2545b"
"##,
        )
        .unwrap();
        let l = &p.lens[0];
        l.validate(&["chart-1".to_string()]).unwrap();
        let q = l.query();
        assert_eq!(
            (q.levels.len(), q.text.as_str(), q.regex, q.ql.as_str()),
            (2, "timeout", true, "level >= error")
        );
        assert_eq!((q.filters.len(), q.markers.clone()), (1, vec!["restart".to_string()]));
        let json = serde_json::to_value(l).unwrap();
        assert_eq!(json["columns"][0], "SourceContext");
        assert!(json.get("description").is_none());
        assert!(toml::from_str::<P>("[[lens]]\nid = \"a\"").is_err());
    }

    #[test]
    fn invalid_lenses_say_what_to_fix() {
        let none: &[String] = &[];
        let mut l = lens("");
        assert!(err(&l, none).contains("needs an id"));
        l.id = "a".into();
        l.title = " ".into();
        assert!(err(&l, none).contains("needs a title"));
        l.title = "T".into();
        l.validate(none).unwrap();

        let mut bad = l.clone();
        bad.text = "x".repeat(MAX_LENS_TEXT + 1);
        assert!(err(&bad, none).contains("limited"));
        bad = l.clone();
        bad.columns = vec!["c".into(); MAX_LENS_ITEMS + 1];
        assert!(err(&bad, none).contains("at most 16 columns"));
        bad.columns = vec![" ".into()];
        assert!(err(&bad, none).contains("column name is empty"));
        bad = l.clone();
        bad.regex = true;
        bad.text = "(".into();
        assert!(err(&bad, none).contains("lens \"a\""));
        bad = l.clone();
        bad.ql = "| | |".into();
        assert!(l.validate(none).is_ok() && bad.validate(none).is_err());
        bad = l.clone();
        bad.filter = vec![FieldFilter {
            field: "f".into(),
            op: "nope".into(),
            value: None,
        }];
        assert!(bad.validate(none).is_err());

        let hl = |field: &str, op: &str, color: &str| LensHighlight {
            field: field.into(),
            op: op.into(),
            value: String::new(),
            color: color.into(),
        };
        bad = l.clone();
        bad.highlight = vec![hl(" ", "eq", "#112233")];
        assert!(err(&bad, none).contains("needs a field"));
        bad.highlight = vec![hl("f", "matches", "#112233")];
        assert!(err(&bad, none).contains("operator"));
        bad.highlight = vec![hl("f", "eq", "red")];
        assert!(err(&bad, none).contains("#rrggbb"));
        bad.highlight = vec![hl("f", "exists", "#a1B2c3")];
        bad.validate(none).unwrap();

        bad = l.clone();
        bad.chart = Some("c1".into());
        assert!(err(&bad, none).contains("not a [[chart]] id"));
        bad.validate(&["c1".to_string()]).unwrap();
    }

    #[test]
    fn a_profile_may_hold_a_bounded_number_of_distinct_lenses() {
        validate_all(&[lens("a"), lens("b")], &[]).unwrap();
        assert!(validate_all(&[lens("a"), lens("a")], &[])
            .unwrap_err()
            .to_string()
            .contains("used twice"));
        let many: Vec<LensCfg> = (0..=MAX_LENSES).map(|i| lens(&i.to_string())).collect();
        assert!(validate_all(&many, &[]).unwrap_err().to_string().contains("at most"));
    }

    #[test]
    fn the_default_profile_ships_lenses_and_a_bad_lens_fails_the_profile() {
        let defaults = crate::profile::Profile::default_profile();
        assert!(defaults
            .lens
            .iter()
            .any(|l| l.id == "errors" && l.levels == vec![Level::Error, Level::Fatal]));
        crate::profile::CompiledProfile::compile(defaults).unwrap();

        let mut p = crate::profile::Profile::default_profile();
        p.lens.push(LensCfg {
            chart: Some("missing".into()),
            ..lens("broken")
        });
        let e = format!("{:#}", crate::profile::CompiledProfile::compile(p).err().unwrap());
        assert!(e.contains("[[lens]]") && e.contains("broken") && e.contains("missing"), "{e}");

        let mut p = crate::profile::Profile::default_profile();
        p.chart[0].id = None;
        p.lens = vec![LensCfg {
            chart: Some("chart-1".into()),
            ..lens("c")
        }];
        crate::profile::CompiledProfile::compile(p).unwrap();
    }
}
