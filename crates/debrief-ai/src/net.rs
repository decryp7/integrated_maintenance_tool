//! The one way the AI layer talks HTTPS (model files from Hugging Face, inference runtimes from GitHub), and
//! what a failed request means in words the user can act on.
//!
//! * [`agent`] builds every client: certificates are checked against the **operating system's** trust store
//!   (`rustls-platform-verifier`), so a corporate TLS-inspecting proxy (Zscaler, Netskope, ...) whose root
//!   certificate IT installed in Windows works like in the browser. ureq's default would be the built-in
//!   Mozilla list, which such proxies always fail. Certificate checking is never switched off.
//! * A proxy is used only when the user configured one (the *Download proxy* setting), never from the
//!   environment.
//! * [`classify`] turns a transport error into an [`AiError::Connection`] with a [`NetKind`]: a certificate
//!   problem, a proxy that wants a sign-in, an unreachable proxy, no network, a timeout or an HTTP status.

use crate::error::{AiError, Result};
use std::fmt;
use std::time::Duration;
use ureq::tls::{RootCerts, TlsConfig};

/// How a client connects: the optional proxy and its timeouts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpConfig {
    /// HTTP(S) proxy, e.g. `http://proxy.corp:3128` (credentials may be given as `http://user:pass@host:port`).
    pub proxy: Option<String>,
    pub connect_timeout: Duration,
    /// Longest a whole request (including reading its body) may take.
    pub timeout: Duration,
}

/// TLS settings for every client: the operating system verifies the server certificate.
pub fn tls_config() -> TlsConfig {
    TlsConfig::builder().root_certs(RootCerts::PlatformVerifier).build()
}

/// A client with Debrief's rules: OS certificate store, the configured proxy only, no automatic redirects (the
/// adapters follow them by hand to allow-listed hosts) and HTTP statuses returned rather than raised.
pub fn agent(cfg: &HttpConfig) -> Result<ureq::Agent> {
    let proxy = match &cfg.proxy {
        Some(p) => Some(ureq::Proxy::new(p).map_err(|e| AiError::Invalid(format!("invalid proxy address: {e}")))?),
        None => None,
    };
    Ok(ureq::Agent::config_builder()
        .http_status_as_error(false)
        .max_redirects(0)
        .timeout_connect(Some(cfg.connect_timeout))
        .timeout_global(Some(cfg.timeout))
        .user_agent("debrief-ai/0.1")
        .proxy(proxy)
        .tls_config(tls_config())
        .build()
        .into())
}

/// Why a request failed, in the user's terms. The UI shows a headline and next steps per kind.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetKind {
    /// The server's certificate could not be verified: almost always a TLS-inspecting proxy whose root
    /// certificate is not in the OS store (or a wrong clock).
    Tls,
    /// The proxy asks for a sign-in (HTTP 407).
    ProxyAuth,
    /// The configured proxy cannot be reached or refused the connection.
    Proxy,
    /// No network, the name does not resolve, or the host refuses connections.
    Offline,
    Timeout,
    /// The server (or a proxy in between) answered with this HTTP status.
    Http(u16),
}

/// A classified connection failure. `detail` is the technical cause, without query strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NetError {
    pub kind: NetKind,
    pub host: String,
    pub detail: String,
    /// The server's `Retry-After` (whole seconds) on a 429/503, when it gave one.
    pub retry_after_secs: Option<u32>,
}

fn trust_store() -> &'static str {
    if cfg!(windows) {
        "Windows' certificate store"
    } else if cfg!(target_os = "macos") {
        "the macOS keychain"
    } else {
        "the system certificate store"
    }
}

impl fmt::Display for NetError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let host = if self.host.is_empty() { "the server" } else { self.host.as_str() };
        match self.kind {
            NetKind::Tls => write!(
                f,
                "the secure connection to {host} could not be verified ({}). A proxy or security tool on your network \
                 probably intercepts HTTPS (for example Zscaler): its root certificate must be installed in {}; ask your IT \
                 department, or download the file manually",
                self.detail,
                trust_store()
            ),
            NetKind::ProxyAuth => write!(
                f,
                "the proxy asks for a sign-in ({}): put the user name and password in the Download proxy setting \
                 (http://user:password@proxy:port), or download the file manually",
                self.detail
            ),
            NetKind::Proxy => write!(
                f,
                "the download proxy could not be used ({}): check the Download proxy setting in Settings → AI",
                self.detail
            ),
            NetKind::Offline => write!(
                f,
                "cannot reach {host} ({}): check that this computer is online; if your network needs a proxy, set Download \
                 proxy in Settings → AI",
                self.detail
            ),
            NetKind::Timeout => write!(
                f,
                "{host} did not answer in time ({}): try again later, or download the file manually",
                self.detail
            ),
            NetKind::Http(status) => {
                write!(f, "{host} answered with status {status}")?;
                if !self.detail.is_empty() {
                    write!(f, " ({})", self.detail)?;
                }
                if status == 403 {
                    f.write_str("; a proxy on your network may block the download: download the file manually or ask your IT department")?;
                }
                Ok(())
            }
        }
    }
}

