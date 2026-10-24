//! One page (tab) of a running browser.

use crate::cdp::Connection;
use crate::{Error, Result};
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;

const CALL: Duration = Duration::from_secs(10);
/// URL patterns blocked by [`Page::block_heavy`].
const HEAVY: &[&str] = &[
    "*.png", "*.jpg", "*.jpeg", "*.gif", "*.webp", "*.avif", "*.svg", "*.ico", "*.woff", "*.woff2", "*.ttf", "*.otf", "*.mp4", "*.webm",
    "*.mp3", "*.ogg",
];

pub struct Page {
    pub(crate) conn: Arc<Connection>,
    pub(crate) session: String,
    pub(crate) target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub url: String,
    pub title: String,
}

impl Page {
    pub(crate) fn attach(conn: Arc<Connection>) -> Result<Page> {
        let t = conn.call(None, "Target.createTarget", json!({"url": "about:blank"}), CALL)?;
        let target = str_of(&t, "targetId")?;
        let s = conn.call(None, "Target.attachToTarget", json!({"targetId": target, "flatten": true}), CALL)?;
        let session = str_of(&s, "sessionId")?;
        conn.watch(&session);
        conn.watch(&target);
        let p = Page { conn, session, target };
        p.call("Page.enable", json!({}))?;
        p.call("Network.enable", json!({}))?;
        // Downloads are never wanted from a page that is only read.
        let _ = p.conn.call(None, "Browser.setDownloadBehavior", json!({"behavior": "deny"}), CALL);
        Ok(p)
    }

    /// The page is gone: its window or tab was closed (by the user), it crashed, or the browser connection ended.
    pub fn is_closed(&self) -> bool {
        self.conn.is_closed() || self.conn.is_gone(&self.session) || self.conn.is_gone(&self.target)
    }

    /// Asks the browser whether this page still exists (for a close the browser did not report as an event).
    /// `Ok(false)`: it is gone; an error says nothing either way.
    pub fn exists(&self) -> Result<bool> {
        if self.is_closed() {
            return Ok(false);
        }
        let r = self.conn.call(None, "Target.getTargets", json!({}), CALL)?;
        Ok(r.get("targetInfos")
            .and_then(Value::as_array)
            .is_some_and(|t| t.iter().any(|i| i.get("targetId").and_then(Value::as_str) == Some(&self.target))))
    }

    /// A call on the page: [`Error::Closed`] once the page is gone, whatever the browser answers.
    fn call(&self, method: &str, params: Value) -> Result<Value> {
        if self.is_closed() {
            return Err(Error::Closed);
        }
        self.closed_wins(self.conn.call(Some(&self.session), method, params, CALL))
    }

    fn closed_wins<T>(&self, r: Result<T>) -> Result<T> {
        if self.is_closed() {
            return Err(Error::Closed);
        }
        r
    }

    /// Waits for the page's load event; a page closed meanwhile ends the wait at once.
    fn wait_load(&self, since: u64, timeout: Duration) -> Result<()> {
        let (s, t) = (self.session.clone(), self.target.clone());
        let ev = self.conn.wait_event(since, timeout, |e| {
            (e.method == "Page.loadEventFired" && e.session.as_deref() == Some(&s)) || ends(e, &s, &t)
        });
        if self.is_closed() {
            return Err(Error::Closed);
        }
        ev.map(|_| ()).ok_or(Error::Timeout("the page load"))
    }

    /// Loads `url` and waits for the load event (or `timeout`).
    pub fn navigate(&self, url: &str, timeout: Duration) -> Result<()> {
        let since = self.conn.mark();
        let r = self.call("Page.navigate", json!({"url": url}))?;
        if let Some(e) = r.get("errorText").and_then(Value::as_str) {
            return Err(Error::Protocol(format!("the page did not load: {e}")));
        }
        self.wait_load(since, timeout)
    }

    pub fn back(&self, timeout: Duration) -> Result<()> {
        let h = self.call("Page.getNavigationHistory", json!({}))?;
        let cur = h.get("currentIndex").and_then(Value::as_i64).unwrap_or(0);
        let id = h
            .get("entries")
            .and_then(|e| e.get((cur - 1).max(0) as usize))
            .and_then(|e| e.get("id"))
            .and_then(Value::as_i64)
            .ok_or_else(|| Error::Protocol("there is no earlier page".into()))?;
        let since = self.conn.mark();
        self.call("Page.navigateToHistoryEntry", json!({"entryId": id}))?;
        match self.wait_load(since, timeout) {
            Err(Error::Closed) => Err(Error::Closed),
            _ => Ok(()),
        }
    }

    /// Evaluates a JavaScript expression in the page and returns its value.
    pub fn evaluate(&self, expression: &str) -> Result<Value> {
        let r = self.call(
            "Runtime.evaluate",
            json!({"expression": expression, "returnByValue": true, "awaitPromise": true, "timeout": 8000}),
        )?;
        if let Some(d) = r.get("exceptionDetails") {
            let m = d
                .pointer("/exception/description")
                .or_else(|| d.get("text"))
                .and_then(Value::as_str)
                .unwrap_or("script error");
            return Err(Error::Protocol(m.to_string()));
        }
        Ok(r.pointer("/result/value").cloned().unwrap_or(Value::Null))
    }

    pub fn info(&self) -> Result<Info> {
        let r = self.closed_wins(self.conn.call(None, "Target.getTargetInfo", json!({"targetId": self.target}), CALL))?;
        let g = |k: &str| {
            r.pointer(&format!("/targetInfo/{k}"))
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string()
        };
        Ok(Info {
            url: g("url"),
            title: g("title"),
        })
    }

    /// A bounded excerpt of the visible text, for challenge detection.
    pub fn text_excerpt(&self) -> Result<String> {
        let v = self.evaluate("document.body ? document.body.innerText.slice(0, 4000) : ''")?;
        Ok(v.as_str().unwrap_or("").to_string())
    }

    pub fn load_finished(&self) -> Result<bool> {
        Ok(self.evaluate("document.readyState")?.as_str() == Some("complete"))
    }

    /// The challenge this page shows, if any.
    pub fn challenge(&self) -> Result<Option<crate::Challenge>> {
        let i = self.info()?;
        Ok(crate::detect_challenge(&i.url, &i.title, &self.text_excerpt().unwrap_or_default()))
    }

    /// A PNG of the page, base64-encoded (diagnostics of the live test).
    pub fn screenshot_png(&self) -> Result<String> {
        let r = self.call("Page.captureScreenshot", json!({"format": "png"}))?;
        str_of(&r, "data")
    }

    /// Blocks images, fonts and media (not needed when nobody looks at the page).
    pub fn block_heavy(&self, on: bool) -> Result<()> {
        let urls: &[&str] = if on { HEAVY } else { &[] };
        self.call("Network.setBlockedURLs", json!({"urls": urls})).map(|_| ())
    }
}

/// An event that ends the page with this session and target.
fn ends(e: &crate::cdp::Event, session: &str, target: &str) -> bool {
    let id = |k: &str| e.params.get(k).and_then(Value::as_str);
    matches!(
        e.method.as_str(),
        "Target.detachedFromTarget" | "Target.targetDestroyed" | "Target.targetCrashed"
    ) && (id("sessionId") == Some(session) || id("targetId") == Some(target))
}

fn str_of(v: &Value, k: &str) -> Result<String> {
    v.get(k)
        .and_then(Value::as_str)
        .map(str::to_string)
        .ok_or_else(|| Error::Protocol(format!("no {k} in the reply")))
}
