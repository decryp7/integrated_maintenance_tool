//! The calls of Pitwall's client contract that Debrief uses (Pitwall ADRs 0003, 0007, 0009): health, pairing, model
//! selection, the model list and streamed chat. Answers are parsed into typed values at once; unknown fields are ignored
//! (the contract grows additively) and Pitwall's typed errors become [`PitwallError`]s that say what to do.

use super::http::{self, HttpError, Request, Response, Timeouts};
use crate::error::AiError;
use serde::Deserialize;
use serde_json::Value;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

/// The client contract version Debrief speaks (`/x/v1`, `apiVersion: 1`).
pub const API_VERSION: u32 = 1;
/// The name Debrief pairs with.
pub const APP_NAME: &str = "Debrief";
/// Largest JSON answer of a quick call.
const MAX_JSON: usize = 1024 * 1024;
/// Largest SSE line.
const MAX_EVENT: usize = 4 * 1024 * 1024;

/// What `GET /x/v1/health` says.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Health {
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub version: String,
    pub api_version: u32,
    /// `serving`, `paused` or `stopped`.
    #[serde(default)]
    pub serving: String,
    #[serde(default)]
    pub offline: bool,
}

impl Health {
    pub fn serving(&self) -> bool {
        self.serving == "serving"
    }
}

/// A pairing request Pitwall accepted: the code the user compares in Pitwall.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairStart {
    pub pairing_id: String,
    pub code: String,
    #[serde(default = "five_minutes")]
    pub expires_in_secs: u64,
}

fn five_minutes() -> u64 {
    300
}

/// How a pairing stands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PairPoll {
    Pending,
    /// Approved; the key comes exactly once (`None` on later polls).
    Approved(Option<String>),
    /// Denied, expired or unknown.
    Denied,
}

/// The model Pitwall would choose.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Selected {
    pub model: String,
    #[serde(default)]
    pub chosen_by: Value,
}

/// One model of `GET /x/v1/models`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelInfo {
    pub id: String,
    #[serde(default)]
    pub capabilities: Vec<String>,
    #[serde(default)]
    pub context_max: Option<u64>,
}

/// Why a call to Pitwall failed, in terms Debrief acts on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PitwallError {
    /// Nothing answers: Pitwall is not running.
    NotRunning(String),
    /// The connection dropped in the middle of an answer: Pitwall stopped.
    Stopped,
    Timeout(&'static str),
    Cancelled,
    /// The key is missing, unknown or revoked (401).
    Unauthorized(String),
    /// Pitwall does not allow this (403): a permission of Debrief's key in Pitwall.
    Forbidden(String),
    /// No installed model fits (404, `preset_target_unavailable`).
    NoModel(String),
    /// Too many requests or a budget is used up (429).
    RateLimited {
        message: String,
        retry_after_secs: Option<u32>,
    },
    /// The model does not fit in memory now (503 `insufficient_memory`).
    Memory(String),
    /// Paused, stopped, queue full or the engine timed out (503, 504).
    Busy(String),
    /// Debrief sent something Pitwall refuses (400, 413).
    Invalid(String),
    /// The engine failed (500) or another status.
    Engine(String),
    /// The answer is not what the contract says.
    Protocol(String),
}