impl NetError {
    pub fn new(kind: NetKind, host: &str, detail: impl Into<String>) -> NetError {
        NetError {
            kind,
            host: host.to_string(),
            detail: strip_query(&detail.into()),
            retry_after_secs: None,
        }
    }

    /// Records the server's `Retry-After` header value (delta-seconds form; dates are ignored).
    pub fn with_retry_after(mut self, header: Option<&str>) -> NetError {
        self.retry_after_secs = header.and_then(|v| v.trim().parse().ok());
        self
    }

    /// The server asked to slow down (429 or 503): a segmented download uses fewer connections.
    pub fn is_throttle(&self) -> bool {
        matches!(self.kind, NetKind::Http(429 | 503))
    }

    /// Worth retrying after a pause: timeouts, dropped networks, rate limits and server errors.
    pub fn is_transient(&self) -> bool {
        match self.kind {
            NetKind::Offline | NetKind::Timeout | NetKind::Proxy => true,
            NetKind::Http(s) => s == 429 || s >= 500,
            NetKind::Tls | NetKind::ProxyAuth => false,
        }
    }
}

/// Removes query strings from the URLs in `s`: signed CDN addresses carry credentials there.
pub fn strip_query(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for w in s.split_inclusive(char::is_whitespace) {
        if w.contains("://") {
            if let Some(i) = w.find('?') {
                out.push_str(&w[..i]);
                if w.ends_with(char::is_whitespace) {
                    out.push(' ');
                }
                continue;
            }
        }
        out.push_str(w);
    }
    out
}

fn tls_cause(e: &std::io::Error) -> Option<String> {
    let inner = e.get_ref()?;
    inner.downcast_ref::<rustls::Error>().map(|r| r.to_string())
}

/// What an I/O error during connect or transfer means (`via_proxy`: a proxy is configured, so a refused
/// or unreachable connection is the proxy's).
pub fn classify_io(e: std::io::Error, host: &str, via_proxy: bool) -> AiError {
    use std::io::ErrorKind as K;
    if let Some(cause) = tls_cause(&e) {
        return AiError::Connection(NetError::new(NetKind::Tls, host, cause));
    }
    // ureq wraps its own errors (e.g. a timeout while reading the body) in io errors.
    if e.get_ref().is_some_and(|i| i.is::<ureq::Error>()) {
        return classify(ureq::Error::from(e), host, via_proxy);
    }
    let unreachable = if via_proxy { NetKind::Proxy } else { NetKind::Offline };
    let kind = match e.kind() {
        K::TimedOut | K::WouldBlock => NetKind::Timeout,
        K::ConnectionRefused | K::HostUnreachable | K::NetworkUnreachable | K::NetworkDown | K::AddrNotAvailable => unreachable,
        K::ConnectionReset | K::ConnectionAborted | K::BrokenPipe | K::UnexpectedEof => {
            return AiError::Network(format!(
                "the connection to {host} was interrupted ({})",
                strip_query(&e.to_string())
            ));
        }
        _ => return AiError::Network(strip_query(&e.to_string())),
    };
    AiError::Connection(NetError::new(kind, host, e.to_string()))
}

