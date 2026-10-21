//! Search engines of the internal browser and the address box: what the user types becomes either a page
//! address or a search with the chosen engine. The browser only builds the engine's ordinary search page
//! URL; IMT never calls a search API.

use super::url::{UrlError, WebUrl};
use serde::{Deserialize, Serialize};

/// Longest search text the browser sends to an engine.
pub const MAX_SEARCH_CHARS: usize = 500;
/// Longest user-defined search URL template.
pub const MAX_TEMPLATE_CHARS: usize = 512;
/// The placeholder for the search text in a template.
pub const QUERY_PLACEHOLDER: &str = "{q}";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SearchEngine {
    /// The default: what most users expect from a search box.
    #[default]
    Google,
    DuckDuckGo,
    Bing,
    StackOverflow,
    /// The user's own URL template with `{q}` (see [`SearchTemplate`]).
    Custom,
}

impl SearchEngine {
    /// Every engine, in the order pickers list them (the default first).
    pub const ALL: [SearchEngine; 5] = [
        SearchEngine::Google,
        SearchEngine::DuckDuckGo,
        SearchEngine::Bing,
        SearchEngine::StackOverflow,
        SearchEngine::Custom,
    ];

    pub fn id(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "duckDuckGo",
            SearchEngine::Google => "google",
            SearchEngine::Bing => "bing",
            SearchEngine::StackOverflow => "stackOverflow",
            SearchEngine::Custom => "custom",
        }
    }

    pub fn parse(id: &str) -> Option<SearchEngine> {
        SearchEngine::ALL.into_iter().find(|e| e.id() == id)
    }

    pub fn label(self) -> &'static str {
        match self {
            SearchEngine::DuckDuckGo => "DuckDuckGo",
            SearchEngine::Google => "Google",
            SearchEngine::Bing => "Bing",
            SearchEngine::StackOverflow => "Stack Overflow",
            SearchEngine::Custom => "Custom",
        }
    }

    /// Where the engine's results page puts its organic results: CSS selectors, tried in order (the results-page link
    /// collector marks the links inside the first one found). A custom engine has none: `main` is tried then.
    pub fn results_region(self) -> &'static [&'static str] {
        match self {
            SearchEngine::Google => &["#rso", "#search"],
            SearchEngine::Bing => &["#b_results"],
            SearchEngine::DuckDuckGo => &["ol.react-results--main", "[data-testid=mainline]", "#links", ".results"],
            SearchEngine::StackOverflow => &[".js-search-results", "#mainbar"],
            SearchEngine::Custom => &[],
        }
    }

    fn template(self) -> Option<&'static str> {
        match self {
            SearchEngine::DuckDuckGo => Some("https://duckduckgo.com/?q={q}"),
            SearchEngine::Google => Some("https://www.google.com/search?q={q}"),
            SearchEngine::Bing => Some("https://www.bing.com/search?q={q}"),
            SearchEngine::StackOverflow => Some("https://stackoverflow.com/search?q={q}"),
            SearchEngine::Custom => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SearchError {
    #[error("type something to search for")]
    Empty,
    #[error("the custom search address is not set; set it in Settings ▸ Web browser")]
    NoTemplate,
    #[error("the custom search address must contain {{q}} where the search text goes")]
    NoPlaceholder,
    #[error("the custom search address is longer than {MAX_TEMPLATE_CHARS} characters")]
    TemplateTooLong,
    #[error("the custom search address is not valid: {0}")]
    BadTemplate(UrlError),
    #[error("{0}")]
    Url(#[from] UrlError),
}

/// A user-defined search URL: `http(s)` with `{q}` where the search text goes, e.g.
/// `https://search.example.org/?q={q}`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SearchTemplate(String);

impl SearchTemplate {
    pub fn parse(raw: &str) -> Result<SearchTemplate, SearchError> {
        let t = raw.trim();
        if t.chars().count() > MAX_TEMPLATE_CHARS {
            return Err(SearchError::TemplateTooLong);
        }
        if !t.contains(QUERY_PLACEHOLDER) {
            return Err(SearchError::NoPlaceholder);
        }
        WebUrl::parse(&t.replace(QUERY_PLACEHOLDER, "test")).map_err(SearchError::BadTemplate)?;
        Ok(SearchTemplate(t.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The engine settings the browser searches with.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SearchSettings {
    pub engine: SearchEngine,
    pub custom: Option<SearchTemplate>,
}

impl SearchSettings {
    /// The engine's search page for `text` (trimmed and capped; encoded as a form value).
    pub fn search_url(&self, engine: SearchEngine, text: &str) -> Result<WebUrl, SearchError> {
        let text: String = text.trim().chars().take(MAX_SEARCH_CHARS).collect();
        if text.trim().is_empty() {
            return Err(SearchError::Empty);
        }
        let template = match engine.template() {
            Some(t) => t.to_string(),
            None => self.custom.as_ref().ok_or(SearchError::NoTemplate)?.0.clone(),
        };
        let encoded: String = url::form_urlencoded::byte_serialize(text.trim().as_bytes()).collect();
        Ok(WebUrl::parse(&template.replace(QUERY_PLACEHOLDER, &encoded))?)
    }

    /// What the address box opens: a web address as is (`https://` added to a bare host name), anything
    /// else as a search with `engine`. Other schemes (`file:`, `javascript:`) are refused.
    pub fn resolve_address(&self, input: &str, engine: SearchEngine) -> Result<WebUrl, SearchError> {
        let t = input.trim();
        if t.is_empty() {
            return Err(SearchError::Empty);
        }
        let lower = t.to_ascii_lowercase();
        if lower.starts_with("http://") || lower.starts_with("https://") {
            return Ok(WebUrl::parse(t)?);
        }
        if let Some((scheme, _)) = t.split_once(':') {
            let looks_like_scheme = !scheme.is_empty()
                && scheme.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.'))
                && !t[scheme.len() + 1..].starts_with(|c: char| c.is_ascii_digit());
            if looks_like_scheme && !t.contains(' ') && !scheme.contains('.') {
                return Err(SearchError::Url(UrlError::Scheme(scheme.chars().take(20).collect())));
            }
        }
        if looks_like_host(t) {
            if let Ok(u) = WebUrl::parse(&format!("https://{t}")) {
                return Ok(u);
            }
        }
        self.search_url(engine, t)
    }
}

/// `learn.microsoft.com/dotnet` or `localhost:8080`: no spaces, a dotted or port-qualified host first. The last
/// label must look like a top-level domain (2 to 12 lower-case letters) or the host be an IPv4 address, so type
/// names such as `System.TimeoutException` are searched for.
fn looks_like_host(t: &str) -> bool {
    if t.contains(char::is_whitespace) {
        return false;
    }
    let host = t.split(['/', '?', '#']).next().unwrap_or("");
    let name = host.split(':').next().unwrap_or("");
    let labels: Vec<&str> = name.split('.').collect();
    let dotted = labels.len() >= 2
        && labels
            .iter()
            .all(|l| !l.is_empty() && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-'))
        && labels
            .last()
            .is_some_and(|tld| (2..=12).contains(&tld.len()) && tld.chars().all(|c| c.is_ascii_lowercase()))
        || name.parse::<std::net::Ipv4Addr>().is_ok();
    dotted || (name == "localhost" && host.contains(':'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s() -> SearchSettings {
        SearchSettings::default()
    }

    #[test]
    fn every_builtin_engine_builds_its_search_page() {
        let cases = [
            (SearchEngine::DuckDuckGo, "https://duckduckgo.com/?q=Timeout+%2B+0x80131500"),
            (SearchEngine::Google, "https://www.google.com/search?q=Timeout+%2B+0x80131500"),
            (SearchEngine::Bing, "https://www.bing.com/search?q=Timeout+%2B+0x80131500"),
            (
                SearchEngine::StackOverflow,
                "https://stackoverflow.com/search?q=Timeout+%2B+0x80131500",
            ),
        ];
        for (e, want) in cases {
            assert_eq!(s().search_url(e, "  Timeout + 0x80131500 ").unwrap().as_str(), want);
            assert_eq!(SearchEngine::parse(e.id()), Some(e));
            assert!(!e.label().is_empty());
        }
        assert_eq!(SearchEngine::parse("yahoo"), None);
        assert_eq!(s().search_url(SearchEngine::Google, "  "), Err(SearchError::Empty));
        let long = "a".repeat(MAX_SEARCH_CHARS + 50);
        assert!(s().search_url(SearchEngine::Bing, &long).unwrap().as_str().len() < MAX_SEARCH_CHARS + 60);
    }

    #[test]
    fn google_is_the_default_and_listed_first() {
        assert_eq!(SearchEngine::default(), SearchEngine::Google);
        assert_eq!(SearchSettings::default().engine, SearchEngine::Google);
        assert_eq!(SearchEngine::ALL[0], SearchEngine::default());
        assert_eq!(
            SearchEngine::ALL.last(),
            Some(&SearchEngine::Custom),
            "the custom template comes last"
        );
    }

    #[test]
    fn custom_templates_are_validated() {
        assert_eq!(s().search_url(SearchEngine::Custom, "x"), Err(SearchError::NoTemplate));
        assert_eq!(SearchTemplate::parse("https://s.example/?q="), Err(SearchError::NoPlaceholder));
        assert!(matches!(SearchTemplate::parse("file:///{q}"), Err(SearchError::BadTemplate(_))));
        assert!(matches!(
            SearchTemplate::parse("http://x.localhost/{q}"),
            Err(SearchError::BadTemplate(_))
        ));
        assert_eq!(
            SearchTemplate::parse(&format!("https://s.example/?q={{q}}&{}", "a".repeat(600))),
            Err(SearchError::TemplateTooLong)
        );
        let custom = SearchSettings {
            engine: SearchEngine::Custom,
            custom: Some(SearchTemplate::parse(" https://search.intra.example/find?text={q}&src=imt ").unwrap()),
        };
        assert_eq!(
            custom.search_url(SearchEngine::Custom, "a b").unwrap().as_str(),
            "https://search.intra.example/find?text=a+b&src=imt"
        );
        assert_eq!(
            custom.custom.as_ref().unwrap().as_str(),
            "https://search.intra.example/find?text={q}&src=imt"
        );
    }

    #[test]
    fn the_address_box_opens_pages_or_searches() {
        let r = |t: &str| s().resolve_address(t, s().engine).map(|u| u.as_str().to_string());
        assert_eq!(r("https://learn.microsoft.com/x").unwrap(), "https://learn.microsoft.com/x");
        assert_eq!(r("learn.microsoft.com/dotnet").unwrap(), "https://learn.microsoft.com/dotnet");
        assert_eq!(r("localhost:8080/health").unwrap(), "https://localhost:8080/health");
        assert_eq!(r("10.1.2.3").unwrap(), "https://10.1.2.3/");
        assert_eq!(
            r("System.TimeoutException").unwrap(),
            "https://www.google.com/search?q=System.TimeoutException"
        );
        assert_eq!(
            r("java.lang.nullpointerexception").unwrap(),
            "https://www.google.com/search?q=java.lang.nullpointerexception"
        );
        assert_eq!(
            r("connection reset by peer").unwrap(),
            "https://www.google.com/search?q=connection+reset+by+peer"
        );
        assert_eq!(
            r("error: 0x80004005").unwrap(),
            "https://www.google.com/search?q=error%3A+0x80004005"
        );
        assert_eq!(r("ERROR:5").unwrap(), "https://www.google.com/search?q=ERROR%3A5");
        assert!(matches!(r("javascript:alert(1)"), Err(SearchError::Url(UrlError::Scheme(_)))));
        assert!(matches!(r("file:///C:/x"), Err(SearchError::Url(UrlError::Scheme(_)))));
        assert!(matches!(r("http://tauri.localhost/"), Err(SearchError::Url(UrlError::AppOrigin))));
        assert_eq!(r(" "), Err(SearchError::Empty));
    }
}
