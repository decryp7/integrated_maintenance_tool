//! The Hugging Face adapter for `ModelSource`.
//!
//! Endpoints used (verified against the live API on 2026-10-17):
//! * `GET  {endpoint}/api/models/{repo}` -> `gated` (false | "auto" | "manual"), `cardData.license`
//! * `POST {endpoint}/api/models/{repo}/paths-info/{rev}` with `{"paths":[file]}` -> `[{type, size, lfs:{oid,size}}]`
//!   (`lfs.oid` is the file's SHA-256)
//! * `GET  {endpoint}/{repo}/resolve/{rev}/{file}` -> 302 to a signed CDN URL (`*.hf.co`, `*.huggingface.co`)
//!   that serves `Range` requests (206 with `Content-Range`).
//!
//! Safety: HTTPS only, redirects are followed by hand and only to allow-listed hosts, the access token (if
//! any) is sent to the Hugging Face host only and never to a CDN, and the file is fetched in bounded `Range`
//! chunks so a stalled connection times out instead of hanging.

use super::source::{ModelSource, RemoteFile};
use crate::catalog::is_safe_file_name;
use crate::error::{AiError, Result};
use serde_json::{json, Value};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use ureq::http::{Response, Uri};
use ureq::Body;

pub const DEFAULT_ENDPOINT: &str = "https://huggingface.co";
/// Hosts (and their sub-domains) that may serve model files.
const ALLOWED_DOMAINS: &[&str] = &["huggingface.co", "hf.co"];
const MAX_HOPS: usize = 6;
const MAX_JSON_BYTES: u64 = 4 * 1024 * 1024;

#[derive(Debug, Clone)]
pub struct HfConfig {
    /// Base URL; set to a mirror on corporate networks. Must be https.
    pub endpoint: String,
    pub revision: String,
    /// Extra hosts allowed to serve files (for a mirror's CDN).
    pub extra_hosts: Vec<String>,
    /// HTTP(S) proxy for downloads only, e.g. `http://proxy.corp:3128`.
    pub proxy: Option<String>,
    /// Access token for gated repositories. Held in memory only: never written to disk or logs.
    pub token: Option<String>,
    /// Trust the operating system's certificate store (for TLS-inspecting proxies).
    pub system_certs: bool,
    pub connect_timeout: Duration,
    pub api_timeout: Duration,
    /// Bytes requested per `Range` request.
    pub chunk_bytes: u64,
    pub chunk_timeout: Duration,
    insecure_loopback: bool,
}

impl Default for HfConfig {
    fn default() -> Self {
        HfConfig {
            endpoint: DEFAULT_ENDPOINT.into(),
            revision: "main".into(),
            extra_hosts: vec![],
            proxy: None,
            token: None,
            system_certs: false,
            connect_timeout: Duration::from_secs(15),
            api_timeout: Duration::from_secs(30),
            chunk_bytes: 32 * 1024 * 1024,
            chunk_timeout: Duration::from_secs(180),
            insecure_loopback: false,
        }
    }
}

impl HfConfig {
    /// For tests against a local mock server: allows plain http to loopback addresses only.
    #[doc(hidden)]
    pub fn local_test(endpoint: &str) -> Self {
        HfConfig {
            endpoint: endpoint.into(),
            insecure_loopback: true,
            chunk_bytes: 64 * 1024,
            connect_timeout: Duration::from_secs(2),
            api_timeout: Duration::from_secs(5),
            chunk_timeout: Duration::from_secs(5),
            ..HfConfig::default()
        }
    }
}

fn host_matches(host: &str, domain: &str) -> bool {
    host == domain || host.strip_suffix(domain).is_some_and(|p| p.ends_with('.'))
}

fn parse(url: &str) -> Result<Uri> {
    url.parse::<Uri>()
        .map_err(|_| AiError::Refused("the model host returned an invalid address".into()))
}

fn strip_query(s: &str) -> String {
    // Signed URLs carry credentials in the query string; keep them out of messages.
    let mut out = String::new();
    for w in s.split_inclusive(char::is_whitespace) {
        out.push_str(match w.find("://") {
            Some(_) => w.split('?').next().unwrap_or(w),
            None => w,
        });
        if w.contains("://") && w.ends_with(char::is_whitespace) && w.contains('?') {
            out.push(' ');
        }
    }
    out
}

fn net(e: ureq::Error) -> AiError {
    AiError::Network(strip_query(&e.to_string()))
}

pub struct HuggingFace {
    cfg: HfConfig,
    api: ureq::Agent,
    dl: ureq::Agent,
    /// The resolved CDN address of the file being downloaded (signed, short-lived).
    resolved: Mutex<Option<(String, String)>>,
}

fn agent(cfg: &HfConfig, timeout: Duration) -> Result<ureq::Agent> {
    let mut b = ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_connect(Some(cfg.connect_timeout))
        .timeout_global(Some(timeout))
        .user_agent("imt-ai/0.1")
        // No proxy from the environment: a proxy is used only when the user configures one.
        .proxy(None);
    if let Some(p) = &cfg.proxy {
        b = b.proxy(Some(
            ureq::Proxy::new(p).map_err(|e| AiError::Invalid(format!("invalid proxy address: {e}")))?,
        ));
    }
    if cfg.system_certs {
        b = b.tls_config(
            ureq::tls::TlsConfig::builder()
                .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                .build(),
        );
    }
    Ok(b.build().into())
}