/// What a failed request means. `host` is the server asked; `via_proxy` as for [`classify_io`].
pub fn classify(e: ureq::Error, host: &str, via_proxy: bool) -> AiError {
    use ureq::Error as E;
    let conn = |kind, detail: String| AiError::Connection(NetError::new(kind, host, detail));
    let unreachable = if via_proxy { NetKind::Proxy } else { NetKind::Offline };
    match e {
        E::Io(io) => classify_io(io, host, via_proxy),
        E::Rustls(r) => conn(NetKind::Tls, r.to_string()),
        E::Tls(m) => conn(NetKind::Tls, m.to_string()),
        E::Pem(p) => conn(NetKind::Tls, p.to_string()),
        E::Timeout(t) => conn(NetKind::Timeout, format!("timeout: {t}")),
        E::HostNotFound => conn(unreachable, "the name could not be resolved".into()),
        E::ConnectionFailed => conn(unreachable, "the connection failed".into()),
        E::ConnectProxyFailed(reason) if reason.contains("407") => conn(NetKind::ProxyAuth, reason),
        E::ConnectProxyFailed(reason) => conn(NetKind::Proxy, reason),
        E::InvalidProxyUrl => AiError::Invalid("invalid proxy address".into()),
        E::StatusCode(407) => conn(NetKind::ProxyAuth, "status 407".into()),
        E::StatusCode(s) => conn(NetKind::Http(s), String::new()),
        other => AiError::Network(strip_query(&other.to_string())),
    }
}

