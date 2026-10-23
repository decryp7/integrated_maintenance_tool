//! Downloads runtime assets from the llama.cpp GitHub releases, as a [`ModelSource`] so the resumable,
//! verified [`crate::models::Downloader`] does the rest. Only assets listed in the pinned
//! [`RuntimeCatalog`] are fetched, only over https, only from GitHub and its release CDN; the bytes are
//! fetched in `Range` chunks so a download resumes and a stalled chunk times out.

use super::catalog::{RuntimeAsset, RuntimeCatalog};
use crate::error::{AiError, Result};
use crate::models::{ModelSource, RemoteFile};
use crate::net::{self, HttpConfig};
use std::io::Read;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;
use ureq::http::Uri;

const ALLOWED_HOSTS: &[&str] = &[
    "github.com",
    "objects.githubusercontent.com",
    "release-assets.githubusercontent.com",
];
const MAX_HOPS: usize = 6;

#[derive(Debug, Clone)]
pub struct ReleaseConfig {
    /// HTTP(S) proxy for downloads only (the AI download proxy setting).
    pub proxy: Option<String>,
    pub connect_timeout: Duration,
    pub chunk_bytes: u64,
    pub chunk_timeout: Duration,
    /// Tests: serve `https://github.com/...` from this plain-http loopback address instead.
    #[doc(hidden)]
    pub loopback_base: Option<String>,
}

impl Default for ReleaseConfig {
    fn default() -> Self {
        ReleaseConfig {
            proxy: None,
            connect_timeout: Duration::from_secs(15),
            chunk_bytes: 32 * 1024 * 1024,
            chunk_timeout: Duration::from_secs(180),
            loopback_base: None,
        }
    }
}

pub struct GithubReleases {
    cfg: ReleaseConfig,
    catalog: RuntimeCatalog,
    agent: ureq::Agent,
    /// The resolved CDN address of the asset being downloaded (signed, short-lived).
    resolved: Mutex<Option<(String, String)>>,
}

fn is_loopback(host: &str) -> bool {
    host == "localhost"
        || host
            .trim_matches(['[', ']'])
            .parse::<std::net::IpAddr>()
            .is_ok_and(|ip| ip.is_loopback())
}

impl GithubReleases {
    pub fn new(cfg: ReleaseConfig, catalog: RuntimeCatalog) -> Result<Self> {
        let agent = net::agent(&HttpConfig {
            proxy: cfg.proxy.clone(),
            connect_timeout: cfg.connect_timeout,
            timeout: cfg.chunk_timeout,
        })?;
        Ok(GithubReleases {
            agent,
            cfg,
            catalog,
            resolved: Mutex::new(None),
        })
    }

    fn asset(&self, tag: &str, name: &str) -> Result<&RuntimeAsset> {
        if tag != self.catalog.tag {
            return Err(AiError::Refused(format!("runtime release {tag} is not the pinned one")));
        }
        self.catalog
            .platforms
            .values()
            .flatten()
            .flat_map(|b| &b.assets)
            .find(|a| a.name == name)
            .ok_or_else(|| AiError::Refused(format!("{name} is not in the runtime catalog")))
    }

    fn allowed(&self, uri: &Uri) -> bool {
        let Some(host) = uri.host() else { return false };
        let host = host.to_ascii_lowercase();
        match uri.scheme_str() {
            Some("https") => ALLOWED_HOSTS.contains(&host.as_str()),
            Some("http") => self.cfg.loopback_base.is_some() && is_loopback(&host),
            _ => false,
        }
    }

    fn start_url(&self, a: &RuntimeAsset) -> String {
        match &self.cfg.loopback_base {
            Some(base) => a.url.replacen("https://github.com", base.trim_end_matches('/'), 1),
            None => a.url.clone(),
        }
    }

