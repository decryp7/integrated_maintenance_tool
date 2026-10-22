//! Golden tests: "import these sample files → expect these events". Results are written as readable JSON next to
//! the tests and compared on every run; `DEBRIEF_BLESS=1 cargo test` writes them (review the diff before committing).

use debrief_plugin_api::Record;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

/// A folder of golden files.
#[derive(Debug, Clone)]
pub struct Golden {
    dir: PathBuf,
}

/// The JSON of one record as golden files keep it (its order of fields kept).
pub fn record_json(r: &Record) -> Value {
    let mut v = json!({
        "line": r.line,
        "ts": r.ts,
        "level": format!("{:?}", r.level),
        "message": r.message,
    });
    if let Some(t) = &r.template {
        v["template"] = json!(t);
    }
    if let Some(e) = &r.exception {
        v["exception"] = json!(e);
    }
    if !r.fields.is_empty() {
        v["fields"] = Value::Object(r.fields.clone());
    }
    v
}

impl Golden {
    /// Golden files in `dir` (relative paths are relative to the crate being tested).
    pub fn new(dir: impl AsRef<Path>) -> Golden {
        Golden {
            dir: dir.as_ref().to_path_buf(),
        }
    }

    /// The file of golden `name`.
    pub fn path(&self, name: &str) -> PathBuf {
        self.dir.join(format!("{name}.json"))
    }

    /// Compares `records` with golden `name`, or writes it when `DEBRIEF_BLESS` is set or the file does not exist
    /// yet (a first run). Panics with a readable difference otherwise.
    #[track_caller]
    pub fn check(&self, name: &str, records: &[Record]) {
        let value = Value::Array(records.iter().map(record_json).collect());
        self.check_value(name, &value);
    }

    /// [`Golden::check`] for any JSON value (annotations, tool results, transform outputs).
    #[track_caller]
    pub fn check_value(&self, name: &str, value: &Value) {
        let path = self.path(name);
        let text = format!("{}\n", serde_json::to_string_pretty(value).unwrap_or_default());
        let bless = std::env::var_os("DEBRIEF_BLESS").is_some();
        let old = std::fs::read_to_string(&path).ok().map(|t| t.replace("\r\n", "\n"));
        match old {
            Some(old) if !bless => {
                if old != text {
                    let first = old
                        .lines()
                        .zip(text.lines())
                        .position(|(a, b)| a != b)
                        .unwrap_or(old.lines().count().min(text.lines().count()));
                    panic!(
                        "golden {} differs from line {}:\n  expected: {}\n  actual:   {}\nRun with DEBRIEF_BLESS=1 to accept the new output.",
                        path.display(),
                        first + 1,
                        old.lines().nth(first).unwrap_or("<end>"),
                        text.lines().nth(first).unwrap_or("<end>")
                    );
                }
            }
            _ => {
                if let Some(parent) = path.parent() {
                    let _ = std::fs::create_dir_all(parent);
                }
                if let Err(e) = std::fs::write(&path, text) {
                    panic!("cannot write golden {}: {e}", path.display());
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn goldens_are_written_once_then_compared() {
        let t = tempfile::tempdir().unwrap();
        let g = Golden::new(t.path());
        let mut r = Record::new(3);
        r.message = "hello".into();
        r.ts = Some(1);
        r.fields.insert("Host".into(), json!("web01"));
        g.check("one", std::slice::from_ref(&r));
        assert!(std::fs::read_to_string(g.path("one")).unwrap().contains("\"Host\": \"web01\""));
        g.check("one", std::slice::from_ref(&r));
        r.message = "changed".into();
        let failed = std::panic::catch_unwind(|| g.check("one", std::slice::from_ref(&r)));
        assert!(failed.is_err());
        let v = record_json(&r);
        assert_eq!(v["level"], "Information");
        assert!(v.get("template").is_none());
    }
}
