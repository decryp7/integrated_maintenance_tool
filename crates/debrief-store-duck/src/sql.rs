//! Compiling the predicate language ([`Expr`]) to SQL.
//!
//! Values are written as escaped literals (numbers formatted, text in single quotes with quotes
//! doubled), so a clause is self-contained and can be reused in any position of a statement. Every
//! compiled predicate is `TRUE` or `FALSE`, never `NULL`, so `NOT` keeps its meaning. A predicate SQL
//! cannot express exactly (a regex, text with a NUL character) compiles to `None`; the conjuncts that do
//! compile prefilter the events and the engine's evaluator decides the rest on a batch scan.
//!
//! Statements read events `e` joined with their source `s` ([`FROM`]).

use debrief_core::batch::{ColumnType, EventSchema};
use debrief_core::instance::KeyScopes;
use debrief_core::model::Level;
use debrief_core::ports::{Filter, StoreError, StoreResult};
use debrief_core::ql::{CmpOp, Expr, NumOp, Operand};
use debrief_core::query::CompiledQuery;
use debrief_core::row::{FieldKey, Pseudo};
use std::sync::Arc;

/// Events with their source (for `@source`): `events_wide`, so promoted `p_<key>` columns can be read.
pub const FROM: &str = "events_wide e JOIN sources s ON s.id = e.source";

/// Where a statement reads fields from: the event schema (declared columns) and the keys every segment
/// promotes to a text column (`p_<key>`, a copy of the value in `extra`; see `crate::promote`).
pub trait FieldSource {
    fn events(&self) -> &EventSchema;
    /// Whether every segment has the key as a text column (then it holds every value).
    fn promoted_text(&self, _key: &str) -> bool {
        false
    }
    /// A SQL condition on `e.id` holding the events with every word of a smart search (from the full-text
    /// index), or `None` when the index cannot answer (then SQL narrows by substrings of the words and the scan
    /// checks the words).
    fn word_hits(&self, _words: &debrief_core::words::WordQuery) -> Option<String> {
        None
    }
}

impl FieldSource for EventSchema {
    fn events(&self) -> &EventSchema {
        self
    }
}

impl FieldSource for std::sync::Arc<EventSchema> {
    fn events(&self) -> &EventSchema {
        self
    }
}

/// The schema and the promotions of the current segments.
#[derive(Debug, Clone)]
pub struct Fields {
    pub schema: std::sync::Arc<EventSchema>,
    /// Keys promoted as text in every segment.
    pub promoted_text: std::collections::BTreeSet<String>,
}

impl FieldSource for Fields {
    fn events(&self) -> &EventSchema {
        &self.schema
    }
    fn promoted_text(&self, key: &str) -> bool {
        self.promoted_text.contains(key)
    }
}

/// The wide columns a read needs: the message, and which named fields (`None`: all). The rest read
/// as empty, so sorts and scans carry only what the analysis uses (a sort of whole rows exceeds the
/// memory limit at 10M events).
#[derive(Debug, Clone, Default)]
pub struct Columns {
    pub message: bool,
    pub fields: Option<Vec<String>>,
}

impl Columns {
    /// Everything but `raw`.
    pub fn all() -> Columns {
        Columns {
            message: true,
            fields: None,
        }
    }

    pub fn only(message: bool, fields: impl IntoIterator<Item = impl Into<String>>) -> Columns {
        Columns {
            message,
            fields: Some(fields.into_iter().map(Into::into).collect()),
        }
    }
}

fn sql_type(t: ColumnType) -> &'static str {
    match t {
        ColumnType::Utf8 => "VARCHAR",
        ColumnType::Int64 => "BIGINT",
        ColumnType::Float64 => "DOUBLE",
        ColumnType::Boolean => "BOOLEAN",
    }
}

/// The select list of event rows with `cols`: the event schema's columns in order (never `raw`, which
/// only re-deriving reads; promoted columns are left out), unneeded wide columns as empty values.
pub fn row_columns(schema: &EventSchema, cols: &Columns) -> String {
    let wanted = |name: &str| cols.fields.as_ref().is_none_or(|f| f.iter().any(|x| x == name));
    let mut out = Vec::new();
    for field in schema.arrow().fields() {
        let name = field.name().as_str();
        let col = format!("e.{}", ident(name));
        out.push(match name {
            "raw" => "'' AS raw".to_string(),
            "message" if !cols.message => "'' AS message".to_string(),
            "extra" => match &cols.fields {
                None => col,
                Some(fields) => {
                    let keys: Vec<String> = fields.iter().filter(|f| !f.starts_with('@')).filter_map(|f| lit(f)).collect();
                    if keys.is_empty() {
                        "CAST(map() AS MAP(VARCHAR, VARCHAR)) AS extra".into()
                    } else {
                        format!(
                            "map_from_entries(list_filter(map_entries(e.extra), lambda x: list_contains([{}], x.key))) AS extra",
                            keys.join(", ")
                        )
                    }
                }
            },
            _ => match schema.declared().iter().find(|c| c.column == name) {
                Some(c) if !wanted(&c.name) => format!("CAST(NULL AS {}) AS {}", sql_type(c.ty), ident(name)),
                _ => col,
            },
        });
    }
    out.join(", ")
}

