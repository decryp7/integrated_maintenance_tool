//! The HTTP client for web search and page fetching: HTTPS only (plain http only to a loopback address, for
//! a self-hosted search instance), no cookies, timeouts, a download size cap, and redirects followed by
//! hand and only to https.

use crate::error::{AiError, Result};
use std::io::Read;
use std::time::Duration;
use ureq::http::Uri;

#[derive(Debug, Clone)]
pub struct WebHttpConfig {
    pub proxy: Option<String>,
    pub connect_timeout: Duration,
    pub total_timeout: Duration,
    pub max_bytes: usize,
    /// Allow `http://` to loopback addresses (self-hosted search on this machine, and tests).
    pub allow_loopback_http: bool,
}

impl Default for WebHttpConfig {
    fn default() -> Self {
        WebHttpConfig {
            proxy: None,
            connect_timeout: Duration::from_secs(8),
            total_timeout: Duration::from_secs(20),
            max_bytes: 2 * 1024 * 1024,
            allow_loopback_http: false,
        }
    }
}

pub struct Response {
    pub status: u16,
    pub body: Vec<u8>,
    /// The body was cut at the size cap.
    pub truncated: bool,
    pub content_type: String,
    pub url: String,
}

pub struct WebHttp {
    agent: ureq::Agent,
    cfg: WebHttpConfig,
}

pub fn is_loopback_host(host: &str) -> bool {
    host.eq_ignore_ascii_case("localhost")
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

/// Hosts a page fetch must not reach: this machine and private networks.
pub fn is_private_host(host: &str) -> bool {
    let h = host.trim_matches(['[', ']']).to_ascii_lowercase();
    if h == "localhost"
        || h.ends_with(".local")
        || h.ends_with(".internal")
        || h.ends_with(".localhost")
        || !h.contains('.') && h.parse::<std::net::IpAddr>().is_err()
    {
        return true;
    }
    match h.parse::<std::net::IpAddr>() {
        Ok(std::net::IpAddr::V4(ip)) => {
            ip.is_private() || ip.is_loopback() || ip.is_link_local() || ip.is_unspecified() || ip.is_broadcast()
        }
        Ok(std::net::IpAddr::V6(ip)) => {
            ip.is_loopback() || ip.is_unspecified() || (ip.segments()[0] & 0xfe00) == 0xfc00 || (ip.segments()[0] & 0xffc0) == 0xfe80
        }
        Err(_) => false,
    }
}

impl WebHttp {
    pub fn new(cfg: WebHttpConfig) -> Result<Self> {
        let mut b = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_connect(Some(cfg.connect_timeout))
            .timeout_global(Some(cfg.total_timeout))
            .user_agent("imt-ai/0.1 (+log analysis tool)")
            .proxy(None);
        if let Some(p) = &cfg.proxy {
            b = b.proxy(Some(
                ureq::Proxy::new(p).map_err(|e| AiError::Invalid(format!("invalid proxy address: {e}")))?,
            ));
        }
        Ok(WebHttp {
            agent: b.build().into(),
            cfg,
        })
    }

    /// Plain http to loopback is allowed (self-hosted search, tests).
    pub fn allows_loopback_http(&self) -> bool {
        self.cfg.allow_loopback_http
    }

    fn check(&self, uri: &Uri) -> Result<()> {
        let host = uri.host().ok_or_else(|| AiError::Refused("the address has no host".into()))?;
        match uri.scheme_str() {
            Some("https") => Ok(()),
            Some("http") if self.cfg.allow_loopback_http && is_loopback_host(host) => Ok(()),
            _ => Err(AiError::Refused("only https:// addresses are allowed".into())),
        }
    }

    pub fn get(&self, url: &str, headers: &[(&str, &str)]) -> Result<Response> {
        let mut url = url.to_string();
        for _ in 0..4 {
            let uri: Uri = url.parse().map_err(|_| AiError::Refused("invalid address".into()))?;
            self.check(&uri)?;
            let mut req = self.agent.get(&url);
            for (k, v) in headers {
                req = req.header(*k, *v);
            }
            let resp = req.call().map_err(|e| AiError::Network(redact_query(&e.to_string())))?;
            let status = resp.status().as_u16();
            if matches!(status, 301 | 302 | 303 | 307 | 308) {
                let loc = resp
                    .headers()
                    .get("location")
                    .and_then(|v| v.to_str().ok())
                    .ok_or_else(|| AiError::Network("a redirect without a target".into()))?;
                url = if loc.starts_with('/') {
                    format!(
                        "{}://{}{}",
                        uri.scheme_str().unwrap_or("https"),
                        uri.authority().map(|a| a.as_str()).unwrap_or(""),
                        loc
                    )
                } else {
                    loc.to_string()
                };
                continue;
            }
            let content_type = resp
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("")
                .to_ascii_lowercase();
            let mut body = Vec::new();
            resp.into_body()
                .into_reader()
                .take(self.cfg.max_bytes as u64 + 1)
                .read_to_end(&mut body)
                .map_err(|e| AiError::Network(e.to_string()))?;
            let truncated = body.len() > self.cfg.max_bytes;
            body.truncate(self.cfg.max_bytes);
            return Ok(Response {
                status,
                body,
                truncated,
                content_type,
                url,
            });
        }
        Err(AiError::Network("too many redirects".into()))
    }
}

fn redact_query(s: &str) -> String {
    s.split_whitespace()
        .map(|w| if w.contains("://") { w.split('?').next().unwrap_or(w) } else { w })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_classification() {
        for h in [
            "localhost",
            "127.0.0.1",
            "[::1]",
            "10.1.2.3",
            "192.168.0.9",
            "169.254.1.1",
            "172.16.0.1",
            "intranet",
            "printer.local",
            "db.internal",
            "fd00::1",
            "fe80::1",
            "0.0.0.0",
        ] {
            assert!(is_private_host(h), "{h}");
        }
        for h in ["example.com", "8.8.8.8", "learn.microsoft.com", "2606:4700::1111"] {
            assert!(!is_private_host(h), "{h}");
        }
        assert!(is_loopback_host("localhost") && is_loopback_host("127.0.0.2") && is_loopback_host("[::1]"));
        assert!(!is_loopback_host("192.168.1.1") && !is_loopback_host("example.com"));
    }

    #[test]
    fn query_strings_are_stripped_from_errors() {
        assert_eq!(
            redact_query("failed https://x.test/p?token=abc: boom"),
            "failed https://x.test/p boom"
        );
    }

    #[test]
    fn scheme_rules() {
        let strict = WebHttp::new(WebHttpConfig::default()).unwrap();
        assert!(strict.get("http://example.com", &[]).is_err());
        assert!(strict.get("http://127.0.0.1:1/", &[]).is_err());
        assert!(strict.get("ftp://example.com", &[]).is_err());
        assert!(strict.get("not a url", &[]).is_err());
        assert!(strict.get("https:///x", &[]).is_err());
        assert!(matches!(
            WebHttp::new(WebHttpConfig {
                proxy: Some("::bad::".into()),
                ..WebHttpConfig::default()
            }),
            Err(AiError::Invalid(_))
        ));
    }
}
