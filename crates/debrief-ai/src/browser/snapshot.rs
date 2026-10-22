//! The page as agent mode sees it: a compact snapshot of the interactive elements (numbered refs, like an
//! accessibility tree), a short excerpt of the visible text, and the signals that hand control back to the
//! user: a CAPTCHA, a login or password field, a payment form. The page script only collects raw facts; the
//! classification and the text the model reads are made here.

use super::protocol::text_field;
use super::url::WebUrl;
use super::{clip, quote_capped};
use serde::Serialize;
use serde_json::{Map, Value};

/// Most elements kept from a snapshot (the page script sends the visible ones first).
pub const MAX_ELEMENTS: usize = 60;
/// Most characters of visible text in a snapshot.
pub const MAX_SNAPSHOT_TEXT: usize = 2_500;
const MAX_NAME_CHARS: usize = 80;
const MAX_FIELDS: usize = 200;
const MAX_FRAMES: usize = 50;
const MAX_MARKS: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum ElementKind {
    Link,
    Button,
    /// A button that submits its form.
    Submit,
    TextBox,
    SearchBox,
    TextArea,
    Checkbox,
    Radio,
    Select,
    /// A password field: the agent never types into it.
    Password,
    Other,
}

impl ElementKind {
    /// How the element is named to the model and in approval cards.
    pub fn describe(self) -> &'static str {
        match self {
            ElementKind::Link => "link",
            ElementKind::Button => "button",
            ElementKind::Submit => "submit button",
            ElementKind::TextBox => "text box",
            ElementKind::SearchBox => "search box",
            ElementKind::TextArea => "text area",
            ElementKind::Checkbox => "checkbox",
            ElementKind::Radio => "radio button",
            ElementKind::Select => "list box",
            ElementKind::Password => "password field",
            ElementKind::Other => "element",
        }
    }

    /// Text can be typed into it (passwords excluded).
    pub fn is_text_input(self) -> bool {
        matches!(self, ElementKind::TextBox | ElementKind::SearchBox | ElementKind::TextArea)
    }
}

/// One interactive element; `r` is the ref the model names it by (the page marks it `data-debrief-ref`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Element {
    pub r: u32,
    pub kind: ElementKind,
    /// The accessible name (label, text, `aria-label`, placeholder), capped.
    pub name: String,
    /// Where a link goes, when it is a web address.
    pub href: Option<String>,
    pub in_form: bool,
    pub in_view: bool,
    /// The tag, as the page reported it (checked again before acting).
    pub tag: String,
}

/// What makes the agent stop and hand control to the user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum HandOff {
    Captcha,
    Login,
    Payment,
}

impl HandOff {
    pub fn describe(self) -> &'static str {
        match self {
            HandOff::Captcha => "a CAPTCHA or human check",
            HandOff::Login => "a login or password field",
            HandOff::Payment => "a payment form",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DomSnapshot {
    pub url: String,
    pub title: String,
    pub elements: Vec<Element>,
    pub text: String,
    /// Set when the page shows a CAPTCHA, a login or a payment form.
    pub hand_off: Option<HandOff>,
}

fn kind_of(tag: &str, ty: &str, role: &str, submit: bool) -> ElementKind {
    match (tag, ty, role) {
        (_, _, "link") | ("a", _, "") => ElementKind::Link,
        (_, _, "checkbox" | "switch") | ("input", "checkbox", _) => ElementKind::Checkbox,
        (_, _, "radio") | ("input", "radio", _) => ElementKind::Radio,
        (_, _, "searchbox") | ("input", "search", _) => ElementKind::SearchBox,
        (_, _, "textbox") => ElementKind::TextBox,
        ("input", "password", _) => ElementKind::Password,
        ("input", "submit" | "image", _) => ElementKind::Submit,
        ("input", "button" | "reset", _) => ElementKind::Button,
        ("input", "hidden" | "file" | "range" | "color", _) => ElementKind::Other,
        ("input", _, _) => ElementKind::TextBox,
        ("textarea", _, _) => ElementKind::TextArea,
        ("select", _, _) | (_, _, "combobox" | "listbox") => ElementKind::Select,
        ("button", _, _) | (_, _, "button" | "menuitem" | "tab") if submit => ElementKind::Submit,
        ("button", _, _) | (_, _, "button" | "menuitem" | "tab") => ElementKind::Button,
        _ => ElementKind::Other,
    }
}

fn lower(m: &Map<String, Value>, key: &str, max: usize) -> String {
    text_field(m, key, max).to_ascii_lowercase()
}

fn element_of(v: &Value) -> Option<Element> {
    let m = v.as_object()?;
    let r = u32::try_from(m.get("r")?.as_u64()?).ok().filter(|r| *r > 0)?;
    let tag = lower(m, "tag", 20);
    let ty = lower(m, "type", 20);
    let role = lower(m, "role", 20);
    let submit = m.get("submit").and_then(Value::as_bool).unwrap_or(false);
    let name = text_field(m, "name", MAX_NAME_CHARS * 2)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let href = m
        .get("href")
        .and_then(Value::as_str)
        .and_then(|h| WebUrl::parse(h).ok())
        .map(|u| u.as_str().to_string());
    Some(Element {
        r,
        kind: kind_of(&tag, &ty, &role, submit),
        name: clip(&name, MAX_NAME_CHARS),
        href,
        in_form: m.get("form").and_then(Value::as_bool).unwrap_or(false),
        in_view: m.get("vis").and_then(Value::as_bool).unwrap_or(false),
        tag,
    })
}

fn strings(m: &Map<String, Value>, key: &str, max: usize) -> Vec<String> {
    m.get(key)
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .take(max)
                .filter_map(Value::as_str)
                .map(|s| s.chars().take(200).collect::<String>().to_ascii_lowercase())
                .collect()
        })
        .unwrap_or_default()
}

