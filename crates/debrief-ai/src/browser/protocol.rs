//! How results come back from a page without giving the page any IPC. Rust evaluates a script in the page
//! (`eval_with_callback`); the script's return value reaches Rust in the callback of exactly that call. The
//! page itself has no way to call Debrief: the browser window has no capability and the shell refuses every
//! command from it.
//!
//! Each evaluation still carries a one-shot nonce, issued by Rust when the user clicks or the agent acts.
//! A [`ReplyGate`] accepts one reply only, only with that nonce, only from the origin Rust expects (the page
//! may have navigated meanwhile) and only up to a size cap. Everything in the reply is untrusted data.

use serde_json::Value;

/// Largest raw reply accepted from a page (the JSON text as the webview hands it over).
pub const MAX_REPLY_BYTES: usize = 2 * 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ReplyError {
    #[error("the page's answer is larger than {MAX_REPLY_BYTES} bytes")]
    TooLarge,
    #[error("the page's answer is not valid")]
    Malformed,
    #[error("the answer does not belong to this request")]
    WrongNonce,
    #[error("this request was already answered")]
    Used,
    #[error("the page changed while it was read (now {0}); try again")]
    OriginChanged(String),
    #[error("the page could not be read: {0}")]
    Script(String),
}

/// A random one-shot token: 32 hex characters from the OS random source.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nonce(String);

impl Nonce {
    pub fn new() -> Nonce {
        let mut b = [0u8; 16];
        if getrandom::fill(&mut b).is_err() {
            // No OS randomness (never seen in practice): the clock and a counter still make it one-shot.
            static COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
            let t = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0);
            let n = u128::from(COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed));
            b = (t ^ (n << 96)).to_le_bytes();
        }
        Nonce(b.iter().map(|x| format!("{x:02x}")).collect())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Default for Nonce {
    fn default() -> Self {
        Nonce::new()
    }
}

/// Waits for exactly one reply to one evaluation.
#[derive(Debug)]
pub struct ReplyGate {
    nonce: Nonce,
    /// `scheme://host[:port]` of the page the script was sent to; `None` accepts any web origin.
    origin: Option<String>,
    used: bool,
}

impl ReplyGate {
    pub fn new(nonce: Nonce, origin: Option<String>) -> ReplyGate {
        ReplyGate {
            nonce,
            origin,
            used: false,
        }
    }

    pub fn nonce(&self) -> &Nonce {
        &self.nonce
    }

    /// Checks one raw reply and returns its object. A reply is the script's `JSON.stringify(...)` result as
    /// the webview serialises it: a JSON string holding the object (an object directly is accepted too).
    pub fn accept(&mut self, raw: &str) -> Result<serde_json::Map<String, Value>, ReplyError> {
        if self.used {
            return Err(ReplyError::Used);
        }
        self.used = true;
        if raw.len() > MAX_REPLY_BYTES {
            return Err(ReplyError::TooLarge);
        }
        let outer: Value = serde_json::from_str(raw).map_err(|_| ReplyError::Malformed)?;
        let inner = match outer {
            Value::String(s) => serde_json::from_str::<Value>(&s).map_err(|_| ReplyError::Malformed)?,
            v => v,
        };
        let Value::Object(map) = inner else {
            return Err(ReplyError::Malformed);
        };
        if map.get("nonce").and_then(Value::as_str) != Some(self.nonce.as_str()) {
            return Err(ReplyError::WrongNonce);
        }
        if let Some(err) = map.get("error").and_then(Value::as_str) {
            return Err(ReplyError::Script(err.chars().take(200).collect()));
        }
        let origin = map.get("origin").and_then(Value::as_str).unwrap_or("");
        if let Some(expected) = &self.origin {
            if origin != expected {
                return Err(ReplyError::OriginChanged(origin.chars().take(100).collect()));
            }
        }
        Ok(map)
    }
}

/// A string field of a reply, capped to `max` characters (missing or not a string: empty).
pub fn text_field(map: &serde_json::Map<String, Value>, key: &str, max: usize) -> String {
    map.get(key)
        .and_then(Value::as_str)
        .map(|s| {
            s.chars()
                .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
                .take(max)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn reply(v: Value) -> String {
        serde_json::to_string(&Value::String(v.to_string())).unwrap()
    }

    #[test]
    fn nonces_are_random_hex() {
        let (a, b) = (Nonce::new(), Nonce::default());
        assert_ne!(a, b);
        assert_eq!(a.as_str().len(), 32);
        assert!(a.as_str().chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn a_gate_accepts_one_matching_reply_from_the_expected_origin() {
        let n = Nonce::new();
        let ok = json!({"nonce": n.as_str(), "origin": "https://e.org", "title": "T"});
        let mut g = ReplyGate::new(n.clone(), Some("https://e.org".into()));
        assert_eq!(g.nonce(), &n);
        assert_eq!(g.accept(&reply(ok.clone())).unwrap()["title"], "T");
        assert_eq!(g.accept(&reply(ok.clone())), Err(ReplyError::Used), "one-shot");
        // An object directly (not double-encoded) is fine too.
        assert!(ReplyGate::new(n.clone(), None).accept(&ok.to_string()).is_ok());
    }

    #[test]
    fn foreign_stale_large_and_broken_replies_are_refused() {
        let n = Nonce::new();
        let gate = || ReplyGate::new(n.clone(), Some("https://e.org".into()));
        let other = json!({"nonce": Nonce::new().as_str(), "origin": "https://e.org"});
        assert_eq!(gate().accept(&reply(other)), Err(ReplyError::WrongNonce));
        assert_eq!(
            gate().accept(&reply(json!({"origin": "https://e.org"}))),
            Err(ReplyError::WrongNonce)
        );
        let moved = json!({"nonce": n.as_str(), "origin": "https://evil.example"});
        assert_eq!(
            gate().accept(&reply(moved)),
            Err(ReplyError::OriginChanged("https://evil.example".into()))
        );
        let failed = json!({"nonce": n.as_str(), "origin": "https://e.org", "error": "TypeError: x"});
        assert_eq!(gate().accept(&reply(failed)), Err(ReplyError::Script("TypeError: x".into())));
        assert_eq!(gate().accept("null"), Err(ReplyError::Malformed));
        assert_eq!(gate().accept(""), Err(ReplyError::Malformed));
        assert_eq!(gate().accept("\"not json\""), Err(ReplyError::Malformed));
        assert_eq!(gate().accept("[1]"), Err(ReplyError::Malformed));
        let big = format!("\"{}\"", "a".repeat(MAX_REPLY_BYTES));
        assert_eq!(gate().accept(&big), Err(ReplyError::TooLarge));
    }

    #[test]
    fn text_fields_are_capped_and_stripped_of_control_characters() {
        let m = json!({"t": "a\u{0}b\nc\u{7}d", "n": 5}).as_object().unwrap().clone();
        assert_eq!(text_field(&m, "t", 100), "ab\ncd");
        assert_eq!(text_field(&m, "t", 2), "ab");
        assert_eq!(text_field(&m, "n", 10), "");
        assert_eq!(text_field(&m, "missing", 10), "");
    }
}
