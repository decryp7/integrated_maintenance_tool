//! What the model reads off a screenshot, as strict data: the model fills a JSON schema (grammar-constrained
//! decoding), and Rust parses it again with unknown fields refused, every string cleaned and cut, every list
//! bounded and de-duplicated. Nothing the model writes is trusted further than these types (ADR 0025).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

/// Most items of one list, and most characters of one item, by kind.
const MAX_ITEMS: usize = 8;
const MAX_IDS: usize = 16;
const SHORT: usize = 120;
const LONG: usize = 300;

/// What kind of screen the image shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ScreenKind {
    ErrorDialog,
    FrozenWindow,
    Console,
    Dashboard,
    LogViewer,
    Browser,
    #[default]
    Other,
}

/// A time shown on the screen, as written there. (The model writes snake_case; the UI reads camelCase.)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all(serialize = "camelCase"))]
pub struct ShownTime {
    pub text: String,
    /// The text has a date, not only a time of day.
    pub has_date: bool,
}

/// The facts of one screenshot. The model writes them in snake_case (the schema's names); the UI reads camelCase.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields, default, rename_all(serialize = "camelCase"))]
pub struct ScreenFacts {
    pub screen_kind: ScreenKind,
    pub window_title: Option<String>,
    pub application: Option<String>,
    pub timestamps: Vec<ShownTime>,
    pub error_texts: Vec<String>,
    pub error_codes: Vec<String>,
    pub exception_names: Vec<String>,
    pub machine_names: Vec<String>,
    pub user_names: Vec<String>,
    pub server_names: Vec<String>,
    pub ids: Vec<String>,
    pub versions: Vec<String>,
    /// One sentence on what the screen shows.
    pub summary: String,
}

/// The JSON schema the model's answer must follow.
pub fn schema() -> Value {
    let list = |max: usize, len: usize| json!({"type": "array", "maxItems": max, "items": {"type": "string", "maxLength": len}});
    let opt = |len: usize| json!({"type": ["string", "null"], "maxLength": len});
    json!({
        "type": "object",
        "additionalProperties": false,
        "required": ["screen_kind", "window_title", "application", "timestamps", "error_texts", "error_codes",
            "exception_names", "machine_names", "user_names", "server_names", "ids", "versions", "summary"],
        "properties": {
            "screen_kind": {"type": "string", "enum": ["error_dialog", "frozen_window", "console", "dashboard", "log_viewer", "browser", "other"]},
            "window_title": opt(SHORT),
            "application": opt(SHORT),
            "timestamps": {"type": "array", "maxItems": MAX_ITEMS, "items": {
                "type": "object", "additionalProperties": false, "required": ["text", "has_date"],
                "properties": {"text": {"type": "string", "maxLength": 40}, "has_date": {"type": "boolean"}}}},
            "error_texts": list(MAX_ITEMS, LONG),
            "error_codes": list(MAX_IDS, 40),
            "exception_names": list(MAX_ITEMS, SHORT),
            "machine_names": list(MAX_ITEMS, SHORT),
            "user_names": list(MAX_ITEMS, SHORT),
            "server_names": list(MAX_ITEMS, SHORT),
            "ids": list(MAX_IDS, SHORT),
            "versions": list(MAX_ITEMS, 40),
            "summary": {"type": "string", "maxLength": LONG}
        }
    })
}

/// The instructions that go with the image.
pub const EXTRACT_PROMPT: &str = "Read this screenshot and fill in the JSON fields with what is literally visible on it. \
Copy text exactly as shown (window title, error messages, codes such as 0x80004005 or HTTP 503, exception type names, \
machine, user and server names, ids, version numbers, and every date or time shown with has_date telling whether it \
includes a date). Leave a field empty when it is not visible; never guess. screen_kind says what the screen is. \
summary is one short sentence.";

fn clean(s: &str, max: usize) -> Option<String> {
    let t: String = s
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let t: String = t.chars().take(max).collect();
    let t = t.trim().to_string();
    (!t.is_empty()).then_some(t)
}

fn clean_list(items: Vec<String>, max_items: usize, max_len: usize) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for i in items {
        if let Some(c) = clean(&i, max_len) {
            if !out.iter().any(|o| o.eq_ignore_ascii_case(&c)) {
                out.push(c);
            }
        }
        if out.len() == max_items {
            break;
        }
    }
    out
}