const CAPTCHA_HOSTS: &[&str] = &[
    "recaptcha",
    "hcaptcha",
    "challenges.cloudflare.com",
    "captcha",
    "arkoselabs",
    "funcaptcha",
];
const CAPTCHA_MARKS: &[&str] = &[
    "g-recaptcha",
    "h-captcha",
    "cf-turnstile",
    "captcha",
    "cf-challenge",
    "challenge-form",
];
pub(crate) const CAPTCHA_PHRASES: &[&str] = &[
    "verify you are human",
    "verify that you are human",
    "i'm not a robot",
    "i am not a robot",
    "are you a robot",
    "complete the security check",
    "unusual traffic from your computer",
    "checking your browser",
    // Stack Overflow's (and other Stack Exchange sites') check.
    "human verification",
];
const LOGIN_AUTOCOMPLETE: &[&str] = &["current-password", "new-password", "one-time-code"];
const LOGIN_NAMES: &[&str] = &["password", "passwd", "passcode", "otp", "totp", "2fa", "mfa"];
const PAYMENT_AUTOCOMPLETE: &[&str] = &["cc-number", "cc-csc", "cc-exp", "cc-exp-month", "cc-exp-year", "cc-name"];
const PAYMENT_NAMES: &[&str] = &[
    "cardnumber",
    "card-number",
    "card_number",
    "ccnum",
    "cvv",
    "cvc",
    "csc",
    "securitycode",
    "expiry",
];

/// Classifies the raw signals a snapshot reply carries (inputs, frames, class and id tokens, visible text).
fn hand_off_of(m: &Map<String, Value>, text: &str) -> Option<HandOff> {
    let frames = strings(m, "frames", MAX_FRAMES);
    let marks = strings(m, "marks", MAX_MARKS);
    let lower_text = text.to_lowercase();
    if frames.iter().any(|f| CAPTCHA_HOSTS.iter().any(|h| f.contains(h)))
        || marks.iter().any(|k| CAPTCHA_MARKS.iter().any(|c| k.contains(c)))
        || CAPTCHA_PHRASES.iter().any(|p| lower_text.contains(p))
    {
        return Some(HandOff::Captcha);
    }
    let fields: Vec<&Map<String, Value>> = m
        .get("fields")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(MAX_FIELDS).filter_map(Value::as_object).collect())
        .unwrap_or_default();
    let visible = |f: &Map<String, Value>| f.get("vis").and_then(Value::as_bool).unwrap_or(true);
    let field_matches = |f: &Map<String, Value>, autocomplete: &[&str], names: &[&str]| {
        let ac = lower(f, "ac", 60);
        let nm: String = lower(f, "nm", 120).chars().filter(|c| !c.is_whitespace()).collect();
        ac.split_whitespace().any(|t| autocomplete.contains(&t)) || names.iter().any(|n| nm.contains(n))
    };
    if fields
        .iter()
        .filter(|f| visible(f))
        .any(|f| field_matches(f, PAYMENT_AUTOCOMPLETE, PAYMENT_NAMES))
    {
        return Some(HandOff::Payment);
    }
    if fields
        .iter()
        .filter(|f| visible(f))
        .any(|f| lower(f, "type", 20) == "password" || field_matches(f, LOGIN_AUTOCOMPLETE, LOGIN_NAMES))
    {
        return Some(HandOff::Login);
    }
    None
}