impl std::fmt::Display for PitwallError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PitwallError::NotRunning(_) => f.write_str("Pitwall is not running. Open Pitwall, or use the built-in engine (Settings ▸ AI)"),
            PitwallError::Stopped => {
                f.write_str("Pitwall stopped answering. Open Pitwall again, or use the built-in engine (Settings ▸ AI)")
            }
            PitwallError::Timeout(what) => write!(
                f,
                "Pitwall did not answer in time ({what}). Try again; if Pitwall is busy, wait for it"
            ),
            PitwallError::Cancelled => f.write_str("cancelled"),
            PitwallError::Unauthorized(_) => f.write_str(
                "Pitwall no longer accepts Debrief's key (it was revoked or Pitwall's apps were reset). Connect again in Settings ▸ AI",
            ),
            PitwallError::Forbidden(m) => write!(
                f,
                "Pitwall does not allow this for Debrief: {m}. Change Debrief's permissions in Pitwall ▸ Apps & keys"
            ),
            PitwallError::NoModel(m) => write!(f, "Pitwall has no model that suits Debrief ({m}). Download a chat model in Pitwall"),
            PitwallError::RateLimited { message, .. } => write!(f, "Pitwall asks Debrief to slow down: {message}"),
            PitwallError::Memory(m) => write!(f, "Pitwall cannot load the model now: {m}"),
            PitwallError::Busy(m) => write!(f, "Pitwall cannot answer now: {m}"),
            PitwallError::Invalid(m) => write!(f, "Pitwall refused the request: {m}"),
            PitwallError::Engine(m) => write!(f, "Pitwall's engine failed: {m}"),
            PitwallError::Protocol(m) => write!(
                f,
                "Pitwall answered something Debrief does not understand ({m}); update Debrief or Pitwall"
            ),
        }
    }
}

impl From<HttpError> for PitwallError {
    fn from(e: HttpError) -> Self {
        match e {
            HttpError::Refused(m) => PitwallError::NotRunning(m),
            HttpError::Timeout(w) => PitwallError::Timeout(w),
            HttpError::Cancelled => PitwallError::Cancelled,
            HttpError::Closed => PitwallError::Stopped,
            HttpError::Protocol(m) => PitwallError::Protocol(m),
        }
    }
}

impl From<PitwallError> for AiError {
    fn from(e: PitwallError) -> Self {
        let msg = e.to_string();
        match e {
            PitwallError::Cancelled => AiError::Cancelled,
            PitwallError::NotRunning(_)
            | PitwallError::Stopped
            | PitwallError::Timeout(_)
            | PitwallError::Engine(_)
            | PitwallError::Busy(_) => AiError::Engine(msg),
            PitwallError::Protocol(_) => AiError::Malformed(msg),
            PitwallError::Unauthorized(_) | PitwallError::Forbidden(_) | PitwallError::RateLimited { .. } => AiError::Refused(msg),
            PitwallError::NoModel(_) => AiError::NotFound(msg),
            PitwallError::Memory(_) => AiError::Memory(msg),
            PitwallError::Invalid(_) => AiError::Invalid(msg),
        }
    }
}

impl PitwallError {
    /// Pitwall went away (not running, or the connection dropped): the session's engine is lost.
    pub fn is_gone(&self) -> bool {
        matches!(self, PitwallError::NotRunning(_) | PitwallError::Stopped)
    }
}

/// The typed error of a non-2xx answer (OpenAI `{"error":{"message","code"}}`, extended `{"error":{"code","message"}}`).
fn status_error(status: u16, retry_after: Option<&str>, body: &[u8]) -> PitwallError {
    let v: Value = serde_json::from_slice(body).unwrap_or(Value::Null);
    let message: String = v
        .pointer("/error/message")
        .and_then(Value::as_str)
        .or_else(|| v.get("error").and_then(Value::as_str))
        .unwrap_or("no details")
        .chars()
        .take(500)
        .collect();
    let code = v.pointer("/error/code").and_then(Value::as_str).unwrap_or_default();
    match (status, code) {
        (401, _) => PitwallError::Unauthorized(message),
        (403, _) => PitwallError::Forbidden(message),
        (404, _) => PitwallError::NoModel(message),
        (429, _) => PitwallError::RateLimited {
            message,
            retry_after_secs: retry_after.and_then(|s| s.trim().parse().ok()),
        },
        (503, "insufficient_memory") => PitwallError::Memory(message),
        (503, _) | (504, _) => PitwallError::Busy(message),
        (400, _) | (413, _) => PitwallError::Invalid(message),
        _ => PitwallError::Engine(format!("HTTP {status}: {message}")),
    }
}

