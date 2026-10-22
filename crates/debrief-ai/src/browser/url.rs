//! The addresses the internal browser may show, parsed once at the edge: only `http` and `https` pages with
//! a host and no credentials, of bounded length. Debrief's own origins (`tauri.localhost`, `ipc.localhost`, any
//! `*.localhost`) are never reachable, so a page can never become "local" to the app.

use crate::web::{is_loopback_host, is_private_host};
use serde::Serialize;
use std::fmt;
use url::Url;

/// Longest address the browser accepts (from the user, the model or a page).
pub const MAX_URL_LEN: usize = 2048;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum UrlError {
    #[error("the address is longer than {MAX_URL_LEN} characters")]
    TooLong,
    #[error("this is not a web address")]
    Invalid,
    #[error("only http and https pages open in the browser (not {0}:)")]
    Scheme(String),
    #[error("the address has no host")]
    NoHost,
    #[error("addresses with a user name or password are not opened")]
    Credentials,
    #[error("Debrief's own pages cannot be opened in the browser")]
    AppOrigin,
}

/// An `http` or `https` URL with a host, no credentials and at most [`MAX_URL_LEN`] characters.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WebUrl(Url);

/// A site as approvals see it: the host in lower case, without a leading `www.`.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
pub struct Site(String);

impl Site {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for Site {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

/// Debrief serves its UI from these hosts (Windows and Android use `http(s)://<scheme>.localhost`).
fn is_app_host(host: &str) -> bool {
    host.trim_end_matches('.').to_ascii_lowercase().ends_with(".localhost")
}

impl WebUrl {
    pub fn parse(raw: &str) -> Result<WebUrl, UrlError> {
        let raw = raw.trim();
        if raw.chars().count() > MAX_URL_LEN {
            return Err(UrlError::TooLong);
        }
        if raw.chars().any(char::is_control) {
            return Err(UrlError::Invalid);
        }
        let url = Url::parse(raw).map_err(|_| UrlError::Invalid)?;
        match url.scheme() {
            "http" | "https" => {}
            other => return Err(UrlError::Scheme(other.chars().take(20).collect())),
        }
        let host = url.host_str().filter(|h| !h.is_empty()).ok_or(UrlError::NoHost)?;
        if !url.username().is_empty() || url.password().is_some() {
            return Err(UrlError::Credentials);
        }
        if is_app_host(host) {
            return Err(UrlError::AppOrigin);
        }
        if url.as_str().len() > MAX_URL_LEN * 3 {
            // Percent-encoding grew it past any reasonable size.
            return Err(UrlError::TooLong);
        }
        Ok(WebUrl(url))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }

    pub fn url(&self) -> &Url {
        &self.0
    }

    /// The host (IPv6 addresses in brackets).
    pub fn host(&self) -> &str {
        self.0.host_str().unwrap_or_default()
    }

    pub fn site(&self) -> Site {
        let h = self.host().trim_end_matches('.').to_ascii_lowercase();
        Site(h.strip_prefix("www.").map(str::to_string).unwrap_or(h))
    }

    /// `scheme://host[:port]`, as `location.origin` reports it.
    pub fn origin(&self) -> String {
        self.0.origin().ascii_serialization()
    }

    /// This machine or a private network: the AI never goes there.
    pub fn is_private(&self) -> bool {
        let h = self.host();
        is_private_host(h) || is_loopback_host(h)
    }

    /// Everything after the host (path, query, fragment), percent-decoded, for the leak guard.
    pub fn decoded_tail(&self) -> String {
        let mut tail = self.0.path().to_string();
        if let Some(q) = self.0.query() {
            tail.push('?');
            tail.push_str(q);
        }
        if let Some(f) = self.0.fragment() {
            tail.push('#');
            tail.push_str(f);
        }
        let plus_as_space = tail.replace('+', " ");
        percent_decode(&plus_as_space)
    }
}

impl fmt::Display for WebUrl {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

fn percent_decode(s: &str) -> String {
    percent_encoding::percent_decode_str(s).decode_utf8_lossy().into_owned()
}

/// What the browser window does with a navigation request (from the user, the model, a link or a script).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Navigation {
    Allow,
    /// Blocked, with the reason shown to the user.
    Block(String),
}

/// The rule every navigation of the browser window passes: web pages and the blank start page only.
pub fn check_navigation(raw: &str) -> Navigation {
    if raw == "about:blank" {
        return Navigation::Allow;
    }
    match WebUrl::parse(raw) {
        Ok(_) => Navigation::Allow,
        Err(e) => Navigation::Block(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_pages_parse() {
        let u = WebUrl::parse(" https://www.Example.org/a?b=1#c ").unwrap();
        assert_eq!(u.as_str(), "https://www.example.org/a?b=1#c");
        assert_eq!(u.site().as_str(), "example.org");
        assert_eq!(u.origin(), "https://www.example.org");
        assert_eq!(
            WebUrl::parse("http://example.org:8080/").unwrap().origin(),
            "http://example.org:8080"
        );
        for (raw, err) in [
            ("file:///etc/passwd", UrlError::Scheme("file".into())),
            ("javascript:alert(1)", UrlError::Scheme("javascript".into())),
            ("data:text/html,hi", UrlError::Scheme("data".into())),
            ("tauri://localhost/", UrlError::Scheme("tauri".into())),
            ("ms-settings:privacy", UrlError::Scheme("ms-settings".into())),
            ("http://tauri.localhost/index.html", UrlError::AppOrigin),
            ("https://ipc.localhost/cmd", UrlError::AppOrigin),
            ("https://user:pw@example.org/", UrlError::Credentials),
            ("not a url", UrlError::Invalid),
            ("https://exa\u{0}mple.org", UrlError::Invalid),
        ] {
            assert_eq!(WebUrl::parse(raw), Err(err), "{raw}");
        }
        assert_eq!(
            WebUrl::parse(&format!("https://e.org/{}", "a".repeat(MAX_URL_LEN))),
            Err(UrlError::TooLong)
        );
    }

    #[test]
    fn navigation_allows_web_pages_and_the_blank_page_only() {
        assert_eq!(check_navigation("about:blank"), Navigation::Allow);
        assert_eq!(check_navigation("https://learn.microsoft.com/"), Navigation::Allow);
        // The user may browse a local web server; the AI may not (see `is_private`).
        assert_eq!(check_navigation("http://localhost:8080/"), Navigation::Allow);
        assert!(WebUrl::parse("http://localhost:8080/").unwrap().is_private());
        assert!(WebUrl::parse("http://10.0.0.5/").unwrap().is_private());
        assert!(!WebUrl::parse("https://example.org/").unwrap().is_private());
        for raw in ["about:config", "file:///C:/x", "blob:https://e.org/1", "http://asset.localhost/x"] {
            assert!(matches!(check_navigation(raw), Navigation::Block(_)), "{raw}");
        }
    }

    #[test]
    fn the_tail_is_decoded_for_the_leak_guard() {
        let u = WebUrl::parse("https://e.org/p%20q?h=WEB01+down&ip=10.0.0.5#x").unwrap();
        assert_eq!(u.decoded_tail(), "/p q?h=WEB01 down&ip=10.0.0.5#x");
        assert_eq!(u.host(), "e.org");
    }
}
