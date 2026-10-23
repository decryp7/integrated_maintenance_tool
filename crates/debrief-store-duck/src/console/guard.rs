//! What the SQL console lets run (ADR 0030): exactly one statement, a query (`SELECT`, `WITH`, `FROM`, `VALUES`,
//! `TABLE`, `DESCRIBE`, `SUMMARIZE`, `PIVOT`...), reading only the console's views and the allowed table functions.
//! Checked twice, both in Rust before anything runs: a lexical pass names the offending keyword and its position;
//! then DuckDB's own parser (`json_serialize_sql`, which serializes SELECT statements only) gives the syntax tree,
//! which is walked for table functions and file names. The sandbox of the console's database (no file, network or
//! extension access, locked configuration) stands behind both.

use debrief_core::ql::QlError;
use serde_json::Value;

/// Table functions a console query may call (they read nothing but their arguments).
pub const TABLE_FUNCTIONS: [&str; 4] = ["range", "generate_series", "unnest", "generate_subscripts"];

/// Statements that read: what the first keyword of a console query may be.
const QUERY_KEYWORDS: [&str; 10] = [
    "select",
    "with",
    "from",
    "values",
    "table",
    "describe",
    "summarize",
    "pivot",
    "unpivot",
    "show",
];

/// A query the console runs, after the checks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checked {
    /// The statement without a leading `EXPLAIN` or trailing `;`.
    pub sql: String,
    /// The text asked for the plan (`EXPLAIN SELECT ...`).
    pub explain: bool,
}

fn refuse(message: impl Into<String>, offset: usize, len: usize) -> QlError {
    QlError::new(message, offset, len)
}

/// A word or a symbol of the SQL text, with its byte offset.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Token {
    text: String,
    offset: usize,
}

/// The words and `;`s outside quotes and comments (`'...'`, `"..."`, `$tag$...$tag$`, `--`, `/* */`). An
/// unterminated quote or comment ends the text (DuckDB's parser reports it).
fn tokens(sql: &str) -> Vec<Token> {
    let b = sql.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'\'' | b'"' => {
                i += 1;
                while i < b.len() {
                    if b[i] == c {
                        // A doubled quote is an escaped one.
                        if b.get(i + 1) == Some(&c) {
                            i += 2;
                            continue;
                        }
                        break;
                    }
                    i += 1;
                }
                i += 1;
            }
            b'-' if b.get(i + 1) == Some(&b'-') => {
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                i += 2;
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i += 2;
            }
            b'$' => {
                // `$tag$ ... $tag$` (a parameter `$1` has no closing `$`).
                let start = i;
                let mut j = i + 1;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                if j < b.len() && b[j] == b'$' {
                    let tag = &sql[start..=j];
                    match sql[j + 1..].find(tag) {
                        Some(end) => i = j + 1 + end + tag.len(),
                        None => i = b.len(),
                    }
                } else {
                    i = j;
                }
            }
            b';' | b'(' => {
                out.push(Token {
                    text: (c as char).to_string(),
                    offset: i,
                });
                i += 1;
            }
            c if c.is_ascii_alphabetic() || c == b'_' => {
                let start = i;
                while i < b.len() && (b[i].is_ascii_alphanumeric() || b[i] == b'_') {
                    i += 1;
                }
                out.push(Token {
                    text: sql[start..i].to_ascii_lowercase(),
                    offset: start,
                });
            }
            _ => i += 1,
        }
    }
    out
}

/// The lexical checks: one statement, a query; a leading `EXPLAIN` becomes [`Checked::explain`].
pub fn lexical(sql: &str) -> Result<Checked, QlError> {
    let toks = tokens(sql);
    // Statements: runs of tokens between `;`s that hold anything.
    let mut statements: Vec<&[Token]> = Vec::new();
    let mut start = 0;
    for (i, t) in toks.iter().enumerate() {
        if t.text == ";" {
            if i > start {
                statements.push(&toks[start..i]);
            }
            start = i + 1;
        }
    }
    if start < toks.len() {
        statements.push(&toks[start..]);
    }
    let Some(first) = statements.first() else {
        return Err(refuse("Write a SELECT query.", 0, 0));
    };
    if let Some(second) = statements.get(1) {
        let at = &second[0];
        return Err(refuse(
            format!(
                "Only one statement runs at a time: remove the second one (it starts with {}).",
                at.text.to_uppercase()
            ),
            at.offset,
            at.text.len(),
        ));
    }
    let (explain, body) = match first[0].text.as_str() {
        "explain" => match first.get(1) {
            Some(t) if t.text == "analyze" || t.text == "analyse" => {
                return Err(refuse(
                    "EXPLAIN ANALYZE runs the query; use the Explain switch for the plan.",
                    t.offset,
                    t.text.len(),
                ))
            }
            Some(t) => (true, t),
            None => return Err(refuse("EXPLAIN needs a query after it.", first[0].offset, first[0].text.len())),
        },
        _ => (false, &first[0]),
    };
    if body.text != "(" && !QUERY_KEYWORDS.contains(&body.text.as_str()) {
        return Err(refuse(
            format!(
                "The SQL console is read-only: {} is not allowed. Write a SELECT query.",
                body.text.to_uppercase()
            ),
            body.offset,
            body.text.len(),
        ));
    }
    // The statement as written, without the `EXPLAIN` and the `;` after it.
    let end = toks
        .iter()
        .find(|t| t.text == ";" && t.offset > body.offset)
        .map_or(sql.len(), |t| t.offset);
    let text = sql[body.offset..end].trim_end().to_string();
    Ok(Checked { sql: text, explain })
}

