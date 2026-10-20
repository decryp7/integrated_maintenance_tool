//! Compiling a query filter to SQL.
//!
//! Values are written as escaped literals (numbers formatted, text in single quotes with quotes
//! doubled), so a clause is self-contained and can be reused in any position of a statement. Text with a
//! NUL character is never put into SQL; such criteria are left to the batch scan.

use imt_core::batch::EventSchema;
use imt_core::ports::{Filter, StoreError, StoreResult};
use imt_core::query::{CompiledQuery, MAX_QUERY_SOURCES};

/// Sort key of time order: untimed events last.
pub const TIME_KEY: &str = "coalesce(e.ts, 9223372036854775807)";

/// `ORDER BY` of time order (ties by source, line, id), or its reverse.
pub fn order_by(descending: bool) -> String {
    let d = if descending { " DESC" } else { "" };
    format!("ORDER BY {TIME_KEY}{d}, e.source{d}, e.line{d}, e.id{d}")
}

/// A SQL string literal, or `None` for text that cannot be one (a NUL character).
pub fn lit(s: &str) -> Option<String> {
    (!s.contains('\0')).then(|| format!("'{}'", s.replace('\'', "''")))
}

/// A quoted identifier.
pub fn ident(s: &str) -> String {
    format!("\"{}\"", s.replace('"', "\"\""))
}

/// The `LIKE` pattern that finds `text` anywhere (`\` escapes).
fn contains_pattern(text: &str) -> String {
    let mut p = String::from("%");
    for c in text.chars() {
        if matches!(c, '%' | '_' | '\\') {
            p.push('\\');
        }
        p.push(c);
    }
    p.push('%');
    p
}

/// SQL text of a field or pseudo-field for events `e` joined with sources `s`, as
/// `Record::field_text` gives it (NULL when missing); `None` when it cannot be expressed.
pub fn text_expr(name: &str, schema: &EventSchema) -> Option<String> {
    Some(match name {
        "@level" => {
            "CASE e.level WHEN 0 THEN 'Verbose' WHEN 1 THEN 'Debug' WHEN 2 THEN 'Information' WHEN 3 THEN 'Warning' WHEN 4 THEN 'Error' ELSE 'Fatal' END"
                .into()
        }
        "@message" => "e.message".into(),
        "@exception" => "e.exception".into(),
        "@template" => "e.template".into(),
        "@source" => "s.name".into(),
        "@line" => "CAST(e.line AS VARCHAR)".into(),
        _ => {
            let key = lit(name)?;
            match schema.declared_index(name) {
                Some(i) => format!(
                    "coalesce(CAST(e.{} AS VARCHAR), e.extra[{key}])",
                    ident(&schema.declared()[i].column)
                ),
                None => format!("e.extra[{key}]"),
            }
        }
    })
}

/// SQL of the instance key (as `imt_core::query::group_key`): the fields' values joined with " · "
/// ("–" for a missing one), or the source name when none is present; "all" without fields.
pub fn group_key_expr(fields: &[String], schema: &EventSchema) -> Option<String> {
    if fields.is_empty() {
        return Some("'all'".into());
    }
    let mut parts = Vec::new();
    let mut present = Vec::new();
    for f in fields {
        let t = text_expr(f, schema)?;
        parts.push(format!("CASE WHEN coalesce({t}, '') = '' THEN '–' ELSE {t} END"));
        present.push(format!("coalesce({t}, '') <> ''"));
    }
    Some(format!(
        "CASE WHEN {} THEN concat({}) ELSE s.name END",
        present.join(" OR "),
        parts.join(", ' · ', ")
    ))
}

/// A filter compiled to SQL conditions on events `e`.
#[derive(Debug, Clone)]
pub struct SqlFilter {
    /// Sources and time range (decide which events belong on the timeline).
    pub structural: Vec<String>,
    /// Levels, plain text and markers (decide which events match).
    pub content: Vec<String>,
    /// True when the SQL is the whole filter; otherwise matching rows must also pass the
    /// [`CompiledQuery`] (regex text, field filters, instance selection, query language).
    pub exact: bool,
}

fn and(parts: &[String]) -> String {
    if parts.is_empty() {
        "TRUE".into()
    } else {
        parts.iter().map(|p| format!("({p})")).collect::<Vec<_>>().join(" AND ")
    }
}

impl SqlFilter {
    pub fn structural_sql(&self) -> String {
        and(&self.structural)
    }

    pub fn content_sql(&self) -> String {
        and(&self.content)
    }

    /// Structural and content conditions together.
    pub fn all_sql(&self) -> String {
        and(&[self.structural_sql(), self.content_sql()])
    }
}

