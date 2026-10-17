//! Search adapters (SearXNG, Brave Search API) and the page fetcher. No search engine's HTML is scraped.
//!
//! Endpoints (checked on 2026-10-17):
//! * SearXNG: `GET {instance}/search?q=..&format=json` -> `{results:[{title,url,content}]}`; the instance must
//!   enable the json format (otherwise 403).
//! * Brave: `GET https://api.search.brave.com/res/v1/web/search?q=..&count=N` with headers
//!   `X-Subscription-Token: <key>` and `Accept: application/json` -> `{web:{results:[{title,url,description}]}}`;
//!   an invalid key answers 422 `SUBSCRIPTION_TOKEN_INVALID`
//!   (https://api-dashboard.search.brave.com/app/documentation/web-search/get-started).

use super::html::html_to_text;
use super::http::{is_private_host, WebHttp, WebHttpConfig};
use super::secrets::SecretStore;
use super::types::{PageFetcher, PageText, SearchHit, WebSearch};
use crate::assistant::truncate_chars;
use crate::error::{AiError, Result};
use serde_json::Value;
use std::sync::Arc;

pub const BRAVE_ENDPOINT: &str = "https://api.search.brave.com/res/v1/web/search";
pub const BRAVE_SECRET_NAME: &str = "brave_api_key";
const MAX_TEXT_CHARS: usize = 20_000;

fn encode(s: &str) -> String {
    s.bytes()
        .map(|b| {
            if b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.' | b'~') {
                (b as char).to_string()
            } else {
                format!("%{b:02X}")
            }
        })
        .collect()
}

fn hits_from(items: &[Value], url_key: &str, snippet_key: &str, max: usize) -> Vec<SearchHit> {
    items
        .iter()
        .filter_map(|r| {
            let url = r.get(url_key)?.as_str()?.trim();
            if !url.starts_with("https://") {
                return None;
            }
            Some(SearchHit {
                title: truncate_chars(r.get("title").and_then(Value::as_str).unwrap_or(url).trim(), 160),
                url: url.to_string(),
                snippet: truncate_chars(r.get(snippet_key).and_then(Value::as_str).unwrap_or("").trim(), 400),
            })
        })
        .take(max)
        .collect()
}

fn json_of(resp: &super::http::Response) -> Result<Value> {
    if resp.truncated {
        return Err(AiError::Malformed("the search answer was too large".into()));
    }
    serde_json::from_slice(&resp.body).map_err(|_| AiError::Malformed("the search service sent an unreadable answer".into()))
}

pub struct SearxngSearch {
    base: String,
    http: WebHttp,
}

impl SearxngSearch {
    /// `base` is the instance address, for example `https://searx.example.org` (http only for localhost).
    pub fn new(base: &str, mut cfg: WebHttpConfig) -> Result<Self> {
        let base = base.trim().trim_end_matches('/').to_string();
        cfg.allow_loopback_http = true;
        if !(base.starts_with("https://") || base.starts_with("http://")) {
            return Err(AiError::Invalid("the SearXNG address must start with https://".into()));
        }
        Ok(SearxngSearch {
            base,
            http: WebHttp::new(cfg)?,
        })
    }
}

impl WebSearch for SearxngSearch {
    fn name(&self) -> &str {
        "searxng"
    }

    fn search(&self, query: &str, max: usize) -> Result<Vec<SearchHit>> {
        let resp = self.http.get(
            &format!("{}/search?q={}&format=json&language=en", self.base, encode(query)),
            &[("Accept", "application/json")],
        )?;
        match resp.status {
            200 => {}
            403 => {
                return Err(AiError::Refused(
                    "this SearXNG instance has JSON output turned off (add json to search.formats in its settings)".into(),
                ))
            }
            429 => {
                return Err(AiError::Network(
                    "the search service is rate limiting requests; try again in a minute".into(),
                ))
            }
            s => return Err(AiError::Network(format!("the search service answered with status {s}"))),
        }
        let v = json_of(&resp)?;
        let items = v
            .get("results")
            .and_then(Value::as_array)
            .ok_or_else(|| AiError::Malformed("the search answer has no results list".into()))?;
        Ok(hits_from(items, "url", "content", max))
    }
}

