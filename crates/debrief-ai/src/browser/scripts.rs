//! The scripts Debrief evaluates in the page (`js/*.js`), with their arguments. Arguments are JSON, so nothing
//! from the model or a page can break out of the argument object; every script returns
//! `JSON.stringify({nonce, origin, url, ...})` for a `protocol::ReplyGate` to check.

use super::capture::{MAX_BLOCKS, MAX_CAPTURE_CHARS};
use super::policy::ScrollDirection;
use super::protocol::Nonce;
use super::results::{MAX_LINK_TEXT, MAX_RESULT_LINKS};
use super::snapshot::{Element, MAX_ELEMENTS};
use serde_json::{json, Value};
use std::sync::OnceLock;

const CAPTURE: &str = include_str!("js/capture.js");
const SNAPSHOT: &str = include_str!("js/snapshot.js");
const ACT: &str = include_str!("js/act.js");
const RESULT_LINKS: &str = include_str!("js/result_links.js");

/// `(function (A) {...})({...})`, with the line endings of the checkout normalised.
fn call(source: &str, args: Value) -> String {
    let args = serde_json::to_string(&args).unwrap_or_else(|_| "{}".into());
    // U+2028/2029 are valid in JSON but were line terminators in old JS engines: escape them anyway.
    let args = args.replace('\u{2028}', "\\u2028").replace('\u{2029}', "\\u2029");
    format!("({})({args})", source.replace("\r\n", "\n").trim())
}

/// The page's readable text (see `capture::capture_from`).
pub fn capture(nonce: &Nonce) -> String {
    // Room for the cap after rendering (headings and fences add a little).
    call(
        CAPTURE,
        json!({"nonce": nonce.as_str(), "maxBlocks": MAX_BLOCKS, "maxChars": MAX_CAPTURE_CHARS * 2}),
    )
}

/// The interactive elements and hand-off signals (see `snapshot::snapshot_from`).
pub fn snapshot(nonce: &Nonce) -> String {
    call(SNAPSHOT, json!({"nonce": nonce.as_str(), "maxElements": MAX_ELEMENTS * 2}))
}

/// The name of the property the results-page collector keeps its DOM observer in: random per process (a letter first,
/// so it is an ordinary property name), so a page cannot look for it in advance.
fn observer_slot() -> &'static str {
    static SLOT: OnceLock<String> = OnceLock::new();
    SLOT.get_or_init(|| format!("i{}", Nonce::new().as_str()))
}

/// Every link of a search results page, in document order, marked when inside `region`, with the page's readiness
/// (see `results.rs`, `readiness.rs`).
pub fn result_links(nonce: &Nonce, region: &[&str]) -> String {
    call(
        RESULT_LINKS,
        json!({"nonce": nonce.as_str(), "max": MAX_RESULT_LINKS, "maxText": MAX_LINK_TEXT, "region": region, "slot": observer_slot()}),
    )
}

pub fn click(nonce: &Nonce, element: &Element) -> String {
    call(
        ACT,
        json!({"nonce": nonce.as_str(), "op": "click", "ref": element.r, "tag": element.tag}),
    )
}

pub fn type_text(nonce: &Nonce, element: &Element, text: &str, submit: bool) -> String {
    call(
        ACT,
        json!({"nonce": nonce.as_str(), "op": "type", "ref": element.r, "tag": element.tag, "text": text, "submit": submit}),
    )
}

pub fn scroll(nonce: &Nonce, direction: ScrollDirection) -> String {
    let dir = match direction {
        ScrollDirection::Up => "up",
        ScrollDirection::Down => "down",
    };
    call(ACT, json!({"nonce": nonce.as_str(), "op": "scroll", "dir": dir}))
}

/// Back or forward in the page's history.
pub fn history(nonce: &Nonce, back: bool) -> String {
    call(ACT, json!({"nonce": nonce.as_str(), "op": if back { "back" } else { "forward" }}))
}

/// Fire-and-forget page commands (no reply needed).
pub const STOP_LOADING: &str = "window.stop()";
pub const RELOAD: &str = "location.reload()";

#[cfg(test)]
mod tests {
    use super::super::snapshot::ElementKind;
    use super::*;

    fn el() -> Element {
        Element {
            r: 7,
            kind: ElementKind::TextBox,
            name: "q".into(),
            href: None,
            in_form: true,
            in_view: true,
            tag: "input".into(),
        }
    }

    /// The JSON argument object at the end of a script.
    fn args(script: &str) -> Value {
        let start = script.rfind(")({").unwrap() + 2;
        serde_json::from_str(&script[start..script.len() - 1]).unwrap()
    }

    #[test]
    fn scripts_are_function_calls_with_json_arguments() {
        let n = Nonce::new();
        for s in [
            capture(&n),
            snapshot(&n),
            result_links(&n, &["#rso"]),
            click(&n, &el()),
            scroll(&n, ScrollDirection::Up),
            history(&n, true),
        ] {
            assert!(
                s.starts_with("(// Evaluated in the page") && s.contains("function (A)"),
                "{}",
                &s[..60]
            );
            assert!(!s.contains('\r'));
            assert_eq!(args(&s)["nonce"], n.as_str());
        }
        assert_eq!(args(&capture(&n))["maxChars"], MAX_CAPTURE_CHARS * 2);
        assert_eq!(args(&click(&n, &el()))["tag"], "input");
        assert_eq!(args(&result_links(&n, &["#rso", "#search"]))["region"], json!(["#rso", "#search"]));
        assert_eq!(args(&result_links(&n, &[]))["max"], MAX_RESULT_LINKS);
        let slot = args(&result_links(&n, &[]))["slot"].as_str().unwrap().to_string();
        assert!(slot.len() > 16 && slot.starts_with('i') && slot.chars().all(|c| c.is_ascii_alphanumeric()));
        assert_eq!(
            args(&result_links(&Nonce::new(), &["#rso"]))["slot"],
            slot.as_str(),
            "one slot per process: the observer a first look installed is found again"
        );
        assert_eq!(args(&scroll(&n, ScrollDirection::Down))["dir"], "down");
        assert_eq!(args(&history(&n, false))["op"], "forward");
    }

    #[test]
    fn typed_text_cannot_escape_the_arguments() {
        let n = Nonce::new();
        let evil = "\"}); alert(1); ({\"x\": \"</script>\u{2028}";
        let s = type_text(&n, &el(), evil, true);
        let a = args(&s);
        assert_eq!(a["text"], evil);
        assert_eq!((a["ref"].as_u64(), a["submit"].as_bool()), (Some(7), Some(true)));
        assert!(!s.contains('\u{2028}'));
        assert_eq!(s.matches("alert(1)").count(), 1, "only inside the JSON string");
    }
}
