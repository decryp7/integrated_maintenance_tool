//! The browser ports on the real window: scripts evaluated with `eval_with_callback`, replies checked by a
//! one-shot `ReplyGate` (nonce, origin, size). Thin on purpose: what the scripts return is parsed, classified
//! and judged in `imt_ai::browser`, where it is tested.

use super::window::{browser_label, open};
use imt_ai::browser::policy::ScrollDirection;
use imt_ai::browser::{
    capture_from, scripts, snapshot_from, BrowserError, DomSnapshot, Element, Nonce, PageCapture, PageDriver, PageReader, ReplyError,
    ReplyGate, WebUrl,
};
use serde_json::{Map, Value};
use std::sync::mpsc::sync_channel;
use std::time::Duration;
use tauri::{AppHandle, Manager, Runtime, WebviewWindow};

/// How long a page may take to answer a script.
const EVAL_TIMEOUT: Duration = Duration::from_secs(8);
/// How long a page load after an action may take before the agent goes on.
const LOAD_TIMEOUT: Duration = Duration::from_secs(15);
/// How long to look for a page load an action may have started.
const SETTLE: Duration = Duration::from_millis(400);

/// One window's browser, read (and, for the chat driving it, driven) from a worker thread. Never call it on
/// the main thread: the replies arrive there.
pub struct TauriPage<R: Runtime> {
    pub app: AppHandle<R>,
    pub owner: String,
    /// The chat whose agent turn may drive the browser (`None`: read only).
    pub chat: Option<String>,
}

impl<R: Runtime> TauriPage<R> {
    fn hub(&self) -> Result<tauri::State<'_, super::BrowserState>, BrowserError> {
        self.app.try_state::<super::BrowserState>().ok_or(BrowserError::NotOpen)
    }

    fn window(&self) -> Result<WebviewWindow<R>, BrowserError> {
        self.app
            .get_webview_window(&browser_label(&self.owner))
            .ok_or(BrowserError::NotOpen)
    }

    /// Evaluates one script and checks its reply.
    fn eval(&self, script: impl FnOnce(&Nonce) -> String) -> Result<Map<String, Value>, BrowserError> {
        let w = self.window()?;
        let origin = w.url().ok().and_then(|u| WebUrl::parse(u.as_str()).ok()).map(|u| u.origin());
        let nonce = Nonce::new();
        let mut gate = ReplyGate::new(nonce.clone(), origin);
        let (tx, rx) = sync_channel::<String>(1);
        w.eval_with_callback(script(&nonce), move |raw| {
            let _ = tx.try_send(raw);
        })
        .map_err(|e| BrowserError::Failed(format!("the page could not be read: {e}")))?;
        let raw = rx.recv_timeout(EVAL_TIMEOUT).map_err(|_| BrowserError::Timeout)?;
        gate.accept(&raw).map_err(|e| match e {
            ReplyError::Script(s) if s == "gone" => BrowserError::PageChanged("the element is gone".into()),
            ReplyError::Script(s) if s == "changed" => BrowserError::PageChanged("the element is a different one now".into()),
            ReplyError::Script(s) if s == "password" => BrowserError::Failed("refused: that is a password field".into()),
            other => other.into(),
        })
    }

    fn driving(&self) -> Result<(), BrowserError> {
        match &self.chat {
            Some(chat) if self.hub()?.is_driving(&self.owner, chat) => Ok(()),
            _ => Err(BrowserError::Failed("another chat is driving the browser now".into())),
        }
    }

    /// After an action: if it started a page load, wait for it (bounded).
    fn settle(&self, loads_before: u64) {
        let Ok(hub) = self.hub() else { return };
        if hub.wait_loaded(&self.owner, loads_before, SETTLE) {
            return;
        }
        if hub.page(&self.owner).loading {
            hub.wait_loaded(&self.owner, loads_before, LOAD_TIMEOUT);
        }
    }

    fn loads(&self) -> u64 {
        self.hub().map(|h| h.page(&self.owner).loads).unwrap_or(0)
    }

    fn act(&self, script: impl FnOnce(&Nonce) -> String) -> Result<(), BrowserError> {
        self.driving()?;
        let before = self.loads();
        self.eval(script)?;
        self.settle(before);
        Ok(())
    }
}

impl<R: Runtime> PageReader for TauriPage<R> {
    fn capture(&self) -> Result<PageCapture, BrowserError> {
        self.eval(scripts::capture).map(|m| capture_from(&m))
    }

    fn snapshot(&self) -> Result<DomSnapshot, BrowserError> {
        self.eval(scripts::snapshot).map(|m| snapshot_from(&m))
    }
}

impl<R: Runtime> PageDriver for TauriPage<R> {
    fn navigate(&self, url: &WebUrl) -> Result<(), BrowserError> {
        self.driving()?;
        let before = self.loads();
        open(&self.app, &self.owner, url, false).map_err(BrowserError::Failed)?;
        self.hub()?.wait_loaded(&self.owner, before, LOAD_TIMEOUT);
        Ok(())
    }

    fn click(&self, element: &Element) -> Result<(), BrowserError> {
        self.act(|n| scripts::click(n, element))
    }

    fn type_text(&self, element: &Element, text: &str, submit: bool) -> Result<(), BrowserError> {
        self.act(|n| scripts::type_text(n, element, text, submit))
    }

    fn scroll(&self, direction: ScrollDirection) -> Result<(), BrowserError> {
        self.act(|n| scripts::scroll(n, direction))
    }

    fn back(&self) -> Result<(), BrowserError> {
        self.act(|n| scripts::history(n, true))
    }
}