/// An HTTP status the adapters do not handle themselves (407 is the proxy asking for a sign-in).
pub fn status_error(status: u16, host: &str, detail: &str) -> AiError {
    let kind = if status == 407 { NetKind::ProxyAuth } else { NetKind::Http(status) };
    AiError::Connection(NetError::new(kind, host, detail))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Error as IoError, ErrorKind};

    fn cfg(proxy: Option<&str>) -> HttpConfig {
        HttpConfig {
            proxy: proxy.map(str::to_string),
            connect_timeout: Duration::from_secs(3),
            timeout: Duration::from_secs(9),
        }
    }

    fn kind(e: &AiError) -> Option<NetKind> {
        match e {
            AiError::Connection(n) => Some(n.kind),
            _ => None,
        }
    }

    #[test]
    fn every_client_verifies_with_the_os_certificate_store_and_only_the_configured_proxy() {
        assert!(matches!(tls_config().root_certs(), RootCerts::PlatformVerifier));
        assert!(!tls_config().disable_verification());
        let a = agent(&cfg(None)).unwrap();
        let c = a.config();
        assert!(matches!(c.tls_config().root_certs(), RootCerts::PlatformVerifier));
        assert!(!c.tls_config().disable_verification());
        assert!(c.proxy().is_none(), "no proxy from the environment");
        assert_eq!(c.max_redirects(), 0);
        assert!(!c.http_status_as_error());
        assert_eq!(c.timeouts().connect, Some(Duration::from_secs(3)));
        assert_eq!(c.timeouts().global, Some(Duration::from_secs(9)));
        let p = agent(&cfg(Some("http://proxy.corp:3128"))).unwrap();
        assert_eq!(p.config().proxy().map(|p| p.host().to_string()).as_deref(), Some("proxy.corp"));
        assert!(matches!(p.config().tls_config().root_certs(), RootCerts::PlatformVerifier));
        assert!(matches!(agent(&cfg(Some("::bad::"))), Err(AiError::Invalid(_))));
    }

    #[test]
    fn transport_errors_are_classified() {
        let h = "huggingface.co";
        assert_eq!(kind(&classify(ureq::Error::HostNotFound, h, false)), Some(NetKind::Offline));
        assert_eq!(kind(&classify(ureq::Error::HostNotFound, h, true)), Some(NetKind::Proxy));
        assert_eq!(kind(&classify(ureq::Error::ConnectionFailed, h, false)), Some(NetKind::Offline));
        assert_eq!(
            kind(&classify(
                ureq::Error::ConnectProxyFailed("proxy server responded 407/407".into()),
                h,
                true
            )),
            Some(NetKind::ProxyAuth)
        );
        assert_eq!(
            kind(&classify(
                ureq::Error::ConnectProxyFailed("proxy server did not respond".into()),
                h,
                true
            )),
            Some(NetKind::Proxy)
        );
        assert_eq!(kind(&classify(ureq::Error::Tls("bad"), h, false)), Some(NetKind::Tls));
        let cert = rustls::Error::InvalidCertificate(rustls::CertificateError::UnknownIssuer);
        assert_eq!(kind(&classify(ureq::Error::Rustls(cert.clone()), h, false)), Some(NetKind::Tls));
        // How ureq reports a failed handshake: an io error carrying the rustls error.
        let wrapped = IoError::new(ErrorKind::InvalidData, cert);
        let e = classify(ureq::Error::Io(wrapped), h, false);
        assert_eq!(kind(&e), Some(NetKind::Tls));
        let msg = e.to_string();
        assert!(msg.contains("Zscaler") && msg.contains(h) && msg.contains("manually"), "{msg}");
        assert_eq!(kind(&classify(ureq::Error::StatusCode(407), h, true)), Some(NetKind::ProxyAuth));
        assert_eq!(kind(&classify(ureq::Error::StatusCode(502), h, false)), Some(NetKind::Http(502)));
        assert!(matches!(classify(ureq::Error::InvalidProxyUrl, h, true), AiError::Invalid(_)));
        assert!(matches!(classify(ureq::Error::TooManyRedirects, h, false), AiError::Network(_)));
        // a timeout ureq wrapped in an io error while reading the body
        let t = IoError::other(ureq::Error::Timeout(ureq::Timeout::Global));
        assert_eq!(kind(&classify_io(t, h, false)), Some(NetKind::Timeout));
    }

    #[test]
    fn io_errors_are_classified() {
        let h = "github.com";
        let io = |k| IoError::new(k, "x");
        assert_eq!(kind(&classify_io(io(ErrorKind::TimedOut), h, false)), Some(NetKind::Timeout));
        assert_eq!(
            kind(&classify_io(io(ErrorKind::ConnectionRefused), h, false)),
            Some(NetKind::Offline)
        );
        assert_eq!(kind(&classify_io(io(ErrorKind::ConnectionRefused), h, true)), Some(NetKind::Proxy));
        assert_eq!(
            kind(&classify_io(io(ErrorKind::NetworkUnreachable), h, false)),
            Some(NetKind::Offline)
        );
        let reset = classify_io(io(ErrorKind::ConnectionReset), h, false);
        assert!(matches!(&reset, AiError::Network(m) if m.contains("interrupted")), "{reset}");
        assert!(reset.is_transient());
        assert!(matches!(classify_io(io(ErrorKind::Other), h, false), AiError::Network(_)));
    }

    #[test]
    fn messages_say_what_to_do_and_hide_signed_queries() {
        let m = |k| NetError::new(k, "cdn.hf.co", "https://cdn.hf.co/x?X-Amz-Signature=secret failed").to_string();
        let tls = m(NetKind::Tls);
        assert!(!tls.contains("secret") && tls.contains("https://cdn.hf.co/x failed"), "{tls}");
        assert!(tls.contains(trust_store()));
        assert!(m(NetKind::ProxyAuth).contains("user:password"));
        assert!(m(NetKind::Proxy).contains("Download proxy"));
        assert!(m(NetKind::Offline).contains("online"));
        assert!(m(NetKind::Timeout).contains("in time"));
        assert!(m(NetKind::Http(403)).contains("block"));
        assert_eq!(
            NetError::new(NetKind::Http(500), "", "").to_string(),
            "the server answered with status 500"
        );
        assert!(matches!(
            status_error(407, "h", ""),
            AiError::Connection(NetError {
                kind: NetKind::ProxyAuth,
                ..
            })
        ));
        assert_eq!(strip_query("see https://a/b?c=d and x?y"), "see https://a/b and x?y");
        assert_eq!(strip_query("https://a/b?c"), "https://a/b");
    }

    #[test]
    fn only_temporary_failures_are_retried() {
        let t = |k| NetError::new(k, "h", "").is_transient();
        assert!(t(NetKind::Offline) && t(NetKind::Timeout) && t(NetKind::Proxy) && t(NetKind::Http(503)) && t(NetKind::Http(429)));
        assert!(!t(NetKind::Tls) && !t(NetKind::ProxyAuth) && !t(NetKind::Http(404)) && !t(NetKind::Http(403)));
        assert!(AiError::Connection(NetError::new(NetKind::Timeout, "h", "")).is_transient());
        assert!(!AiError::Connection(NetError::new(NetKind::Tls, "h", "")).is_transient());
    }

    #[test]
    fn throttling_carries_the_retry_after_the_server_gave() {
        let e = |st, h| AiError::Connection(NetError::new(NetKind::Http(st), "h", "").with_retry_after(h));
        assert_eq!(e(429, Some(" 7 ")).throttle(), Some(Some(Duration::from_secs(7))));
        assert_eq!(e(503, None).throttle(), Some(None));
        assert_eq!(e(503, Some("Wed, 21 Oct 2026 07:28:00 GMT")).throttle(), Some(None));
        assert_eq!(e(500, None).throttle(), None);
        assert_eq!(AiError::Network("x".into()).throttle(), None);
    }
}