/// Builds the snapshot from a checked page reply (see `protocol::ReplyGate`).
pub fn snapshot_from(reply: &Map<String, Value>) -> DomSnapshot {
    let mut elements: Vec<Element> = reply
        .get("elements")
        .and_then(Value::as_array)
        .map(|a| a.iter().take(MAX_ELEMENTS * 4).filter_map(element_of).collect())
        .unwrap_or_default();
    elements.sort_by_key(|e| (!e.in_view, e.r));
    let mut seen = std::collections::HashSet::new();
    elements.retain(|e| seen.insert(e.r));
    elements.truncate(MAX_ELEMENTS);
    elements.sort_by_key(|e| e.r);
    let text = text_field(reply, "text", MAX_SNAPSHOT_TEXT * 2);
    let text = clip(&text.split_whitespace().collect::<Vec<_>>().join(" "), MAX_SNAPSHOT_TEXT);
    DomSnapshot {
        url: text_field(reply, "url", super::url::MAX_URL_LEN),
        title: clip(&text_field(reply, "title", 300), 200),
        hand_off: hand_off_of(reply, &text),
        elements,
        text,
    }
}

impl DomSnapshot {
    /// A page that could not be read (nothing to click, no hand-off signals).
    pub fn blank() -> DomSnapshot {
        DomSnapshot {
            url: String::new(),
            title: String::new(),
            elements: vec![],
            text: String::new(),
            hand_off: None,
        }
    }

    pub fn element(&self, r: u32) -> Option<&Element> {
        self.elements.iter().find(|e| e.r == r)
    }

