//! Compiles regexes that come from users (profiles, searches, filters) with
//! limits, so a pathological pattern fails with a message instead of
//! exhausting memory. The `regex` crate already guarantees linear-time matching.

use anyhow::{anyhow, Result};
use regex::{Regex, RegexBuilder};

/// Longest pattern accepted, in bytes.
pub const MAX_PATTERN_LEN: usize = 4096;
/// Upper bound for the compiled program and the lazy DFA cache, in bytes.
const SIZE_LIMIT: usize = 8 << 20;

pub fn compile(pattern: &str, case_insensitive: bool) -> Result<Regex> {
    if pattern.len() > MAX_PATTERN_LEN {
        return Err(anyhow!("regex is {} bytes long; the limit is {MAX_PATTERN_LEN}", pattern.len()));
    }
    RegexBuilder::new(pattern)
        .case_insensitive(case_insensitive)
        .size_limit(SIZE_LIMIT)
        .dfa_size_limit(SIZE_LIMIT)
        .build()
        .map_err(|e| anyhow!("{e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compiles_within_limits() {
        assert!(compile("(?P<a>\\d+)", false).unwrap().is_match("42"));
        assert!(compile("abc", true).unwrap().is_match("ABC"));
    }

    #[test]
    fn rejects_oversized_patterns() {
        assert!(compile(&"a".repeat(MAX_PATTERN_LEN + 1), false)
            .unwrap_err()
            .to_string()
            .contains("limit"));
        assert!(compile(r"\w{1000}{1000}", false).is_err());
        assert!(compile("(", false).is_err());
    }
}
