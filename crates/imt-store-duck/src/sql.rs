//! Compiling the predicate language ([`Expr`]) to SQL.
//!
//! Values are written as escaped literals (numbers formatted, text in single quotes with quotes
//! doubled), so a clause is self-contained and can be reused in any position of a statement. Every
//! compiled predicate is `TRUE` or `FALSE`, never `NULL`, so `NOT` keeps its meaning. A predicate SQL
//! cannot express exactly (a regex, text with a NUL character) compiles to `None`; the conjuncts that do
//! compile prefilter the events and the engine's evaluator decides the rest on a batch scan.
//!
//! Statements read events `e` joined with their source `s` ([`FROM`]).

use imt_core::batch::{ColumnType, EventSchema};
use imt_core::model::Level;
use imt_core::ports::{Filter, StoreError, StoreResult};
use imt_core::ql::{CmpOp, Expr, NumOp, Operand};
use imt_core::query::CompiledQuery;
use imt_core::row::{FieldKey, Pseudo};

/// Events with their source (for `@source`).
pub const FROM: &str = "events e JOIN sources s ON s.id = e.source";

/// The event columns rows are read with: everything but `raw` (left empty), which is wide and only
/// re-deriving fields needs (events are Parquet segments; sorting or aggregating `raw` exceeds the
/// memory limit).
pub const ROWS: &str = "e.* REPLACE ('' AS raw)";

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