impl ScreenFacts {
    /// Parses the model's answer: strict JSON of [`schema`] (a ```json fence is tolerated), then cleaned and
    /// bounded. The error says what was wrong, for the one retry.
    pub fn parse(text: &str) -> Result<ScreenFacts, String> {
        let t = text.trim();
        let t = t
            .strip_prefix("```json")
            .or_else(|| t.strip_prefix("```"))
            .map(|r| r.trim_end().trim_end_matches("```"))
            .unwrap_or(t)
            .trim();
        if t.len() > 64 * 1024 {
            return Err("the answer is far too long".into());
        }
        let raw: ScreenFacts = serde_json::from_str(t).map_err(|e| {
            let mut m = e.to_string();
            m.truncate(200);
            format!("not valid JSON for the schema: {m}")
        })?;
        Ok(raw.bounded())
    }

    /// Every string cleaned (no control characters, whitespace collapsed), cut, de-duplicated and bounded.
    pub fn bounded(self) -> ScreenFacts {
        let mut times: Vec<ShownTime> = Vec::new();
        for t in self.timestamps {
            if let Some(text) = clean(&t.text, 40) {
                if !times.iter().any(|o| o.text == text) && times.len() < MAX_ITEMS {
                    times.push(ShownTime {
                        text,
                        has_date: t.has_date,
                    });
                }
            }
        }
        ScreenFacts {
            screen_kind: self.screen_kind,
            window_title: self.window_title.and_then(|s| clean(&s, SHORT)),
            application: self.application.and_then(|s| clean(&s, SHORT)),
            timestamps: times,
            error_texts: clean_list(self.error_texts, MAX_ITEMS, LONG),
            error_codes: clean_list(self.error_codes, MAX_IDS, 40),
            exception_names: clean_list(self.exception_names, MAX_ITEMS, SHORT),
            machine_names: clean_list(self.machine_names, MAX_ITEMS, SHORT),
            user_names: clean_list(self.user_names, MAX_ITEMS, SHORT),
            server_names: clean_list(self.server_names, MAX_ITEMS, SHORT),
            ids: clean_list(self.ids, MAX_IDS, SHORT),
            versions: clean_list(self.versions, MAX_ITEMS, 40),
            summary: clean(&self.summary, LONG).unwrap_or_default(),
        }
    }

    /// Whether the model found anything to search for.
    pub fn is_empty(&self) -> bool {
        self.window_title.is_none()
            && self.application.is_none()
            && self.timestamps.is_empty()
            && self.error_texts.is_empty()
            && self.error_codes.is_empty()
            && self.exception_names.is_empty()
            && self.machine_names.is_empty()
            && self.user_names.is_empty()
            && self.server_names.is_empty()
            && self.ids.is_empty()
            && self.versions.is_empty()
    }

    /// The engine's search clues (the engine, not the model, searches; ADR 0025).
    pub fn clues(&self) -> imt_core::related::Clues {
        imt_core::related::Clues {
            window_title: self.window_title.clone(),
            application: self.application.clone(),
            error_texts: self.error_texts.clone(),
            error_codes: self.error_codes.clone(),
            exception_names: self.exception_names.clone(),
            machine_names: self.machine_names.clone(),
            user_names: self.user_names.clone(),
            server_names: self.server_names.clone(),
            ids: self.ids.clone(),
            versions: self.versions.clone(),
        }
        .bounded()
    }