/// Every object of a JSON tree, depth first.
fn walk<'a>(v: &'a Value, visit: &mut dyn FnMut(&'a serde_json::Map<String, Value>) -> Result<(), QlError>) -> Result<(), QlError> {
    match v {
        Value::Object(map) => {
            visit(map)?;
            for child in map.values() {
                walk(child, visit)?;
            }
            Ok(())
        }
        Value::Array(items) => items.iter().try_for_each(|i| walk(i, visit)),
        _ => Ok(()),
    }
}

/// The byte offset DuckDB records for a node (absent: the start).
fn location(node: &serde_json::Map<String, Value>) -> usize {
    node.get("query_location")
        .and_then(Value::as_u64)
        .filter(|l| *l < u32::MAX as u64)
        .map_or(0, |l| l as usize)
}

/// The checks on DuckDB's syntax tree of `sql` (`json_serialize_sql`'s output `tree`): it parsed as exactly one
/// SELECT statement, whose table functions are allowed and whose tables are not file names.
pub fn tree(sql: &str, tree: &Value) -> Result<(), QlError> {
    if tree.get("error").and_then(Value::as_bool) != Some(false) {
        let message = tree
            .get("error_message")
            .and_then(Value::as_str)
            .unwrap_or("the query could not be read");
        let kind = tree.get("error_type").and_then(Value::as_str).unwrap_or_default();
        let position = tree
            .get("position")
            .and_then(|p| p.as_str().and_then(|s| s.parse().ok()).or(p.as_u64()))
            .unwrap_or(0) as usize;
        return Err(if kind == "parser" {
            refuse(format!("Syntax error: {message}"), position.min(sql.len()), 0)
        } else {
            refuse("The SQL console is read-only: only one SELECT query can run.", 0, 0)
        });
    }
    let statements = tree.get("statements").and_then(Value::as_array).map_or(0, Vec::len);
    if statements != 1 {
        return Err(refuse("Only one statement runs at a time.", 0, 0));
    }
    walk(tree, &mut |node| {
        match node.get("type").and_then(Value::as_str) {
            Some("TABLE_FUNCTION") => {
                let name = node
                    .get("function")
                    .and_then(|f| f.get("function_name"))
                    .and_then(Value::as_str)
                    .unwrap_or_default();
                if !TABLE_FUNCTIONS.contains(&name.to_ascii_lowercase().as_str()) {
                    return Err(refuse(
                        format!(
                            "The table function {name} is not allowed in the SQL console (allowed: {}); query the views events, sources, fields and notes.",
                            TABLE_FUNCTIONS.join(", ")
                        ),
                        location(node),
                        name.len(),
                    ));
                }
            }
            Some("BASE_TABLE") => {
                let name = node.get("table_name").and_then(Value::as_str).unwrap_or_default();
                if name.contains(['.', '/', '\\', ':']) {
                    return Err(refuse(
                        format!("Files cannot be read in the SQL console ({name}); query the views events, sources, fields and notes."),
                        location(node),
                        name.len(),
                    ));
                }
            }
            _ => {}
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn refused(sql: &str) -> QlError {
        lexical(sql).expect_err(sql)
    }

    #[test]
    fn one_query_passes_with_its_explain_and_without_its_semicolon() {
        assert_eq!(
            lexical("  SELECT 1 ; ").unwrap(),
            Checked {
                sql: "SELECT 1".into(),
                explain: false
            }
        );
        assert_eq!(lexical("explain select * from events").unwrap().sql, "select * from events");
        assert!(lexical("EXPLAIN SELECT 1").unwrap().explain);
        for ok in [
            "WITH t AS (SELECT 1) SELECT * FROM t",
            "FROM events",
            "(SELECT 1) UNION (SELECT 2)",
            "VALUES (1)",
            "select ';' as semi, \"a;b\" from t -- ; DROP\n",
            "select $$ ; drop $$",
            "select 1 /* ; drop */",
            "select $1",
            "DESCRIBE events",
            "SUMMARIZE events",
        ] {
            assert!(lexical(ok).is_ok(), "{ok}");
        }
    }

    #[test]
    fn writes_settings_and_files_are_refused_with_the_keyword_and_its_place() {
        for (sql, word) in [
            ("COPY events TO 'x.csv'", "COPY"),
            ("ATTACH 'x.db'", "ATTACH"),
            ("PRAGMA version", "PRAGMA"),
            ("INSTALL httpfs", "INSTALL"),
            ("LOAD httpfs", "LOAD"),
            ("SET memory_limit = '1GB'", "SET"),
            ("CALL pragma_version()", "CALL"),
            ("CREATE TABLE t AS SELECT 1", "CREATE"),
            ("INSERT INTO t VALUES (1)", "INSERT"),
            ("UPDATE t SET a = 1", "UPDATE"),
            ("DELETE FROM t", "DELETE"),
            ("  -- hello\n  DROP VIEW events", "DROP"),
            ("EXPORT DATABASE 'x'", "EXPORT"),
        ] {
            let e = refused(sql);
            assert!(e.message.contains(word) && e.message.contains("read-only"), "{sql}: {}", e.message);
            assert_eq!(&sql[e.offset..e.offset + e.len].to_uppercase(), word, "{sql}");
        }
        let e = refused("SELECT 1; DROP TABLE x");
        assert!(e.message.contains("one statement") && e.message.contains("DROP"), "{}", e.message);
        assert_eq!(e.offset, 10);
        assert!(refused("select 1; select 2").message.contains("one statement"));
        assert!(refused("EXPLAIN ANALYZE SELECT 1").message.contains("Explain switch"));
        assert!(refused("explain").message.contains("needs a query"));
        assert!(refused("   ").message.contains("SELECT"));
        assert!(refused("-- only a comment").message.contains("SELECT"));
        assert!(refused(";;").message.contains("SELECT"));
        assert_eq!(lexical(";; select 1;").unwrap().sql, "select 1");
        // An unterminated quote or comment hides the rest (DuckDB's parser then reports it).
        assert!(lexical("select 'abc").is_ok());
        assert!(lexical("select /* abc").is_ok());
        assert!(lexical("select $a$ never").is_ok());
    }

    fn table(kind: &str, name: &str) -> Value {
        let mut node = serde_json::json!({"type": kind, "query_location": 14});
        if kind == "TABLE_FUNCTION" {
            node["function"] = serde_json::json!({"function_name": name});
        } else {
            node["table_name"] = Value::from(name);
        }
        serde_json::json!({"error": false, "statements": [{"node": {"from_table": node}}]})
    }

    #[test]
    fn the_tree_allows_views_and_harmless_table_functions_only() {
        assert!(tree("x", &table("BASE_TABLE", "events")).is_ok());
        assert!(tree("x", &table("TABLE_FUNCTION", "range")).is_ok());
        assert!(tree("x", &table("TABLE_FUNCTION", "UNNEST")).is_ok());
        for f in [
            "read_csv",
            "read_parquet",
            "glob",
            "parquet_metadata",
            "query",
            "pragma_table_info",
            "read_text",
            "duckdb_settings",
        ] {
            let e = tree("SELECT * FROM xxxxxxxxxx", &table("TABLE_FUNCTION", f)).unwrap_err();
            assert!(e.message.contains(f) && e.message.contains("not allowed"), "{f}: {}", e.message);
            assert_eq!((e.offset, e.len), (14, f.len()));
        }
        for file in ["a.csv", "C:/x/y.parquet", "s3://bucket/x", "..\\x"] {
            let e = tree("x", &table("BASE_TABLE", file)).unwrap_err();
            assert!(e.message.contains("Files cannot be read"), "{file}: {}", e.message);
        }
        let parse = serde_json::json!({"error": true, "error_type": "parser", "error_message": "syntax error at or near \"SELEC\"", "position": "0"});
        let e = tree("SELEC 1", &parse).unwrap_err();
        assert!(e.message.starts_with("Syntax error") && e.offset == 0, "{e:?}");
        let far = serde_json::json!({"error": true, "error_type": "parser", "error_message": "x", "position": 99});
        assert_eq!(tree("abc", &far).unwrap_err().offset, 3);
        let other = serde_json::json!({"error": true, "error_type": "not implemented", "error_message": "Only SELECT statements can be serialized to json!"});
        assert!(tree("x", &other).unwrap_err().message.contains("read-only"));
        let two = serde_json::json!({"error": false, "statements": [{}, {}]});
        assert!(tree("x", &two).unwrap_err().message.contains("one statement"));
        let none = serde_json::json!({"statements": []});
        assert!(tree("x", &none).is_err());
        let deep = serde_json::json!({"error": false, "statements": [{"node": {"where_clause": {"subquery": [table("TABLE_FUNCTION", "glob")]}}}]});
        assert!(tree("x", &deep).unwrap_err().message.contains("glob"));
    }
}