/// A finite number as a SQL `DOUBLE` literal.
fn double(v: f64) -> Option<String> {
    v.is_finite().then(|| format!("CAST('{v:?}' AS DOUBLE)"))
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

/// SQL text of a field or pseudo-field, as `RowRef::text` gives it (NULL when missing); `None` when it
/// cannot be expressed.
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

/// A number parsed from text as `value_number` parses it (trimmed, finite), else NULL.
fn parse_number(text: &str) -> String {
    let v = format!("TRY_CAST(trim({text}, ' \t\n\r') AS DOUBLE)");
    format!("CASE WHEN isfinite({v}) THEN {v} END")
}

/// SQL number of a field, as `RowRef::number` gives it (NULL when missing or not numeric).
pub fn number_expr(name: &str, schema: &EventSchema) -> Option<String> {
    Some(match name {
        "@line" => "CAST(e.line AS DOUBLE)".into(),
        n if Pseudo::parse(n).is_some() => "CAST(NULL AS DOUBLE)".into(),
        _ => {
            let key = lit(name)?;
            match schema.declared_index(name).map(|i| &schema.declared()[i]) {
                Some(c) if c.ty != ColumnType::Utf8 => format!(
                    "coalesce(CAST(e.{} AS DOUBLE), {})",
                    ident(&c.column),
                    parse_number(&format!("e.extra[{key}]"))
                ),
                _ => parse_number(&text_expr(name, schema)?),
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

fn sql_op(op: CmpOp) -> &'static str {
    match op {
        CmpOp::Ne => "<>",
        CmpOp::Gt => ">",
        CmpOp::Ge => ">=",
        CmpOp::Lt => "<",
        CmpOp::Le => "<=",
        _ => "=",
    }
}

/// A query-language comparison (see `imt_core::ql::Expr::Cmp`): by severity for levels, numerically
/// when both sides are numbers, else as text; a missing field only satisfies `!=`.
fn cmp_sql(field: &FieldKey, op: CmpOp, rhs: &Operand, schema: &EventSchema) -> Option<String> {
    if let Some(level) = rhs.level {
        return Some(format!("(e.level {} {})", sql_op(op), level.index()));
    }
    let text = text_expr(field.name(), schema)?;
    let lit_text = lit(&rhs.lit.text)?;
    let o = sql_op(op);
    let as_text = format!("{text} {o} {lit_text}");
    let decided = match rhs.lit.number.and_then(double) {
        Some(n) => {
            let num = number_expr(field.name(), schema)?;
            format!("CASE WHEN {num} IS NOT NULL THEN {num} {o} {n} ELSE {as_text} END")
        }
        None => as_text,
    };
    let missing = if op == CmpOp::Ne { "TRUE" } else { "FALSE" };
    Some(format!("(CASE WHEN {text} IS NULL THEN {missing} ELSE {decided} END)"))
}

fn level_named(value: &str) -> Option<Level> {
    Level::ALL.iter().copied().find(|l| l.as_str().eq_ignore_ascii_case(value))
}

/// The SQL of `e` (never NULL), or `None` when SQL cannot express it exactly. `group_by` is the
/// query's instance key (for [`Expr::Group`]).
pub fn expr_sql(e: &Expr, schema: &EventSchema, group_by: &[String]) -> Option<String> {
    let text = |k: &FieldKey| text_expr(k.name(), schema);
    Some(match e {
        Expr::And(v) | Expr::Or(v) => {
            let and = matches!(e, Expr::And(_));
            let parts = v.iter().map(|x| expr_sql(x, schema, group_by)).collect::<Option<Vec<_>>>()?;
            if parts.is_empty() {
                return Some(if and { "TRUE" } else { "FALSE" }.into());
            }
            format!("({})", parts.join(if and { " AND " } else { " OR " }))
        }
        Expr::Not(x) => format!("(NOT {})", expr_sql(x, schema, group_by)?),
        Expr::Const(b) => if *b { "TRUE" } else { "FALSE" }.into(),
        Expr::Cmp { field, op, rhs } => cmp_sql(field, *op, rhs, schema)?,
        Expr::In { field, set } => {
            let parts = set
                .iter()
                .map(|o| cmp_sql(field, CmpOp::Eq, o, schema))
                .collect::<Option<Vec<_>>>()?;
            if parts.is_empty() {
                "FALSE".into()
            } else {
                format!("({})", parts.join(" OR "))
            }
        }
        Expr::Contains { field, needle, negate } => {
            let n = lit(needle)?;
            let has = format!("coalesce(contains(lower({}), {n}), FALSE)", text(field)?);
            if *negate {
                format!("(NOT {has})")
            } else {
                has
            }
        }
        Expr::Regex { .. } => return None,
        Expr::Exists(f) => format!("(coalesce({}, '') <> '')", text(f)?),
        Expr::Missing(f) => format!("(coalesce({}, '') = '')", text(f)?),
        Expr::Text(needle) => {
            let n = lit(needle)?;
            format!(
                "(coalesce(contains(lower(e.message), {n}), FALSE) OR coalesce(contains(lower(e.exception), {n}), FALSE) OR coalesce(contains(lower(e.template), {n}), FALSE))"
            )
        }
        Expr::TextEq { field, value, ignore_case } if *ignore_case => {
            // Only level names are compared ignoring (ASCII) case.
            if field.name() != "@level" {
                return None;
            }
            match level_named(value) {
                Some(l) => format!("(e.level = {})", l.index()),
                None => "FALSE".into(),
            }
        }
        Expr::TextEq { field, value, .. } => format!("coalesce({} = {}, FALSE)", text(field)?, lit(value)?),
        Expr::TextIn { field, set } => {
            if set.is_empty() {
                return Some("FALSE".into());
            }
            let values = set.iter().map(|v| lit(v)).collect::<Option<Vec<_>>>()?;
            format!("coalesce({} IN ({}), FALSE)", text(field)?, values.join(", "))
        }
        Expr::NumCmp { field, op, value } => {
            let o = match op {
                NumOp::Gt => ">",
                NumOp::Lt => "<",
            };
            let v = double(*value)?;
            if field.name() == "@level" {
                format!("(CAST(e.level AS DOUBLE) {o} {v})")
            } else {
                format!("coalesce({} {o} {v}, FALSE)", number_expr(field.name(), schema)?)
            }
        }
        Expr::Search(s) => {
            let p = lit(&contains_pattern(s.literal.as_deref()?))?;
            let op = if s.case_sensitive { "LIKE" } else { "ILIKE" };
            format!(
                "(coalesce(e.message {op} {p} ESCAPE '\\', FALSE) OR coalesce(e.exception {op} {p} ESCAPE '\\', FALSE) OR coalesce(e.template {op} {p} ESCAPE '\\', FALSE))"
            )
        }
        Expr::Levels(mask) => {
            let codes: Vec<String> = (0u8..6).filter(|i| mask & (1 << i) != 0).map(|i| i.to_string()).collect();
            if codes.is_empty() {
                "FALSE".into()
            } else {
                format!("e.level IN ({})", codes.join(", "))
            }
        }
        Expr::Markers(mask) => format!("((e.markers & {mask}::UBIGINT) <> 0)"),
        Expr::Sources(set) => {
            if set.is_empty() {
                "FALSE".into()
            } else {
                let ids: Vec<String> = set.iter().map(u32::to_string).collect();
                format!("e.source IN ({})", ids.join(", "))
            }
        }
        Expr::Time { from, to } => {
            let mut parts = Vec::new();
            if let Some(f) = from {
                parts.push(format!("e.ts >= {f}"));
            }
            if let Some(t) = to {
                parts.push(format!("e.ts <= {t}"));
            }
            if parts.is_empty() {
                "TRUE".into()
            } else {
                format!("coalesce({}, FALSE)", parts.join(" AND "))
            }
        }
        Expr::Group { set, .. } => {
            if set.is_empty() {
                return Some("FALSE".into());
            }
            let key = group_key_expr(group_by, schema)?;
            let values = set.iter().map(|v| lit(v)).collect::<Option<Vec<_>>>()?;
            format!("coalesce({key} IN ({}), FALSE)", values.join(", "))
        }
    })
}

/// A filter compiled to SQL conditions on events `e` (joined with sources `s`).
#[derive(Debug, Clone)]
pub struct SqlFilter {
    /// The structural conjuncts SQL expresses (sources, time range, field filters, instance selection:
    /// they decide which events belong on the timeline).
    pub structural: Vec<String>,
    /// The content conjuncts SQL expresses (levels, markers, the search box, the query language).
    pub content: Vec<String>,
    /// True when the SQL is the whole filter; otherwise matching rows must also pass the
    /// [`CompiledQuery`] on a batch scan.
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

/// Compiles the conjuncts of a lowered query: the SQL of those SQL expresses, and whether that is all.
pub fn compile_query(cq: &CompiledQuery, schema: &EventSchema) -> SqlFilter {
    let mut exact = true;
    let mut part = |exprs: &[Expr]| -> Vec<String> {
        exprs
            .iter()
            .filter_map(|e| {
                let sql = expr_sql(e, schema, &cq.group_by);
                exact &= sql.is_some();
                sql
            })
            .collect()
    };
    let structural = part(cq.structural_exprs());
    let content = part(cq.content_exprs());
    SqlFilter {
        structural,
        content,
        exact,
    }
}

/// Validates the filter (as the in-memory store does) and compiles what SQL can express.
pub fn compile(
    q: &Filter,
    default_group_by: &[String],
    marker_kinds: &[String],
    schema: &EventSchema,
) -> StoreResult<(SqlFilter, CompiledQuery)> {
    if q.window.is_some() {
        return Err(StoreError::unsupported("live time windows", "6c"));
    }
    let cq = CompiledQuery::new(q, default_group_by, marker_kinds).map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
    Ok((compile_query(&cq, schema), cq))
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::query::FieldFilter;

    fn plain() -> EventSchema {
        EventSchema::plain()
    }

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
        assert_eq!(double(f64::NAN), None);
        assert_eq!(double(2.5).unwrap(), "CAST('2.5' AS DOUBLE)");
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
        let (f, _) = compile(&q, &[], &["stop".into(), "start".into()], &plain()).unwrap();
        assert!(f.exact);
        assert_eq!(
            f.structural_sql(),
            "(e.source IN (1, 2)) AND (coalesce(e.ts >= 10 AND e.ts <= 20, FALSE))"
        );
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
            &plain(),
        )
        .unwrap();
        assert_eq!(f.content_sql(), "(FALSE)");
        assert_eq!(
            compile(&Filter::default(), &[], &[], &plain()).unwrap().0.all_sql(),
            "(TRUE) AND (TRUE)"
        );
    }

    #[test]
    fn field_filters_and_the_query_language_compile() {
        for (op, value) in [
            ("eq", Some("x")),
            ("ne", Some("x")),
            ("contains", Some("x")),
            ("notContains", Some("x")),
            ("exists", None),
            ("missing", None),
            ("gt", Some("1")),
            ("lt", Some("1")),
            ("in", Some("a|b")),
        ] {
            let q = Filter {
                filters: vec![FieldFilter {
                    field: "A".into(),
                    op: op.into(),
                    value: value.map(str::to_string),
                }],
                groups: vec!["g".into()],
                ql: "level >= warning and (ms > 5 or not message contains 'x') and Host in (a, 2) and exists(Host)".into(),
                ..Filter::default()
            };
            let (f, _) = compile(&q, &["Host".into()], &[], &plain()).unwrap();
            assert!(f.exact, "{op}: {}", f.all_sql());
        }
        let (f, _) = compile(
            &Filter {
                filters: vec![FieldFilter {
                    field: "@level".into(),
                    op: "eq".into(),
                    value: Some("WARNING".into()),
                }],
                ..Filter::default()
            },
            &[],
            &[],
            &plain(),
        )
        .unwrap();
        assert_eq!(f.structural_sql(), "((e.level = 3))");
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
                    op: "matches".into(),
                    value: Some("x+".into()),
                }],
                ..Filter::default()
            },
            Filter {
                ql: "message ~ 'a+'".into(),
                ..Filter::default()
            },
            Filter {
                text: "nul\0".into(),
                ..Filter::default()
            },
        ] {
            assert!(!compile(&q, &[], &[], &plain()).unwrap().0.exact, "{q:?}");
        }
        let bad = Filter {
            text: "(".into(),
            regex: true,
            ..Filter::default()
        };
        assert!(matches!(compile(&bad, &[], &[], &plain()), Err(StoreError::Invalid(_))));
        let live = Filter {
            window: Some(imt_core::query::TimeWindow::Live { last_ms: 5 }),
            ..Filter::default()
        };
        assert!(matches!(compile(&live, &[], &[], &plain()), Err(StoreError::Unsupported { .. })));
    }

    #[test]
    fn field_text_numbers_and_group_keys() {
        let schema = EventSchema::new([("Ms", ColumnType::Float64), ("Name", ColumnType::Utf8)]).unwrap();
        assert_eq!(text_expr("@source", &schema).unwrap(), "s.name");
        assert_eq!(
            text_expr("Ms", &schema).unwrap(),
            "coalesce(CAST(e.\"f_Ms\" AS VARCHAR), e.extra['Ms'])"
        );
        assert_eq!(text_expr("Host", &schema).unwrap(), "e.extra['Host']");
        assert_eq!(text_expr("a\0", &schema), None);
        assert_eq!(number_expr("a\0", &schema), None);
        assert!(number_expr("Ms", &schema)
            .unwrap()
            .starts_with("coalesce(CAST(e.\"f_Ms\" AS DOUBLE)"));
        assert!(number_expr("Name", &schema).unwrap().contains("e.\"f_Name\""));
        assert_eq!(number_expr("@message", &schema).unwrap(), "CAST(NULL AS DOUBLE)");
        for p in ["@level", "@message", "@exception", "@template", "@line"] {
            assert!(text_expr(p, &schema).is_some());
        }
        assert_eq!(group_key_expr(&[], &schema).unwrap(), "'all'");
        let k = group_key_expr(&["Host".into(), "Ms".into()], &schema).unwrap();
        assert!(k.starts_with("CASE WHEN coalesce(e.extra['Host'], '') <> '' OR"), "{k}");
        assert!(k.ends_with("ELSE s.name END"), "{k}");
    }
}