    /// The shown times, for the engine's time resolution.
    pub fn shown_times(&self) -> Vec<imt_core::related::ShownTime> {
        self.timestamps
            .iter()
            .map(|t| imt_core::related::ShownTime {
                text: t.text.clone(),
                has_date: t.has_date,
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn full() -> Value {
        json!({
            "screen_kind": "error_dialog",
            "window_title": "OrderWorker - Error",
            "application": "OrderWorker",
            "timestamps": [{"text": "10:41:22", "has_date": false}, {"text": "10:41:22", "has_date": false}],
            "error_texts": ["Unhandled exception:\n  HttpRequestException\u{7}", "  "],
            "error_codes": ["0x80004005"],
            "exception_names": ["System.Net.Http.HttpRequestException", "system.net.http.httprequestexception"],
            "machine_names": ["WEB01"],
            "user_names": [],
            "server_names": [],
            "ids": ["ORD-1042"],
            "versions": ["2.4.1"],
            "summary": "An error dialog of OrderWorker."
        })
    }

    #[test]
    fn a_valid_answer_is_parsed_cleaned_and_bounded() {
        let f = ScreenFacts::parse(&full().to_string()).unwrap();
        assert_eq!(f.screen_kind, ScreenKind::ErrorDialog);
        assert_eq!(f.window_title.as_deref(), Some("OrderWorker - Error"));
        assert_eq!(f.timestamps.len(), 1, "duplicates are dropped");
        assert_eq!(f.error_texts, ["Unhandled exception: HttpRequestException"]);
        assert_eq!(f.exception_names.len(), 1, "case-insensitive duplicates are dropped");
        assert!(!f.is_empty());
        let fenced = format!("```json\n{}\n```", full());
        assert_eq!(ScreenFacts::parse(&fenced).unwrap(), f);
        let c = f.clues();
        assert_eq!(c.exception_names, f.exception_names);
        assert_eq!(f.shown_times()[0].text, "10:41:22");
    }

    #[test]
    fn invalid_answers_say_what_is_wrong() {
        assert!(ScreenFacts::parse("I see an error dialog.").unwrap_err().contains("JSON"));
        let mut extra = full();
        extra["password"] = json!("x");
        assert!(ScreenFacts::parse(&extra.to_string()).unwrap_err().contains("unknown field"));
        let mut kind = full();
        kind["screen_kind"] = json!("spaceship");
        assert!(ScreenFacts::parse(&kind.to_string()).is_err());
        assert!(ScreenFacts::parse(&"x".repeat(70_000)).unwrap_err().contains("too long"));
        // Missing fields default to empty: an answer with nothing in it is empty.
        let empty = ScreenFacts::parse("{}").unwrap();
        assert!(empty.is_empty() && empty.screen_kind == ScreenKind::Other);
    }

    #[test]
    fn hostile_sizes_are_cut() {
        let mut v = full();
        v["ids"] = json!((0..100).map(|i| format!("id-{i}")).collect::<Vec<_>>());
        v["error_texts"] = json!(["e".repeat(5000)]);
        v["window_title"] = json!("t".repeat(5000));
        v["timestamps"] = json!((0..50)
            .map(|i| json!({"text": format!("10:00:{i:02}"), "has_date": false}))
            .collect::<Vec<_>>());
        let f = ScreenFacts::parse(&v.to_string()).unwrap();
        assert_eq!(f.ids.len(), MAX_IDS);
        assert_eq!(f.error_texts[0].chars().count(), LONG);
        assert_eq!(f.window_title.unwrap().chars().count(), SHORT);
        assert_eq!(f.timestamps.len(), MAX_ITEMS);
    }

    #[test]
    fn the_schema_lists_every_field_strictly() {
        let s = schema();
        assert_eq!(s["additionalProperties"], false);
        let required: Vec<&str> = s["required"].as_array().unwrap().iter().filter_map(Value::as_str).collect();
        // Every field of the type is required by the schema and an answer with all of them parses.
        let all: serde_json::Map<String, Value> = required
            .iter()
            .map(|k| {
                let v = match s["properties"][*k]["type"].clone() {
                    Value::String(t) if t == "array" => json!([]),
                    Value::String(t) if t == "string" => json!(if *k == "screen_kind" { "other" } else { "" }),
                    _ => Value::Null,
                };
                (k.to_string(), v)
            })
            .collect();
        assert_eq!(ScreenFacts::parse(&Value::Object(all).to_string()).unwrap(), ScreenFacts::default());
        let ui = serde_json::to_value(ScreenFacts::default()).unwrap();
        assert_eq!(ui.as_object().unwrap().len(), required.len());
        assert!(ui.get("windowTitle").is_some() && ui.get("window_title").is_none());
    }

    proptest::proptest! {
        #[test]
        fn parsing_never_panics(text in ".{0,600}") {
            let _ = ScreenFacts::parse(&text);
        }
    }
}
