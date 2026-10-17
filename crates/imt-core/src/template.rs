//! Serilog message templates and output templates.

use crate::model::{value_text, Fields};

/// Renders a message template ("Order {OrderId} took {Elapsed:0.0} ms") with
/// the event's properties, the way Serilog's `{Message:lj}` does: strings are
/// not quoted, formats and alignment are ignored. Unknown holes stay as-is.
pub fn render(template: &str, fields: &Fields) -> String {
    let mut out = String::with_capacity(template.len() + 32);
    let mut rest = template;
    while let Some(i) = rest.find(['{', '}']) {
        out.push_str(&rest[..i]);
        let tail = &rest[i..];
        if tail.starts_with("{{") || tail.starts_with("}}") {
            out.push(tail.as_bytes()[0] as char);
            rest = &tail[2..];
            continue;
        }
        if let Some(after) = tail.strip_prefix('}') {
            out.push('}');
            rest = after;
            continue;
        }
        match tail.find('}') {
            Some(end) => {
                let hole = &tail[1..end];
                let name = hole_name(hole);
                match fields.get(name) {
                    Some(v) => out.push_str(&value_text(v)),
                    None => out.push_str(&tail[..=end]),
                }
                rest = &tail[end + 1..];
            }
            None => {
                out.push_str(tail);
                rest = "";
            }
        }
    }
    out.push_str(rest);
    out
}

fn hole_name(hole: &str) -> &str {
    let hole = hole.trim_start_matches(['@', '$']);
    let end = hole.find([':', ',']).unwrap_or(hole.len());
    hole[..end].trim()
}

/// Group name used for a `{Properties}` hole; its JSON is lifted into fields.
pub const PROPERTIES_GROUP: &str = "__props";

/// Converts a Serilog output template into a regex matching the first line of
/// an event. Everything from `{NewLine}` on belongs to continuation lines.
pub fn output_template_to_regex(template: &str) -> String {
    #[derive(Debug)]
    enum Tok {
        Lit(String),
        Hole(String),
    }
    let mut toks = Vec::new();
    let mut lit = String::new();
    let mut rest = template;
    while let Some(i) = rest.find('{') {
        lit.push_str(&rest[..i]);
        let tail = &rest[i..];
        if let Some(after) = tail.strip_prefix("{{") {
            lit.push('{');
            rest = after;
            continue;
        }
        let Some(end) = tail.find('}') else {
            lit.push_str(tail);
            rest = "";
            break;
        };
        if !lit.is_empty() {
            toks.push(Tok::Lit(std::mem::take(&mut lit)));
        }
        toks.push(Tok::Hole(hole_name(&tail[1..end]).to_string()));
        rest = &tail[end + 1..];
    }
    lit.push_str(rest);
    if !lit.is_empty() {
        toks.push(Tok::Lit(lit));
    }
    if let Some(nl) = toks
        .iter()
        .position(|t| matches!(t, Tok::Hole(h) if h == "NewLine" || h == "Exception"))
    {
        toks.truncate(nl);
    }
    // Trailing whitespace in the template is not significant on the line.
    if let Some(Tok::Lit(l)) = toks.last_mut() {
        let trimmed = l.trim_end().to_string();
        *l = trimmed;
    }

    let mut re = String::from("^");
    let mut used = std::collections::HashSet::new();
    let n = toks.len();
    for (i, tok) in toks.iter().enumerate() {
        let last = i == n - 1;
        match tok {
            Tok::Lit(l) => {
                // Let runs of spaces match flexible whitespace.
                let mut first = true;
                for part in l.split(' ') {
                    if !first {
                        re.push_str(r"\s+");
                    }
                    re.push_str(&regex::escape(part));
                    first = false;
                }
            }
            Tok::Hole(h) => {
                let (group, pattern) = match h.as_str() {
                    "Timestamp" => ("timestamp", TIMESTAMP_PATTERN),
                    "Level" => ("level", r"[A-Za-z]{3,11}"),
                    "Message" => ("message", if last { ".*" } else { ".*?" }),
                    "Properties" => (PROPERTIES_GROUP, r"\{.*\}"),
                    other => (other, if last { ".*" } else { ".*?" }),
                };
                let valid = group.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                    && group.chars().next().is_some_and(|c| c.is_ascii_alphabetic() || c == '_');
                if valid && used.insert(group.to_string()) {
                    re.push_str(&format!("(?P<{group}>{pattern})"));
                } else {
                    re.push_str(&format!("(?:{pattern})"));
                }
            }
        }
    }
    if toks.is_empty() {
        re.push_str("(?P<message>.*)");
    }
    re.push('$');
    re
}