impl HuggingFace {
    pub fn new(cfg: HfConfig) -> Result<Self> {
        let uri = parse(&cfg.endpoint)?;
        let ok_scheme = uri.scheme_str() == Some("https")
            || (cfg.insecure_loopback && uri.scheme_str() == Some("http") && is_loopback(uri.host().unwrap_or("")));
        if !ok_scheme || uri.host().is_none() {
            return Err(AiError::Refused("the model host address must start with https://".into()));
        }
        Ok(HuggingFace {
            api: agent(&cfg, cfg.api_timeout)?,
            dl: agent(&cfg, cfg.chunk_timeout)?,
            cfg,
            resolved: Mutex::new(None),
        })
    }

    fn endpoint_host(&self) -> String {
        parse(&self.cfg.endpoint)
            .ok()
            .and_then(|u| u.host().map(str::to_string))
            .unwrap_or_default()
    }

    fn allowed(&self, uri: &Uri) -> bool {
        let Some(host) = uri.host() else { return false };
        let scheme_ok =
            uri.scheme_str() == Some("https") || (self.cfg.insecure_loopback && uri.scheme_str() == Some("http") && is_loopback(host));
        let host = host.to_ascii_lowercase();
        scheme_ok
            && (host == self.endpoint_host().to_ascii_lowercase()
                || ALLOWED_DOMAINS.iter().any(|d| host_matches(&host, d))
                || self.cfg.extra_hosts.iter().any(|h| host_matches(&host, &h.to_ascii_lowercase())))
    }

    /// One request following redirects only to allowed hosts. Returns the final response and URL.
    fn send(
        &self,
        agent: &ureq::Agent,
        method: &str,
        url: &str,
        headers: &[(&str, String)],
        body: Option<&Value>,
    ) -> Result<(Response<Body>, String)> {
        let mut url = url.to_string();
        for _ in 0..MAX_HOPS {
            let uri = parse(&url)?;
            if !self.allowed(&uri) {
                return Err(AiError::Refused(format!(
                    "{} is not an allowed model host",
                    uri.host().unwrap_or("that address")
                )));
            }
            let first_party = uri.host().is_some_and(|h| h.eq_ignore_ascii_case(&self.endpoint_host()));
            let resp = {
                let mut req_headers: Vec<(&str, String)> = headers.to_vec();
                if let (true, Some(t)) = (first_party, &self.cfg.token) {
                    req_headers.push(("Authorization", format!("Bearer {t}")));
                }
                let result = if method == "POST" {
                    let mut r = agent.post(&url);
                    for (k, v) in &req_headers {
                        r = r.header(*k, v);
                    }
                    r.header("content-type", "application/json")
                        .send(body.cloned().unwrap_or(Value::Null).to_string())
                } else {
                    let mut r = agent.get(&url);
                    for (k, v) in &req_headers {
                        r = r.header(*k, v);
                    }
                    r.call()
                };
                result.map_err(net)?
            };
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
            return Ok((resp, url));
        }
        Err(AiError::Network("too many redirects".into()))
    }

    fn gated_error(&self, repo: &str) -> AiError {
        AiError::Gated {
            url: format!("{}/{}", self.cfg.endpoint.trim_end_matches('/'), repo),
        }
    }

    fn check_status(&self, status: u16, repo: &str) -> Result<()> {
        match status {
            200..=299 => Ok(()),
            401 | 403 => Err(self.gated_error(repo)),
            404 => Err(AiError::NotFound(format!("{repo} on the model host"))),
            429 => Err(AiError::Network(
                "the model host is rate limiting requests; try again in a minute".into(),
            )),
            500..=599 => Err(AiError::Network(format!("the model host had a problem (status {status})"))),
            _ => Err(AiError::Refused(format!("the model host answered with status {status}"))),
        }
    }

    fn json(&self, resp: Response<Body>) -> Result<Value> {
        let mut buf = Vec::new();
        resp.into_body()
            .into_reader()
            .take(MAX_JSON_BYTES)
            .read_to_end(&mut buf)
            .map_err(|e| AiError::Network(e.to_string()))?;
        serde_json::from_slice(&buf).map_err(|_| AiError::Malformed("the model host sent an unreadable answer".into()))
    }

    fn resolve_url(&self, remote: &RemoteFile, force: bool) -> Result<String> {
        let key = format!("{}/{}", remote.repo, remote.file);
        if !force {
            if let Some((k, u)) = self.resolved.lock().unwrap_or_else(|p| p.into_inner()).as_ref() {
                if *k == key {
                    return Ok(u.clone());
                }
            }
        }
        let start = format!(
            "{}/{}/resolve/{}/{}",
            self.cfg.endpoint.trim_end_matches('/'),
            remote.repo,
            self.cfg.revision,
            remote.file
        );
        let (resp, final_url) = self.send(&self.dl, "GET", &start, &[("Range", "bytes=0-0".to_string())], None)?;
        self.check_status(resp.status().as_u16(), &remote.repo)?;
        *self.resolved.lock().unwrap_or_else(|p| p.into_inner()) = Some((key, final_url.clone()));
        Ok(final_url)
    }