/// Every event column but `raw` (see [`row_columns`]).
pub fn rows_sql(schema: &EventSchema) -> String {
    row_columns(schema, &Columns::all())
}

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
pub(crate) fn double(v: f64) -> Option<String> {
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

/// Finds `text` anywhere in the message, exception or template.
fn substring_sql(text: &str, case_sensitive: bool) -> Option<String> {
    let p = lit(&contains_pattern(text))?;
    let op = if case_sensitive { "LIKE" } else { "ILIKE" };
    Some(format!(
        "(coalesce(e.message {op} {p} ESCAPE '\\', FALSE) OR coalesce(e.exception {op} {p} ESCAPE '\\', FALSE) OR coalesce(e.template {op} {p} ESCAPE '\\', FALSE))"
    ))
}

/// For a smart search the index cannot answer (still indexing, unavailable, too many hits): a SQL condition
/// every match passes (each word as a substring, or the whole text as one), so SQL narrows the events and the
/// scan checks the words exactly.
fn search_superset(e: &Expr) -> Option<String> {
    let Expr::Search(s) = e else { return None };
    let words = s.words.as_ref()?;
    let parts = words
        .terms
        .iter()
        .chain(&words.prefixes)
        .chain(&words.phrases)
        .map(|w| substring_sql(w, false))
        .collect::<Option<Vec<_>>>()?;
    let words = and(&parts);
    if s.substring {
        Some(format!("({words}) OR {}", substring_sql(s.literal.as_deref()?, s.case_sensitive)?))
    } else {
        Some(words)
    }
}

/// SQL text of a field or pseudo-field, as `RowRef::text` gives it (NULL when missing); `None` when it
/// cannot be expressed.
pub fn text_expr(name: &str, schema: &dyn FieldSource) -> Option<String> {
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
        "@id" => "CAST(e.id AS VARCHAR)".into(),
        _ => {
            let key = lit(name)?;
            let events = schema.events();
            match events.declared_index(name) {
                Some(i) => format!(
                    "coalesce(CAST(e.{} AS VARCHAR), e.extra[{key}])",
                    ident(&events.declared()[i].column)
                ),
                // Every segment keeps the key in a text column: read it instead of the whole map.
                None if schema.promoted_text(name) => format!("e.{}", ident(&crate::promote::column(name))),
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
pub fn number_expr(name: &str, schema: &dyn FieldSource) -> Option<String> {
    Some(match name {
        "@line" => "CAST(e.line AS DOUBLE)".into(),
        "@id" => "CAST(e.id AS DOUBLE)".into(),
        n if Pseudo::parse(n).is_some() => "CAST(NULL AS DOUBLE)".into(),
        _ => {
            let key = lit(name)?;
            let events = schema.events();
            match events.declared_index(name).map(|i| &events.declared()[i]) {
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

/// SQL of the instance key, as `debrief_core::query::group_key`: the values from the first present field to the
/// last present one joined with " · " ("–" for a missing one in between), led by the source name when the
/// first field is missing; the source name when none is present; "all" without fields.
///
/// An identity field that applies only to some sources (`scopes`) is missing on the events of the others.
pub fn group_key_expr(fields: &[String], scopes: &KeyScopes, schema: &dyn FieldSource) -> Option<String> {
    if fields.is_empty() {
        return Some("'all'".into());
    }
    let texts: Vec<String> = fields
        .iter()
        .map(|f| {
            let text = text_expr(f, schema)?;
            Some(match scopes.sources_of(f) {
                None => text,
                Some(names) => {
                    let names = names.iter().map(|n| lit(n)).collect::<Option<Vec<_>>>()?;
                    format!("(CASE WHEN s.name IN ({}) THEN {text} END)", names.join(", "))
                }
            })
        })
        .collect::<Option<_>>()?;
    let present: Vec<String> = texts.iter().map(|t| format!("coalesce({t}, '') <> ''")).collect();
    let any = |range: &[String]| {
        if range.is_empty() {
            "FALSE".to_string()
        } else {
            format!("({})", range.join(" OR "))
        }
    };
    let mut parts = vec![format!("CASE WHEN NOT {} THEN concat(s.name, ' · ') ELSE '' END", present[0])];
    for (i, t) in texts.iter().enumerate() {
        // Between the first and the last present value; a separator unless it is the first one written.
        let inside = format!("{} AND {}", any(&present[..=i]), any(&present[i..]));
        parts.push(format!(
            "CASE WHEN {inside} THEN concat(CASE WHEN {} THEN ' · ' ELSE '' END, CASE WHEN {} THEN {t} ELSE '–' END) ELSE '' END",
            any(&present[..i]),
            present[i]
        ));
    }
    Some(format!(
        "CASE WHEN {} THEN concat({}) ELSE s.name END",
        any(&present),
        parts.join(", ")
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

/// A query-language comparison (see `debrief_core::ql::Expr::Cmp`): by severity for levels, numerically
/// when both sides are numbers, else as text; a missing field only satisfies `!=`.
fn cmp_sql(field: &FieldKey, op: CmpOp, rhs: &Operand, schema: &dyn FieldSource) -> Option<String> {
    if *field == FieldKey::Pseudo(Pseudo::Id) {
        if let Some(id) = id_operand(rhs) {
            return Some(format!("(e.id {} {id})", sql_op(op)));
        }
    }
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

/// An operand that is a whole, non-negative number: an event id as `@id` compares it (exactly, as an integer).
fn id_operand(o: &Operand) -> Option<u64> {
    let n = o.lit.number?;
    (o.level.is_none() && n.fract() == 0.0 && (0.0..9.0e15).contains(&n)).then_some(n as u64)
}

fn level_named(value: &str) -> Option<Level> {
    Level::ALL.iter().copied().find(|l| l.as_str().eq_ignore_ascii_case(value))
}

/// The SQL of `e` (never NULL), or `None` when SQL cannot express it exactly. `group_by` is the
/// query's instance key (for [`Expr::Group`]).
pub fn expr_sql(e: &Expr, schema: &dyn FieldSource, group_by: &[String]) -> Option<String> {
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
        Expr::In { field, set } if *field == FieldKey::Pseudo(Pseudo::Id) && set.iter().all(|o| id_operand(o).is_some()) => {
            // A result set's ids (`@id in (...)`): one list the engine looks up, not one comparison per id.
            let ids: Vec<String> = set.iter().filter_map(id_operand).map(|i| i.to_string()).collect();
            if ids.is_empty() {
                "FALSE".into()
            } else {
                format!("(e.id IN ({}))", ids.join(", "))
            }
        }
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
            let text = substring_sql(s.literal.as_deref()?, s.case_sensitive)?;
            let Some(words) = &s.words else { return Some(text) };
            // Word hits from the index (else not expressible: see `search_superset`); phrases as written.
            let mut parts = vec![schema.word_hits(words)?];
            for p in &words.phrases {
                parts.push(substring_sql(p, false)?);
            }
            let words = and(&parts);
            if s.substring {
                format!("({words}) OR {text}")
            } else {
                words
            }
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
        Expr::Group { set, scopes, .. } => {
            if set.is_empty() {
                return Some("FALSE".into());
            }
            let key = group_key_expr(group_by, scopes, schema)?;
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
    /// Files the SQL reads (word hits), kept while the filter is in use.
    pub keep: Vec<std::sync::Arc<crate::hits::HitsFile>>,
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
pub fn compile_query(cq: &CompiledQuery, schema: &dyn FieldSource) -> SqlFilter {
    let mut exact = true;
    let mut part = |exprs: &[Expr]| -> Vec<String> {
        exprs
            .iter()
            .filter_map(|e| {
                let sql = expr_sql(e, schema, &cq.group_by);
                exact &= sql.is_some();
                sql.or_else(|| search_superset(e))
            })
            .collect()
    };
    let structural = part(cq.structural_exprs());
    let content = part(cq.content_exprs());
    SqlFilter {
        structural,
        content,
        exact,
        keep: Vec::new(),
    }
}

/// Validates the filter (as the in-memory store does) and compiles what SQL can express.
#[cfg(test)]
pub fn compile(
    q: &Filter,
    default_group_by: &[String],
    marker_kinds: &[String],
    schema: &dyn FieldSource,
) -> StoreResult<(SqlFilter, CompiledQuery)> {
    compile_scoped(q, default_group_by, &Arc::default(), marker_kinds, schema)
}

/// [`compile`] with the scopes of the default key's identity fields.
pub fn compile_scoped(
    q: &Filter,
    default_group_by: &[String],
    key_scopes: &Arc<KeyScopes>,
    marker_kinds: &[String],
    schema: &dyn FieldSource,
) -> StoreResult<(SqlFilter, CompiledQuery)> {
    if q.window.is_some() {
        return Err(StoreError::unsupported("live time windows", "6c"));
    }
    let cq = CompiledQuery::new_scoped(q, default_group_by, key_scopes, marker_kinds).map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
    Ok((compile_query(&cq, schema), cq))
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_core::query::FieldFilter;

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

    struct Hits;
    impl FieldSource for Hits {
        fn events(&self) -> &EventSchema {
            static S: std::sync::OnceLock<EventSchema> = std::sync::OnceLock::new();
            S.get_or_init(EventSchema::plain)
        }
        fn word_hits(&self, _words: &debrief_core::words::WordQuery) -> Option<String> {
            Some("e.id IN (1, 2)".into())
        }
    }

    #[test]
    fn smart_search_reads_index_hits_or_narrows_for_the_scan() {
        use debrief_core::query::TextMode;
        let q = |text: &str, text_mode| Filter {
            text: text.into(),
            text_mode,
            ..Filter::default()
        };
        // The index answers: exact SQL, phrases checked as written, substrings added on request.
        let (f, _) = compile(&q("timeout \"timed out\"", TextMode::Words), &[], &[], &Hits).unwrap();
        assert!(f.exact);
        let sql = f.content_sql();
        assert!(
            sql.contains("e.id IN (1, 2)") && sql.contains("'%timed out%'") && !sql.contains("'%timeout%'"),
            "{sql}"
        );
        let (f, _) = compile(&q("imeout", TextMode::WordsOrSubstring), &[], &[], &Hits).unwrap();
        assert!(
            f.exact && f.content_sql().contains(") OR (coalesce(e.message ILIKE '%imeout%'"),
            "{}",
            f.content_sql()
        );
        // It cannot: SQL narrows by substrings of the words and the scan checks them.
        let (f, _) = compile(&q("timeout req", TextMode::Words), &[], &[], &plain()).unwrap();
        assert!(!f.exact);
        let sql = f.content_sql();
        assert!(sql.contains("'%timeout%'") && sql.contains("'%req%'"), "{sql}");
        let (f, _) = compile(&q("x", TextMode::WordsOrSubstring), &[], &[], &plain()).unwrap();
        assert!(!f.exact && f.content_sql().contains(") OR (coalesce"));
        // A substring search is unchanged.
        let (f, _) = compile(&q("timeout req", TextMode::Substring), &[], &[], &plain()).unwrap();
        assert!(f.exact && f.content_sql().contains("'%timeout req%'"));
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
            window: Some(debrief_core::query::TimeWindow::Live { last_ms: 5 }),
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
        assert_eq!(group_key_expr(&[], &KeyScopes::default(), &schema).unwrap(), "'all'");
        let k = group_key_expr(&["Host".into(), "Ms".into()], &KeyScopes::default(), &schema).unwrap();
        assert!(k.starts_with("CASE WHEN (coalesce(e.extra['Host'], '') <> '' OR"), "{k}");
        assert!(k.ends_with("ELSE s.name END"), "{k}");
    }

    /// The SQL key equals `debrief_core::query::group_key` for every combination of present and missing fields.
    #[test]
    fn group_key_sql_matches_the_engine() {
        let schema = EventSchema::plain();
        let names: Vec<String> = ["A", "B", "C"].map(String::from).to_vec();
        let expr = group_key_expr(&names, &KeyScopes::default(), &schema).unwrap();
        let conn = duckdb::Connection::open_in_memory().unwrap();
        for mask in 0u8..8 {
            let mut rec = debrief_core::model::Record::new(1);
            let (mut keys, mut values) = (Vec::new(), Vec::new());
            for (i, n) in names.iter().enumerate() {
                if mask & (1 << i) != 0 {
                    rec.fields.insert(n.clone(), format!("v{i}").into());
                    keys.push(format!("'{n}'"));
                    values.push(format!("'v{i}'"));
                }
            }
            let sql = format!(
                "SELECT {expr} FROM (SELECT MAP([{}]::VARCHAR[], [{}]::VARCHAR[]) AS extra) e, (SELECT 'src' AS name) s",
                keys.join(", "),
                values.join(", ")
            );
            let got: String = conn.query_row(&sql, [], |r| r.get(0)).unwrap();
            assert_eq!(got, debrief_core::query::group_key(&names, &rec, "src"), "mask {mask:03b}");
        }
    }
}
