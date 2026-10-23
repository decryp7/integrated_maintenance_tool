//! The local model behind the AI aliases of the name matcher (`debrief_core::names::alias_job`): Pitwall's
//! OpenAI-compatible endpoint on this computer.
//!
//! Optional by design: without a configured, reachable Pitwall [`PitwallModel::connect`] fails and the feature is
//! simply off. Nothing leaves the machine: only a loopback address is accepted (`localhost`, `127.0.0.1`, `[::1]`),
//! no proxy is used, redirects are not followed, and answers are read with a size cap.

use crate::error::{AiError, Result};
use debrief_core::names::alias_job::{AliasModel, MAX_REPLY_BYTES};
use serde_json::{json, Value};
use std::net::IpAddr;
use std::time::Duration;

/// Longest a request may take (a batch of names on a small local model).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(120);
const CONNECT_TIMEOUT: Duration = Duration::from_millis(800);

/// Pitwall's chat endpoint as an [`AliasModel`].
pub struct PitwallModel {
    agent: ureq::Agent,
    base: String,
    model: String,
}

/// The base address with its path trimmed of slashes, if it is an `http` URL on a loopback host.
pub fn loopback_base(url: &str) -> Result<String> {
    let u = url::Url::parse(url.trim()).map_err(|e| AiError::Invalid(format!("the Pitwall address is not a URL: {e}")))?;
    if u.scheme() != "http" {
        return Err(AiError::Invalid(
            "the Pitwall address must start with http:// (it runs on this computer)".into(),
        ));
    }
    let local = match u.host() {
        Some(url::Host::Domain(d)) => d.eq_ignore_ascii_case("localhost"),
        Some(url::Host::Ipv4(ip)) => IpAddr::V4(ip).is_loopback(),
        Some(url::Host::Ipv6(ip)) => IpAddr::V6(ip).is_loopback(),
        None => false,
    };
    if !local || !u.username().is_empty() || u.password().is_some() {
        return Err(AiError::Invalid(
            "the Pitwall address must be on this computer (localhost) without credentials".into(),
        ));
    }
    Ok(u.as_str().trim_end_matches('/').to_string())
}

impl PitwallModel {
    /// Connects to Pitwall at `base_url` and picks `model`, or the first model it lists. Fails (the feature stays off)
    /// when the address is not local or Pitwall does not answer.
    pub fn connect(base_url: &str, model: Option<&str>) -> Result<PitwallModel> {
        let base = loopback_base(base_url)?;
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(CONNECT_TIMEOUT))
            .timeout_global(Some(REQUEST_TIMEOUT))
            .proxy(None)
            .user_agent("debrief-ai/0.1")
            .build()
            .into();
        let model = match model.map(str::trim).filter(|m| !m.is_empty()) {
            Some(m) => m.to_string(),
            None => {
                let v = get_json(&agent, &format!("{base}/v1/models"))?;
                v["data"][0]["id"]
                    .as_str()
                    .map(str::to_string)
                    .ok_or_else(|| AiError::Engine("Pitwall lists no model".into()))?
            }
        };
        Ok(PitwallModel { agent, base, model })
    }
}

fn get_json(agent: &ureq::Agent, url: &str) -> Result<Value> {
    let mut r = agent
        .get(url)
        .call()
        .map_err(|e| AiError::Engine(format!("Pitwall is not reachable: {e}")))?;
    if !r.status().is_success() {
        return Err(AiError::Engine(format!("Pitwall answered {}", r.status())));
    }
    let text = r
        .body_mut()
        .with_config()
        .limit(MAX_REPLY_BYTES as u64)
        .read_to_string()
        .map_err(|e| AiError::Engine(format!("Pitwall's answer cannot be read: {e}")))?;
    serde_json::from_str(&text).map_err(|e| AiError::Malformed(format!("Pitwall's answer is not JSON: {e}")))
}

impl AliasModel for PitwallModel {
    fn version(&self) -> String {
        format!("pitwall:{}", self.model)
    }

    fn complete(&self, prompt: &str) -> std::result::Result<String, String> {
        let body = json!({
            "model": self.model,
            "messages": [{"role": "user", "content": prompt}],
            "temperature": 0.0,
            "max_tokens": 1500,
            "stream": false,
        });
        let mut r = self
            .agent
            .post(&format!("{}/v1/chat/completions", self.base))
            .header("Content-Type", "application/json")
            .send(body.to_string())
            .map_err(|e| format!("Pitwall is not reachable: {e}"))?;
        if !r.status().is_success() {
            return Err(format!("Pitwall answered {}", r.status()));
        }
        let text = r
            .body_mut()
            .with_config()
            .limit(MAX_REPLY_BYTES as u64)
            .read_to_string()
            .map_err(|e| format!("Pitwall's answer cannot be read: {e}"))?;
        let v: Value = serde_json::from_str(&text).map_err(|e| format!("Pitwall's answer is not JSON: {e}"))?;
        v["choices"][0]["message"]["content"]
            .as_str()
            .map(str::to_string)
            .ok_or_else(|| "Pitwall's answer has no text".to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_local_http_addresses_are_accepted() {
        assert_eq!(loopback_base("http://127.0.0.1:8080/").unwrap(), "http://127.0.0.1:8080");
        assert_eq!(loopback_base(" http://localhost:9000 ").unwrap(), "http://localhost:9000");
        assert!(loopback_base("http://[::1]:1234").is_ok());
        for bad in [
            "https://localhost",
            "http://example.com",
            "http://192.168.1.5:80",
            "http://user:pw@localhost",
            "nonsense",
            "http://10.0.0.1",
        ] {
            assert!(loopback_base(bad).is_err(), "{bad}");
        }
    }
}
