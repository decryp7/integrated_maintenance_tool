//! Small helpers shared by the views: DTO conversion, metric selection,
//! percentages, paging and search.

use crate::api::{FrameDto, MetricValues};
use crate::error::{ProfileError, Result};
use crate::limits::query::{MAX_PAGE, MAX_REGEX_SIZE, MAX_SEARCH_LEN};
use crate::model::{Frame, Measurement, MetricId, Profile};
use crate::ops::tree::CallTree;
use regex::{Regex, RegexBuilder};

pub fn frame_dto(profile: &Profile, id: u32) -> FrameDto {
    let f = &profile.frames[id as usize];
    FrameDto {
        id,
        name: f.function.clone(),
        module: f.module.clone(),
        file: f.file.clone(),
        line: f.line,
        kind: f.kind,
        is_framework: f.is_framework,
    }
}

/// Values of a row as a map keyed by metric.
pub fn values(metrics: &[MetricId], row: &[u64]) -> MetricValues {
    metrics.iter().copied().zip(row.iter().copied()).collect()
}

/// The requested metric (or the profile's default) and its index.
pub fn pick_metric(profile: &Profile, tree: &CallTree, metric: Option<MetricId>) -> Result<(MetricId, usize, Measurement)> {
    let id = metric.unwrap_or_else(|| profile.default_metric());
    let k = tree.metric_index(id)?;
    Ok((id, k, profile.metrics[k].measurement))
}

/// `part` as a percentage of `whole` (0 when the whole is 0).
pub fn pct(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        0.0
    } else {
        part as f64 * 100.0 / whole as f64
    }
}

/// Checks paging arguments.
pub fn check_page(limit: usize) -> Result<()> {
    if limit == 0 || limit > MAX_PAGE {
        return Err(ProfileError::invalid(format!("page size must be between 1 and {MAX_PAGE}")));
    }
    Ok(())
}

/// `rows[offset..offset + limit]`, clamped.
pub fn page<T>(mut rows: Vec<T>, offset: usize, limit: usize) -> Vec<T> {
    if offset >= rows.len() {
        return Vec::new();
    }
    rows.drain(..offset);
    rows.truncate(limit);
    rows
}

/// Case-insensitive text or regex matcher for searches.
#[derive(Debug, Clone)]
pub enum Search {
    Text(String),
    Regex(Regex),
}

impl Search {
    /// `None` for an empty search.
    pub fn new(text: Option<&str>, regex: bool) -> Result<Option<Search>> {
        let Some(text) = text.map(str::trim).filter(|t| !t.is_empty()) else {
            return Ok(None);
        };
        if text.len() > MAX_SEARCH_LEN {
            return Err(ProfileError::invalid(format!(
                "the search is longer than {MAX_SEARCH_LEN} characters"
            )));
        }
        if regex {
            let re = RegexBuilder::new(text)
                .case_insensitive(true)
                .size_limit(MAX_REGEX_SIZE)
                .build()
                .map_err(|e| ProfileError::invalid(format!("invalid search pattern: {e}")))?;
            Ok(Some(Search::Regex(re)))
        } else {
            Ok(Some(Search::Text(text.to_lowercase())))
        }
    }

    pub fn is_match(&self, s: &str) -> bool {
        match self {
            Search::Text(t) => s.to_lowercase().contains(t.as_str()),
            Search::Regex(re) => re.is_match(s),
        }
    }

    pub fn matches_frame(&self, f: &Frame) -> bool {
        self.is_match(&f.function)
            || f.module.as_deref().is_some_and(|m| self.is_match(m))
            || f.file.as_deref().is_some_and(|m| self.is_match(m))
    }
}

/// The identity of a function across profiles: name and module.
pub fn function_key(f: &Frame) -> String {
    match &f.module {
        Some(m) => format!("{}\u{1}{m}", f.function),
        None => f.function.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn helpers() {
        assert_eq!(pct(1, 4), 25.0);
        assert_eq!(pct(1, 0), 0.0);
        assert!(check_page(0).is_err());
        assert!(check_page(MAX_PAGE + 1).is_err());
        assert!(check_page(10).is_ok());
        assert_eq!(page(vec![1, 2, 3, 4], 1, 2), [2, 3]);
        assert!(page(vec![1], 5, 2).is_empty());
        assert!(Search::new(Some("  "), false).unwrap().is_none());
        assert!(Search::new(None, true).unwrap().is_none());
        let s = Search::new(Some("JSON"), false).unwrap().unwrap();
        assert!(s.is_match("System.Text.Json.Serialize"));
        let r = Search::new(Some("^sys.*json"), true).unwrap().unwrap();
        assert!(r.is_match("System.Text.Json"));
        assert!(!r.is_match("x System.Text.Json"));
        assert!(Search::new(Some("("), true).is_err());
        assert!(Search::new(Some(&"a".repeat(MAX_SEARCH_LEN + 1)), false).is_err());
        let f = Frame {
            function: "f".into(),
            module: Some("m.dll".into()),
            file: Some("src/x.cs".into()),
            line: None,
            kind: crate::model::FrameKind::Managed,
            is_framework: false,
        };
        assert!(Search::new(Some("x.cs"), false).unwrap().unwrap().matches_frame(&f));
        assert!(Search::new(Some("M.DLL"), false).unwrap().unwrap().matches_frame(&f));
        assert_eq!(function_key(&f), "f\u{1}m.dll");
    }
}