    /// A GET following redirects only to allowed hosts; returns the response and the final address.
    fn get(&self, url: &str, range: &str) -> Result<(ureq::http::Response<ureq::Body>, String)> {
        let mut url = url.to_string();
        for _ in 0..MAX_HOPS {
            let uri: Uri = url
                .parse()
                .map_err(|_| AiError::Refused("the download address is invalid".into()))?;
            if !self.allowed(&uri) {
                return Err(AiError::Refused(format!(
                    "{} is not an allowed download host",
                    uri.host().unwrap_or("that address")
                )));
            }
            let host = uri.host().unwrap_or("").to_string();
            let resp = self
                .agent
                .get(&url)
                .header("Range", range)
                .call()
                .map_err(|e| net::classify(e, &host, self.cfg.proxy.is_some()))?;
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

    fn check_status(status: u16, name: &str) -> Result<()> {
        match status {
            200..=299 => Ok(()),
            404 => Err(AiError::NotFound(format!("{name} on GitHub"))),
            401 | 403 | 429 => Err(AiError::Network(format!(
                "GitHub refused the download (status {status}); try again later"
            ))),
            500..=599 => Err(net::status_error(status, "github.com", "GitHub had a problem")),
            _ => Err(net::status_error(status, "github.com", "")),
        }
    }

    fn resolve(&self, a: &RuntimeAsset, force: bool) -> Result<String> {
        let mut cached = self.resolved.lock().unwrap_or_else(|p| p.into_inner());
        if !force {
            if let Some((k, u)) = cached.as_ref() {
                if *k == a.name {
                    return Ok(u.clone());
                }
            }
        }
        let (resp, url) = self.get(&self.start_url(a), "bytes=0-0")?;
        Self::check_status(resp.status().as_u16(), &a.name)?;
        *cached = Some((a.name.clone(), url.clone()));
        Ok(url)
    }

    fn fetch_chunk(&self, url: &str, pos: u64, end: u64, sink: &mut dyn FnMut(&[u8]) -> Result<()>, cancel: &AtomicBool) -> Result<()> {
        let (resp, _) = self.get(url, &format!("bytes={pos}-{end}"))?;
        let status = resp.status().as_u16();
        match status {
            206 => {
                let range = resp.headers().get("content-range").and_then(|v| v.to_str().ok()).unwrap_or("");
                let start: Option<u64> = range
                    .strip_prefix("bytes ")
                    .and_then(|r| r.split(['-', '/']).next())
                    .and_then(|s| s.parse().ok());
                if start != Some(pos) {
                    return Err(AiError::Network("GitHub answered a different byte range than requested".into()));
                }
            }
            200 if pos > 0 => return Err(AiError::Network("the host ignored the resume request".into())),
            401 | 403 => return Err(AiError::Gated { url: String::new() }),
            s @ (429 | 503) => {
                let after = resp.headers().get("retry-after").and_then(|v| v.to_str().ok());
                return Err(match net::status_error(s, "github.com", "GitHub asked to slow down") {
                    AiError::Connection(n) => AiError::Connection(n.with_retry_after(after)),
                    e => e,
                });
            }
            s => Self::check_status(s, "the runtime")?,
        }
        let want = end - pos + 1;
        let host = url
            .parse::<Uri>()
            .ok()
            .and_then(|u| u.host().map(str::to_string))
            .unwrap_or_default();
        let mut reader = resp.into_body().into_reader().take(want);
        let mut buf = vec![0u8; 64 * 1024];
        let mut got = 0u64;
        loop {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            let n = reader
                .read(&mut buf)
                .map_err(|e| net::classify_io(e, &host, self.cfg.proxy.is_some()))?;
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

impl ModelSource for GithubReleases {
    /// `repo` is the release tag and `file` the asset name; answered from the pinned catalog, no network.
    fn describe(&self, repo: &str, file: &str) -> Result<RemoteFile> {
        let a = self.asset(repo, file)?;
        Ok(RemoteFile {
            repo: repo.into(),
            file: a.name.clone(),
            size: a.size,
            sha256: a.sha256.clone(),
            license: Some("MIT".into()),
            gated: false,
        })
    }

    fn fetch(&self, remote: &RemoteFile, offset: u64, sink: &mut dyn FnMut(&[u8]) -> Result<()>, cancel: &AtomicBool) -> Result<()> {
        self.fetch_range(remote, offset, remote.size, sink, cancel)
    }

    /// One `Range` request to the resolved CDN address must be answered `206` for the same total size.
    fn supports_ranges(&self, remote: &RemoteFile) -> bool {
        let Ok(a) = self.asset(&remote.repo, &remote.file).cloned() else {
            return false;
        };
        let Ok(url) = self.resolve(&a, false) else { return false };
        let Ok((resp, _)) = self.get(&url, "bytes=0-0") else { return false };
        let total = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
            .and_then(|r| r.rsplit('/').next())
            .and_then(|t| t.parse::<u64>().ok());
        resp.status().as_u16() == 206 && total == Some(a.size)
    }

    fn fetch_range(
        &self,
        remote: &RemoteFile,
        start: u64,
        end: u64,
        sink: &mut dyn FnMut(&[u8]) -> Result<()>,
        cancel: &AtomicBool,
    ) -> Result<()> {
        let a = self.asset(&remote.repo, &remote.file)?.clone();
        let end = end.min(a.size);
        let mut pos = start;
        let mut url = self.resolve(&a, false)?;
        let mut refreshed = false;
        while pos < end {
            if cancel.load(Ordering::Relaxed) {
                return Err(AiError::Cancelled);
            }
            let last = (pos + self.cfg.chunk_bytes - 1).min(end - 1);
            match self.fetch_chunk(&url, pos, last, &mut *sink, cancel) {
                Ok(()) => pos = last + 1,
                // The signed CDN address expired: resolve again once.
                Err(AiError::Gated { .. }) if !refreshed => {
                    refreshed = true;
                    url = self.resolve(&a, true)?;
                }
                Err(AiError::Gated { .. }) => return Err(AiError::Network("GitHub refused the download; try again later".into())),
                Err(e) => return Err(e),
            }
        }
        Ok(())
    }
}