    fn fetch_chunk(
        &self,
        remote: &RemoteFile,
        url: &str,
        pos: u64,
        end: u64,
        sink: &mut dyn FnMut(&[u8]) -> Result<()>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let (resp, _) = self.send(&self.dl, "GET", url, &[("Range", format!("bytes={pos}-{end}"))], None)?;
        let status = resp.status().as_u16();
        if status == 200 && pos > 0 {
            return Err(AiError::Network("the host ignored the resume request".into()));
        }
        if status == 206 {
            let range = resp.headers().get("content-range").and_then(|v| v.to_str().ok()).unwrap_or("");
            let start: Option<u64> = range
                .strip_prefix("bytes ")
                .and_then(|r| r.split(['-', '/']).next())
                .and_then(|s| s.parse().ok());
            if start != Some(pos) {
                return Err(AiError::Network("the host answered a different byte range than requested".into()));
            }
        } else {
            self.check_status(status, &remote.repo)?;
        }
        let want = end - pos + 1;
        let mut reader = resp.into_body().into_reader().take(want);
        let mut buf = vec![0u8; 64 * 1024];
        let mut got = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            let n = reader.read(&mut buf).map_err(|e| AiError::Network(strip_query(&e.to_string())))?;
            if n == 0 {
                break;
            }
            sink(&buf[..n])?;
            got += n as u64;
        }
        if got != want {
            return Err(AiError::Network("the connection closed early".into()));
        }
        Ok(())
    }
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

fn safe_repo(repo: &str) -> bool {
    let parts: Vec<&str> = repo.split('/').collect();
    let ok =
        |p: &str| !p.is_empty() && p != "." && p != ".." && p.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    parts.len() == 2 && parts.iter().all(|p| ok(p))
}

impl ModelSource for HuggingFace {
    fn describe(&self, repo: &str, file: &str) -> Result<RemoteFile> {
        if !safe_repo(repo) || !is_safe_file_name(file) {
            return Err(AiError::Refused("that repository or file name is not acceptable".into()));
        }
        let base = self.cfg.endpoint.trim_end_matches('/');
        let (resp, _) = self.send(&self.api, "GET", &format!("{base}/api/models/{repo}"), &[], None)?;
        self.check_status(resp.status().as_u16(), repo)?;
        let info = self.json(resp)?;
        let gated = !matches!(info.get("gated"), None | Some(Value::Bool(false)) | Some(Value::Null));
        let license = info.pointer("/cardData/license").and_then(|l| match l {
            Value::String(s) => Some(s.clone()),
            Value::Array(a) => Some(a.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", ")),
            _ => None,
        });
        let (resp, _) = self.send(
            &self.api,
            "POST",
            &format!("{base}/api/models/{repo}/paths-info/{}", self.cfg.revision),
            &[],
            Some(&json!({ "paths": [file] })),
        )?;
        self.check_status(resp.status().as_u16(), repo)?;
        let entries = self.json(resp)?;
        let entry = entries.as_array().and_then(|a| {
            a.iter()
                .find(|e| e.get("path").and_then(Value::as_str) == Some(file) && e.get("type").and_then(Value::as_str) == Some("file"))
        });
        let entry = entry.ok_or_else(|| AiError::NotFound(format!("{file} in {repo}")))?;
        let sha256 = entry.pointer("/lfs/oid").and_then(Value::as_str).unwrap_or("").to_ascii_lowercase();
        let size = entry
            .pointer("/lfs/size")
            .or_else(|| entry.get("size"))
            .and_then(Value::as_u64)
            .unwrap_or(0);
        if sha256.len() != 64 || !sha256.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(AiError::Integrity(
                "the host gave no SHA-256 checksum for this file, so it cannot be verified".into(),
            ));
        }
        Ok(RemoteFile {
            repo: repo.into(),
            file: file.into(),
            size,
            sha256,
            license,
            gated,
        })
    }

    fn fetch(&self, remote: &RemoteFile, offset: u64, sink: &mut dyn FnMut(&[u8]) -> Result<()>, cancel: &AtomicBool) -> Result<()> {
        if !safe_repo(&remote.repo) || !is_safe_file_name(&remote.file) {
            return Err(AiError::Refused("that repository or file name is not acceptable".into()));
        }
        let mut pos = offset;
        let mut url = self.resolve_url(remote, false)?;
        let mut refreshed = false;
        while pos < remote.size {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            let end = (pos + self.cfg.chunk_bytes - 1).min(remote.size - 1);
            match self.fetch_chunk(remote, &url, pos, end, &mut *sink, cancel) {
                Ok(()) => pos = end + 1,
                // The signed address expired: resolve again once.
                Err(AiError::Gated { .. }) if !refreshed => {
                    refreshed = true;
                    url = self.resolve_url(remote, true)?;
                }
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}
