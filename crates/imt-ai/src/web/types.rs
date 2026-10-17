//! The web ports. Web content is untrusted data, never instructions.

use crate::error::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SearchHit {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageText {
    pub title: String,
    pub url: String,
    pub text: String,
}

/// Contract: sends exactly the query it is given (already sanitised and approved) and nothing else,
/// over HTTPS, with timeouts; returns at most `max` hits; never panics on malformed answers.
pub trait WebSearch: Send + Sync {
    fn name(&self) -> &str;
    fn search(&self, query: &str, max: usize) -> Result<Vec<SearchHit>>;
}

/// Contract: fetches one HTTPS page, at most 2 MB, and returns at most 20,000 characters of text with
/// scripts and styles removed.
pub trait PageFetcher: Send + Sync {
    fn fetch_text(&self, url: &str) -> Result<PageText>;
}