/// Anything that looks like a date and/or time, up to an optional offset.
pub const TIMESTAMP_PATTERN: &str =
    r"(?:\d{1,4}[-/.]\d{1,2}[-/.]\d{1,4}[ T])?\d{1,2}:\d{2}:\d{2}(?:[.,]\d{1,9})?(?:\s?[AaPp][Mm])?(?:\s?(?:Z|[+-]\d{2}:?\d{2}))?";

#[cfg(test)]
mod tests {
    use super::*;
    use regex::Regex;
    use serde_json::json;

    #[test]
    fn renders_templates() {
        let f = json!({"OrderId": 42, "User": "ann", "Elapsed": 12.5}).as_object().unwrap().clone();
        assert_eq!(
            render("Order {OrderId} by {@User} took {Elapsed:0.0} ms {{x}} {Missing}", &f),
            "Order 42 by ann took 12.5 ms {x} {Missing}"
        );
    }

    #[test]
    fn serilog_default_file_template() {
        let re = Regex::new(&output_template_to_regex(
            "{Timestamp:yyyy-MM-dd HH:mm:ss.fff zzz} [{Level:u3}] {Message:lj}{NewLine}{Exception}",
        ))
        .unwrap();
        let c = re.captures("2026-10-14 10:41:22.318 +08:00 [INF] Application started").unwrap();
        assert_eq!(&c["timestamp"], "2026-10-14 10:41:22.318 +08:00");
        assert_eq!(&c["level"], "INF");
        assert_eq!(&c["message"], "Application started");
    }

    #[test]
    fn custom_holes_become_groups() {
        let re = Regex::new(&output_template_to_regex(
            "{Timestamp:HH:mm:ss} [{Level:u3}] [{MachineName}/{ProcessId}] {Message:lj} {Properties:j}{NewLine}{Exception}",
        ))
        .unwrap();
        let c = re
            .captures("10:41:22 [WRN] [WEB02/4120] Slow call {\"SourceContext\": \"X\"}")
            .unwrap();
        assert_eq!(&c["MachineName"], "WEB02");
        assert_eq!(&c["ProcessId"], "4120");
        assert_eq!(&c["message"], "Slow call");
        assert_eq!(&c[PROPERTIES_GROUP], "{\"SourceContext\": \"X\"}");
    }

    #[test]
    fn render_edge_cases() {
        let f = json!({"A": "x"}).as_object().unwrap().clone();
        assert_eq!(render("a }} b } c {A,-10} {unclosed", &f), "a } b } c x {unclosed");
        assert_eq!(render("{$A:l}", &f), "x");
    }

    #[test]
    fn output_template_edge_cases() {
        // Empty template still yields a message group.
        assert_eq!(output_template_to_regex(""), "^(?P<message>.*)$");
        // Escaped braces, trailing literal, duplicate and invalid hole names, unclosed hole.
        let re = Regex::new(&output_template_to_regex("{{x}} {Level} {Level} {My-Name} {Message} end  ")).unwrap();
        let c = re.captures("{x}} INF INF n msg end").unwrap();
        assert_eq!(&c["level"], "INF");
        assert_eq!(&c["message"], "msg");
        let re = Regex::new(&output_template_to_regex("{Message} {broken")).unwrap();
        assert!(re.is_match("hello {broken"));
    }
}