/// Validates the filter (as the in-memory store does) and compiles what SQL can express.
pub fn compile(q: &Filter, default_group_by: &[String], marker_kinds: &[String]) -> StoreResult<(SqlFilter, CompiledQuery)> {
    if q.window.is_some() {
        return Err(StoreError::unsupported("live time windows", "6c"));
    }
    let cq = CompiledQuery::new(q, default_group_by, marker_kinds).map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
    let mut exact = q.filters.is_empty() && q.groups.is_empty() && q.ql.trim().is_empty();
    let mut structural = Vec::new();
    if !q.sources.is_empty() {
        debug_assert!(q.sources.len() <= MAX_QUERY_SOURCES, "checked by CompiledQuery::new");
        let ids: Vec<String> = q.sources.iter().map(u32::to_string).collect();
        structural.push(format!("e.source IN ({})", ids.join(", ")));
    }
    if let Some(from) = q.from {
        structural.push(format!("e.ts >= {from}"));
    }
    if let Some(to) = q.to {
        structural.push(format!("e.ts <= {to}"));
    }
    let mut content = Vec::new();
    if !q.levels.is_empty() {
        let codes: Vec<String> = q.levels.iter().map(|l| l.index().to_string()).collect();
        content.push(format!("e.level IN ({})", codes.join(", ")));
    }
    let text = q.text.trim();
    if !text.is_empty() {
        match (q.regex, lit(&contains_pattern(text))) {
            (false, Some(p)) => {
                let op = if q.case_sensitive { "LIKE" } else { "ILIKE" };
                content.push(format!(
                    "e.message {op} {p} ESCAPE '\\' OR e.exception {op} {p} ESCAPE '\\' OR e.template {op} {p} ESCAPE '\\'"
                ));
            }
            _ => exact = false,
        }
    }
    if !q.markers.is_empty() {
        let mask = q
            .markers
            .iter()
            .filter_map(|m| marker_kinds.iter().position(|k| k == m))
            .fold(0u64, |m, i| m | (1 << i));
        content.push(if mask == 0 {
            "FALSE".into()
        } else {
            format!("(e.markers & {mask}::UBIGINT) <> 0")
        });
    }
    Ok((
        SqlFilter {
            structural,
            content,
            exact,
        },
        cq,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::batch::ColumnType;
    use imt_core::model::Level;
    use imt_core::query::FieldFilter;

    #[test]
    fn literals_and_identifiers_are_escaped() {
        assert_eq!(lit("it's").unwrap(), "'it''s'");
        assert_eq!(lit("a\0b"), None);
        assert_eq!(ident("f_a\"b"), "\"f_a\"\"b\"");
        assert_eq!(contains_pattern("50%_off\\"), "%50\\%\\_off\\\\%");
        assert_eq!(
            order_by(true),
            "ORDER BY coalesce(e.ts, 9223372036854775807) DESC, e.source DESC, e.line DESC, e.id DESC"
        );
    }

    #[test]
    fn simple_filters_compile_exactly() {
        let q = Filter {
            from: Some(10),
            to: Some(20),
            levels: vec![Level::Warning, Level::Error],
            text: " it's ".into(),
            markers: vec!["start".into()],
            sources: vec![1, 2],
            ..Filter::default()
        };
        let (f, _) = compile(&q, &[], &["stop".into(), "start".into()]).unwrap();
        assert!(f.exact);
        assert_eq!(f.structural_sql(), "(e.source IN (1, 2)) AND (e.ts >= 10) AND (e.ts <= 20)");
        let content = f.content_sql();
        assert!(content.contains("e.level IN (3, 4)"), "{content}");
        assert!(content.contains("e.message ILIKE '%it''s%' ESCAPE '\\'"), "{content}");
        assert!(content.contains("(e.markers & 2::UBIGINT) <> 0"), "{content}");
        let (f, _) = compile(
            &Filter {
                markers: vec!["nope".into()],
                ..Filter::default()
            },
            &[],
            &[],
        )
        .unwrap();
        assert_eq!(f.content_sql(), "(FALSE)");
        assert_eq!(compile(&Filter::default(), &[], &[]).unwrap().0.all_sql(), "(TRUE) AND (TRUE)");
    }

    #[test]
    fn other_criteria_fall_back_to_the_scan() {
        for q in [
            Filter {
                text: "a.c".into(),
                regex: true,
                ..Filter::default()
            },
            Filter {
                filters: vec![FieldFilter {
                    field: "A".into(),
                    op: "exists".into(),
                    value: None,
                }],
                ..Filter::default()
            },
            Filter {
                groups: vec!["x".into()],
                ..Filter::default()
            },
            Filter {
                ql: "level >= warning".into(),
                ..Filter::default()
            },
            Filter {
                text: "nul\0".into(),
                ..Filter::default()
            },
        ] {
            assert!(!compile(&q, &[], &[]).unwrap().0.exact, "{q:?}");
        }
        let bad = Filter {
            text: "(".into(),
            regex: true,
            ..Filter::default()
        };
        assert!(matches!(compile(&bad, &[], &[]), Err(StoreError::Invalid(_))));
        let live = Filter {
            window: Some(imt_core::query::TimeWindow::Live { last_ms: 5 }),
            ..Filter::default()
        };
        assert!(matches!(compile(&live, &[], &[]), Err(StoreError::Unsupported { .. })));
    }

    #[test]
    fn field_text_and_group_keys() {
        let schema = EventSchema::new([("Ms", ColumnType::Float64)]).unwrap();
        assert_eq!(text_expr("@source", &schema).unwrap(), "s.name");
        assert_eq!(
            text_expr("Ms", &schema).unwrap(),
            "coalesce(CAST(e.\"f_Ms\" AS VARCHAR), e.extra['Ms'])"
        );
        assert_eq!(text_expr("Host", &schema).unwrap(), "e.extra['Host']");
        assert_eq!(text_expr("a\0", &schema), None);
        for p in ["@level", "@message", "@exception", "@template", "@line"] {
            assert!(text_expr(p, &schema).is_some());
        }
        assert_eq!(group_key_expr(&[], &schema).unwrap(), "'all'");
        let k = group_key_expr(&["Host".into(), "Ms".into()], &schema).unwrap();
        assert!(k.starts_with("CASE WHEN coalesce(e.extra['Host'], '') <> '' OR"), "{k}");
        assert!(k.ends_with("ELSE s.name END"), "{k}");
    }
}