pub struct BraveSearch {
    secrets: Arc<dyn SecretStore>,
    endpoint: String,
    http: WebHttp,
}

impl BraveSearch {
    pub fn new(secrets: Arc<dyn SecretStore>, cfg: WebHttpConfig) -> Result<Self> {
        Self::with_endpoint(secrets, BRAVE_ENDPOINT, cfg)
    }

    #[doc(hidden)]
    pub fn with_endpoint(secrets: Arc<dyn SecretStore>, endpoint: &str, mut cfg: WebHttpConfig) -> Result<Self> {
        if !endpoint.starts_with("https://") {
            cfg.allow_loopback_http = true;
        }
        Ok(BraveSearch {
            secrets,
            endpoint: endpoint.to_string(),
            http: WebHttp::new(cfg)?,
        })
    }
}

impl WebSearch for BraveSearch {
    fn name(&self) -> &str {
        "brave"
    }

    fn search(&self, query: &str, max: usize) -> Result<Vec<SearchHit>> {
        let key = self
            .secrets
            .get(BRAVE_SECRET_NAME)?
            .ok_or_else(|| AiError::Refused("no Brave Search API key is stored; add one in the AI settings".into()))?;
        let url = format!("{}?q={}&count={}", self.endpoint, encode(query), max.clamp(1, 20));
        let resp = self
            .http
            .get(&url, &[("Accept", "application/json"), ("X-Subscription-Token", key.as_str())])?;
        match resp.status {
            200 => {}
            401 | 403 | 422 => {
                return Err(AiError::Refused(
                    "Brave Search rejected the API key; check the key in the AI settings".into(),
                ))
            }
            429 => return Err(AiError::Network("Brave Search is rate limiting requests; try again later".into())),
            s => return Err(AiError::Network(format!("Brave Search answered with status {s}"))),
        }
        let v = json_of(&resp)?;
        let items = v
            .pointer("/web/results")
            .and_then(Value::as_array)
            .ok_or_else(|| AiError::Malformed("the Brave answer has no web results".into()))?;
        Ok(hits_from(items, "url", "description", max))
    }
}

pub struct HttpPageFetcher {
    http: WebHttp,
}

impl HttpPageFetcher {
    pub fn new(cfg: WebHttpConfig) -> Result<Self> {
        Ok(HttpPageFetcher { http: WebHttp::new(cfg)? })
    }
}

impl PageFetcher for HttpPageFetcher {
    fn fetch_text(&self, url: &str) -> Result<PageText> {
        let uri: ureq::http::Uri = url.parse().map_err(|_| AiError::Refused("invalid address".into()))?;
        let host = uri.host().unwrap_or("");
        if host.is_empty() || is_private_host(host) && !self.http.allows_loopback_http() {
            return Err(AiError::Refused(
                "pages on this machine or on private networks are never fetched".into(),
            ));
        }
        let resp = self.http.get(url, &[("Accept", "text/html, text/plain;q=0.9")])?;
        if resp.status != 200 {
            return Err(AiError::Network(format!("the page answered with status {}", resp.status)));
        }
        let is_html = resp.content_type.contains("html");
        if !(is_html || resp.content_type.starts_with("text/plain") || resp.content_type.is_empty()) {
            return Err(AiError::Refused("that address is not a text page".into()));
        }
        let raw = String::from_utf8_lossy(&resp.body).into_owned();
        let (title, text) = if is_html || resp.content_type.is_empty() {
            html_to_text(&raw, MAX_TEXT_CHARS)
        } else {
            (String::new(), truncate_chars(&raw, MAX_TEXT_CHARS))
        };
        Ok(PageText {
            title: if title.is_empty() { host.to_string() } else { title },
            url: url.to_string(),
            text,
        })
    }
}
