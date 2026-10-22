//! Rendering results (pure and bounded): tables as aligned text, CSV or JSON, and the JSON envelopes scripts read.

use crate::args::TableFormat;
use serde_json::{json, Map, Value};
use std::io::Write;

/// Characters of one cell; longer text is cut with an ellipsis (a log message can be megabytes).
pub const MAX_CELL_CHARS: usize = 500;

/// A table of results; `total` is how many rows the engine had, `rows` the (bounded) ones kept.
#[derive(Debug, Clone, PartialEq)]
pub struct Table {
    pub columns: Vec<String>,
    pub rows: Vec<Vec<Value>>,
    pub total: usize,
}

impl Table {
    pub fn truncated(&self) -> bool {
        self.rows.len() < self.total
    }
}

/// A cell as text, cut at [`MAX_CELL_CHARS`]; control characters become spaces so a row stays one line.
pub fn cell_text(v: &Value) -> String {
    let raw = match v {
        Value::Null => String::new(),
        Value::String(s) => s.clone(),
        other => other.to_string(),
    };
    let mut out: String = raw
        .chars()
        .take(MAX_CELL_CHARS)
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if raw.chars().count() > MAX_CELL_CHARS {
        out.push('…');
    }
    out
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_string()
    }
}

/// Writes `table` in `format`. The note about a cut result goes to `note` (standard error), never into the data.
pub fn write_table(table: &Table, format: TableFormat, out: &mut dyn Write, note: &mut dyn Write) -> std::io::Result<()> {
    match format {
        TableFormat::Json => {
            let rows: Vec<Value> = table
                .rows
                .iter()
                .map(|r| {
                    let mut m = Map::new();
                    for (c, v) in table.columns.iter().zip(r) {
                        m.insert(c.clone(), clip_value(v));
                    }
                    Value::Object(m)
                })
                .collect();
            let v = json!({"ok": true, "columns": table.columns, "rows": rows, "total": table.total, "truncated": table.truncated()});
            writeln!(out, "{v}")?;
        }
        TableFormat::Csv => {
            writeln!(out, "{}", table.columns.iter().map(|c| csv_field(c)).collect::<Vec<_>>().join(","))?;
            for r in &table.rows {
                writeln!(out, "{}", r.iter().map(|v| csv_field(&cell_text(v))).collect::<Vec<_>>().join(","))?;
            }
        }
        TableFormat::Table => {
            let cells: Vec<Vec<String>> = table.rows.iter().map(|r| r.iter().map(cell_text).collect()).collect();
            let widths: Vec<usize> = (0..table.columns.len())
                .map(|i| {
                    cells
                        .iter()
                        .filter_map(|r| r.get(i))
                        .map(|c| c.chars().count())
                        .chain([table.columns[i].chars().count()])
                        .max()
                        .unwrap_or(0)
                })
                .collect();
            let line = |row: &[String]| -> String {
                row.iter()
                    .zip(&widths)
                    .map(|(c, w)| format!("{c:<w$}"))
                    .collect::<Vec<_>>()
                    .join("  ")
                    .trim_end()
                    .to_string()
            };
            writeln!(out, "{}", line(&table.columns))?;
            for r in &cells {
                writeln!(out, "{}", line(r))?;
            }
        }
    }
    if table.truncated() {
        writeln!(
            note,
            "{} of {} rows shown; raise --limit (at most {}) or narrow the query",
            table.rows.len(),
            table.total,
            crate::args::MAX_LIMIT
        )?;
    }
    Ok(())
}

fn clip_value(v: &Value) -> Value {
    match v {
        Value::String(s) if s.chars().count() > MAX_CELL_CHARS => Value::String(cell_text(v)),
        other => other.clone(),
    }
}

/// The JSON object printed when a `--json` command fails.
pub fn error_json(message: &str) -> Value {
    json!({"ok": false, "error": message})
}

#[cfg(test)]
mod tests {
    use super::*;

    fn table(rows: usize, total: usize) -> Table {
        Table {
            columns: vec!["a".into(), "text".into()],
            rows: (0..rows).map(|i| vec![json!(i), json!(format!("x,\"{i}\"\nline"))]).collect(),
            total,
        }
    }

    fn render(t: &Table, f: TableFormat) -> (String, String) {
        let (mut out, mut note) = (Vec::new(), Vec::new());
        write_table(t, f, &mut out, &mut note).unwrap();
        (String::from_utf8(out).unwrap(), String::from_utf8(note).unwrap())
    }

    #[test]
    fn csv_quotes_and_keeps_notes_out_of_the_data() {
        let (out, note) = render(&table(2, 5), TableFormat::Csv);
        assert_eq!(out.lines().next(), Some("a,text"));
        assert!(out.contains("0,\"x,\"\"0\"\" line\""), "{out}");
        assert!(note.contains("2 of 5 rows"), "{note}");
    }

    #[test]
    fn json_is_one_object_with_totals() {
        let (out, note) = render(&table(1, 1), TableFormat::Json);
        let v: Value = serde_json::from_str(out.trim()).unwrap();
        assert_eq!(v["ok"], true);
        assert_eq!(v["rows"][0]["a"], 0);
        assert_eq!(v["truncated"], false);
        assert!(note.is_empty());
    }

    #[test]
    fn table_aligns_columns() {
        let t = Table {
            columns: vec!["name".into(), "n".into()],
            rows: vec![vec![json!("alpha"), json!(1)], vec![json!("b"), json!(22)]],
            total: 2,
        };
        let (out, _) = render(&t, TableFormat::Table);
        assert_eq!(out, "name   n\nalpha  1\nb      22\n");
    }

    #[test]
    fn long_cells_are_cut() {
        let long = Value::String("é".repeat(MAX_CELL_CHARS + 10));
        let cut = cell_text(&long);
        assert_eq!(cut.chars().count(), MAX_CELL_CHARS + 1);
        assert!(cut.ends_with('…'));
        assert_eq!(cell_text(&Value::Null), "");
        assert_eq!(cell_text(&json!("a\tb")), "a b");
        assert_eq!(error_json("no")["ok"], false);
    }
}