/// How the end of a streamed answer looked.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StreamEnd {
    /// `X-Pitwall-Model`: the model that answered.
    pub model: Option<String>,
    /// `stop`, `length`, `tool_calls`, ...; `None` when the stream ended without one.
    pub finish_reason: Option<String>,
    pub prompt_tokens: usize,
    pub completion_tokens: usize,
}

/// A client of one Pitwall server.
#[derive(Debug, Clone)]
pub struct Client {
    pub port: u16,
    key: Option<String>,
}

impl Client {
    pub fn new(port: u16, key: Option<String>) -> Client {
        Client { port, key }
    }

    fn authed<'a>(&self, r: Request<'a>) -> Request<'a> {
        match &self.key {
            Some(k) => r.header("Authorization", format!("Bearer {k}")),
            None => r,
        }
    }

    fn json(&self, req: Request<'_>) -> Result<Value, PitwallError> {
        let r = http::send(self.port, &req, Timeouts::QUICK, None)?;
        let status = r.status;
        let retry = r.header("retry-after").map(str::to_string);
        let body = r.read_all(MAX_JSON)?;
        if !(200..300).contains(&status) {
            return Err(status_error(status, retry.as_deref(), &body));
        }
        serde_json::from_slice(&body).map_err(|e| PitwallError::Protocol(format!("not JSON: {e}")))
    }

    fn typed<T: serde::de::DeserializeOwned>(&self, req: Request<'_>, what: &str) -> Result<T, PitwallError> {
        serde_json::from_value(self.json(req)?).map_err(|e| PitwallError::Protocol(format!("{what}: {e}")))
    }

    /// `GET /x/v1/health` (no key needed).
    pub fn health(&self) -> Result<Health, PitwallError> {
        self.typed(Request::get("/x/v1/health"), "health")
    }

    /// `POST /x/v1/pair`: asks Pitwall to show a pairing request for Debrief.
    pub fn pair(&self) -> Result<PairStart, PitwallError> {
        let body = serde_json::json!({ "name": APP_NAME }).to_string().into_bytes();
        let p: PairStart = self.typed(Request::post("/x/v1/pair", body), "pairing")?;
        if p.code.is_empty() || p.code.len() > 16 || p.pairing_id.is_empty() || p.pairing_id.len() > 128 {
            return Err(PitwallError::Protocol("a pairing answer without a usable code".into()));
        }
        Ok(p)
    }

    /// `GET /x/v1/pair/{id}`.
    pub fn poll(&self, pairing_id: &str) -> Result<PairPoll, PitwallError> {
        if !pairing_id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_') {
            return Err(PitwallError::Protocol("a pairing id with unexpected characters".into()));
        }
        let path = format!("/x/v1/pair/{pairing_id}");
        let v = self.json(Request::get(&path))?;
        Ok(match v.get("status").and_then(Value::as_str) {
            Some("pending") => PairPoll::Pending,
            Some("approved") => PairPoll::Approved(v.get("key").and_then(Value::as_str).and_then(super::keys::parse_key)),
            _ => PairPoll::Denied,
        })
    }

    /// `POST /x/v1/select`: which model would serve `spec`, without running it.
    pub fn select(&self, spec: &str) -> Result<Selected, PitwallError> {
        let body = serde_json::json!({ "model": spec }).to_string().into_bytes();
        self.typed(self.authed(Request::post("/x/v1/select", body)), "select")
    }

    /// `GET /v1/models`: the ids (installed models and the presets that resolve).
    pub fn model_ids(&self) -> Result<Vec<String>, PitwallError> {
        let v = self.json(self.authed(Request::get("/v1/models")))?;
        Ok(v.get("data")
            .and_then(Value::as_array)
            .map(|a| {
                a.iter()
                    .filter_map(|m| m.get("id").and_then(Value::as_str))
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default())
    }

    /// `GET /x/v1/models`.
    pub fn models(&self) -> Result<Vec<ModelInfo>, PitwallError> {
        let v = self.json(self.authed(Request::get("/x/v1/models")))?;
        serde_json::from_value(v.get("models").cloned().unwrap_or(Value::Array(vec![])))
            .map_err(|e| PitwallError::Protocol(format!("models: {e}")))
    }

    /// `POST /v1/chat/completions` with `stream: true`: `on_text` receives the content as it arrives. Cancel closes the
    /// connection, which stops the request in Pitwall.
    pub fn chat_stream(
        &self,
        body: &Value,
        priority: &str,
        timeouts: Timeouts,
        cancel: Option<Arc<AtomicBool>>,
        on_text: &mut dyn FnMut(&str),
    ) -> Result<StreamEnd, PitwallError> {
        let bytes = body.to_string().into_bytes();
        let req = self
            .authed(Request::post("/v1/chat/completions", bytes))
            .header("Accept", "text/event-stream")
            .header("X-Pitwall-Priority", priority);
        let mut r = http::send(self.port, &req, timeouts, cancel)?;
        if !(200..300).contains(&r.status) {
            let retry = r.header("retry-after").map(str::to_string);
            let status = r.status;
            let body = r.read_all(MAX_JSON)?;
            return Err(status_error(status, retry.as_deref(), &body));
        }
        let mut end = StreamEnd {
            model: r.header("x-pitwall-model").map(|m| m.chars().take(200).collect()),
            ..StreamEnd::default()
        };
        read_sse(&mut r, &mut end, on_text)?;
        Ok(end)
    }
}

/// Reads `data:` events until `[DONE]` or the end of the body. An `{"error": ...}` event is the server's error.
fn read_sse(r: &mut Response, end: &mut StreamEnd, on_text: &mut dyn FnMut(&str)) -> Result<(), PitwallError> {
    let mut line: Vec<u8> = Vec::new();
    loop {
        let piece = r.next_piece()?;
        if piece.is_empty() {
            // The body ended without `[DONE]`: an answer that never finished is a dropped connection.
            return if end.finish_reason.is_some() {
                Ok(())
            } else {
                Err(PitwallError::Stopped)
            };
        }
        for &b in &piece {
            if b != b'\n' {
                if line.len() >= MAX_EVENT {
                    return Err(PitwallError::Protocol("an event is too large".into()));
                }
                line.push(b);
                continue;
            }
            let text = String::from_utf8_lossy(&line).trim_end_matches('\r').to_string();
            line.clear();
            let Some(data) = text.strip_prefix("data:") else { continue };
            let data = data.trim_start();
            if data == "[DONE]" {
                return Ok(());
            }
            let v: Value = serde_json::from_str(data).map_err(|e| PitwallError::Protocol(format!("a stream event is not JSON: {e}")))?;
            if let Some(err) = v.get("error") {
                let message = err
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("no details")
                    .chars()
                    .take(500)
                    .collect();
                return Err(PitwallError::Engine(message));
            }
            if let Some(choice) = v.pointer("/choices/0") {
                if let Some(t) = choice.pointer("/delta/content").and_then(Value::as_str) {
                    if !t.is_empty() {
                        on_text(t);
                    }
                }
                if let Some(f) = choice.get("finish_reason").and_then(Value::as_str) {
                    end.finish_reason = Some(f.chars().take(32).collect());
                }
            }
            if let Some(u) = v.get("usage") {
                let n = |k: &str| u.get(k).and_then(Value::as_u64).unwrap_or(0) as usize;
                end.prompt_tokens = n("prompt_tokens");
                end.completion_tokens = n("completion_tokens");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_become_typed_errors_with_advice() {
        let body = |code: &str| format!(r#"{{"error":{{"message":"m","type":"t","code":"{code}"}}}}"#).into_bytes();
        assert!(matches!(
            status_error(401, None, &body("unauthorized")),
            PitwallError::Unauthorized(_)
        ));
        assert!(matches!(status_error(403, None, &body("forbidden")), PitwallError::Forbidden(m) if m == "m"));
        assert!(matches!(
            status_error(404, None, &body("preset_target_unavailable")),
            PitwallError::NoModel(_)
        ));
        assert_eq!(
            status_error(429, Some("5"), &body("rate_limited")),
            PitwallError::RateLimited {
                message: "m".into(),
                retry_after_secs: Some(5)
            }
        );
        assert!(matches!(
            status_error(503, None, &body("insufficient_memory")),
            PitwallError::Memory(_)
        ));
        assert!(matches!(status_error(503, None, &body("overloaded")), PitwallError::Busy(_)));
        assert!(matches!(status_error(504, None, &body("timeout")), PitwallError::Busy(_)));
        assert!(matches!(
            status_error(413, None, &body("request_too_large")),
            PitwallError::Invalid(_)
        ));
        assert!(matches!(status_error(500, None, b"not json"), PitwallError::Engine(m) if m.contains("no details")));
        // Ollama's shape.
        assert!(matches!(status_error(400, None, br#"{"error":"bad"}"#), PitwallError::Invalid(m) if m == "bad"));
    }

    #[test]
    fn errors_map_to_the_engine_errors_and_say_what_to_do() {
        type Check = fn(&AiError) -> bool;
        let cases: Vec<(PitwallError, Check)> = vec![
            (PitwallError::Cancelled, |e| *e == AiError::Cancelled),
            (
                PitwallError::NotRunning("x".into()),
                |e| matches!(e, AiError::Engine(m) if m.contains("not running")),
            ),
            (
                PitwallError::Stopped,
                |e| matches!(e, AiError::Engine(m) if m.contains("stopped answering")),
            ),
            (PitwallError::Timeout("x"), |e| matches!(e, AiError::Engine(_))),
            (PitwallError::Busy("x".into()), |e| matches!(e, AiError::Engine(_))),
            (PitwallError::Engine("x".into()), |e| matches!(e, AiError::Engine(_))),
            (
                PitwallError::Protocol("x".into()),
                |e| matches!(e, AiError::Malformed(m) if m.contains("update")),
            ),
            (
                PitwallError::Unauthorized("x".into()),
                |e| matches!(e, AiError::Refused(m) if m.contains("Connect again")),
            ),
            (
                PitwallError::Forbidden("x".into()),
                |e| matches!(e, AiError::Refused(m) if m.contains("Apps & keys")),
            ),
            (
                PitwallError::RateLimited {
                    message: "x".into(),
                    retry_after_secs: None,
                },
                |e| matches!(e, AiError::Refused(_)),
            ),
            (
                PitwallError::NoModel("x".into()),
                |e| matches!(e, AiError::NotFound(m) if m.contains("chat model")),
            ),
            (PitwallError::Memory("x".into()), |e| matches!(e, AiError::Memory(_))),
            (PitwallError::Invalid("x".into()), |e| matches!(e, AiError::Invalid(_))),
        ];
        for (p, check) in cases {
            let e = AiError::from(p.clone());
            assert!(check(&e), "{p:?} -> {e:?}");
        }
        assert!(PitwallError::Stopped.is_gone() && PitwallError::NotRunning(String::new()).is_gone());
        assert!(!PitwallError::Cancelled.is_gone());
        assert_eq!(PitwallError::from(HttpError::Closed), PitwallError::Stopped);
        assert!(matches!(
            PitwallError::from(HttpError::Refused("r".into())),
            PitwallError::NotRunning(_)
        ));
        assert!(matches!(
            PitwallError::from(HttpError::Protocol("p".into())),
            PitwallError::Protocol(_)
        ));
        assert_eq!(PitwallError::from(HttpError::Cancelled), PitwallError::Cancelled);
        assert_eq!(PitwallError::from(HttpError::Timeout("t")), PitwallError::Timeout("t"));
    }
}