    /// The snapshot as the model reads it, quoted as untrusted web content and capped.
    pub fn render(&self, max_chars: usize) -> String {
        let mut body = format!("Page: {} ({})\n", self.title, self.url);
        if let Some(h) = self.hand_off {
            body.push_str(&format!("The page shows {}: stop and let the user take over.\n", h.describe()));
        }
        body.push_str("Interactive elements (use the number as ref):\n");
        for e in &self.elements {
            let mut line = format!("[{}] {} \"{}\"", e.r, e.kind.describe(), e.name);
            if let Some(h) = &e.href {
                line.push_str(&format!(" -> {h}"));
            }
            if e.in_form && e.kind != ElementKind::Link {
                line.push_str(" (in a form)");
            }
            if !e.in_view {
                line.push_str(" (scroll to see)");
            }
            body.push_str(&line);
            body.push('\n');
        }
        if self.elements.is_empty() {
            body.push_str("(none)\n");
        }
        body.push_str("Visible text:\n");
        body.push_str(&self.text);
        quote_capped(&self.url, &body, max_chars)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::web::UNTRUSTED_REMINDER;
    use serde_json::json;

    fn snap(v: Value) -> DomSnapshot {
        snapshot_from(v.as_object().unwrap())
    }

    #[test]
    fn elements_are_typed_numbered_and_capped() {
        let s = snap(json!({
            "url": "https://e.org/", "title": "Docs",
            "elements": [
                {"r": 2, "tag": "A", "name": "  Next   page ", "href": "https://e.org/2", "vis": true},
                {"r": 1, "tag": "input", "type": "search", "name": "Search", "form": true, "vis": true},
                {"r": 3, "tag": "button", "submit": true, "name": "Go", "form": true, "vis": true},
                {"r": 4, "tag": "input", "type": "password", "name": "Password", "vis": true},
                {"r": 5, "tag": "a", "name": "js link", "href": "javascript:void(0)", "vis": false},
                {"r": 6, "tag": "div", "role": "button", "name": "Menu", "vis": true},
                {"r": 7, "tag": "textarea", "name": "x".repeat(300), "vis": true},
                {"r": 8, "tag": "select", "vis": true},
                {"r": 9, "tag": "input", "type": "checkbox", "vis": true},
                {"r": 10, "tag": "input", "type": "radio", "vis": true},
                {"r": 11, "tag": "input", "vis": true},
                {"r": 12, "tag": "input", "type": "hidden", "vis": true},
                {"r": 13, "tag": "span", "role": "textbox", "vis": true},
                {"r": 14, "tag": "input", "type": "submit", "vis": true},
                {"r": 15, "tag": "input", "type": "button", "vis": true},
                {"r": 16, "tag": "summary", "vis": true},
                {"r": 0, "tag": "a"},
                {"tag": "a"},
                "junk",
                {"r": 2, "tag": "a", "name": "duplicate"}
            ]
        }));
        let kinds: Vec<(u32, ElementKind)> = s.elements.iter().map(|e| (e.r, e.kind)).collect();
        use ElementKind::*;
        assert_eq!(
            kinds,
            vec![
                (1, SearchBox),
                (2, Link),
                (3, Submit),
                (4, Password),
                (5, Link),
                (6, Button),
                (7, TextArea),
                (8, Select),
                (9, Checkbox),
                (10, Radio),
                (11, TextBox),
                (12, Other),
                (13, TextBox),
                (14, Submit),
                (15, Button),
                (16, Other)
            ]
        );
        assert_eq!(s.element(2).unwrap().name, "Next page");
        assert_eq!(s.element(5).unwrap().href, None, "javascript: is not a web address");
        assert_eq!(s.element(7).unwrap().name.chars().count(), MAX_NAME_CHARS);
        assert!(s.element(99).is_none());
        assert!(TextArea.is_text_input() && !Password.is_text_input());
        let many: Vec<Value> = (1..=200).map(|r| json!({"r": r, "tag": "a", "vis": r > 150})).collect();
        let s = snap(json!({"elements": many}));
        assert_eq!(s.elements.len(), MAX_ELEMENTS);
        assert!(
            s.elements.iter().filter(|e| e.in_view).count() == 50,
            "visible elements are kept first"
        );
    }

    #[test]
    fn captchas_logins_and_payment_forms_hand_control_back() {
        let h = |v: Value| snap(v).hand_off;
        assert_eq!(
            h(json!({"frames": ["www.google.com/recaptcha/api2/anchor"]})),
            Some(HandOff::Captcha)
        );
        assert_eq!(h(json!({"marks": ["cf-turnstile"]})), Some(HandOff::Captcha));
        assert_eq!(
            h(json!({"text": "Please verify you are human to continue"})),
            Some(HandOff::Captcha)
        );
        assert_eq!(h(json!({"fields": [{"type": "password", "vis": true}]})), Some(HandOff::Login));
        assert_eq!(
            h(json!({"fields": [{"type": "text", "ac": "one-time-code"}]})),
            Some(HandOff::Login)
        );
        assert_eq!(h(json!({"fields": [{"type": "text", "nm": "OTP code"}]})), Some(HandOff::Login));
        assert_eq!(
            h(json!({"fields": [{"type": "password", "vis": false}]})),
            None,
            "a hidden field is not a login"
        );
        assert_eq!(h(json!({"fields": [{"type": "text", "ac": "cc-number"}]})), Some(HandOff::Payment));
        assert_eq!(
            h(json!({"fields": [{"type": "tel", "nm": "card number"}, {"type": "password"}]})),
            Some(HandOff::Payment)
        );
        assert_eq!(
            h(json!({"fields": [{"type": "search", "nm": "q"}], "text": "Robots are fun"})),
            None
        );
        assert_eq!(h(json!({})), None);
    }

    #[test]
    fn the_model_reads_a_compact_quoted_snapshot() {
        let s = snap(json!({
            "url": "https://e.org/", "title": "Docs", "text": "Hello   world",
            "elements": [{"r": 1, "tag": "a", "name": "Next", "href": "https://e.org/2", "vis": true},
                          {"r": 2, "tag": "input", "type": "text", "name": "Query", "form": true, "vis": false}],
            "fields": [{"type": "password"}]
        }));
        let r = s.render(10_000);
        assert!(r.starts_with(UNTRUSTED_REMINDER), "{r}");
        assert!(r.contains("[1] link \"Next\" -> https://e.org/2\n"), "{r}");
        assert!(r.contains("[2] text box \"Query\" (in a form) (scroll to see)\n"), "{r}");
        assert!(r.contains("a login or password field: stop"), "{r}");
        assert!(r.contains("Visible text:\nHello world"), "{r}");
        assert!(snap(json!({})).render(10_000).contains("(none)"));
        assert!(s.render(400).chars().count() <= 400 && s.render(400).ends_with(crate::web::CLOSE_TAG));
    }
}
