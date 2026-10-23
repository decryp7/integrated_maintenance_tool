//! The scanner: URLs first (masked out), then tokens.

use super::classify::classify_token;
use super::{Entity, EntityKind, Extracted, MAX_NAME_CHARS, MAX_PER_EVENT, MAX_SCAN_CHARS};
use std::collections::HashSet;

/// Longest token looked at (characters).
const MAX_TOKEN: usize = 300;

/// The entities in the message and exception of an event.
pub fn extract(texts: &[&str]) -> Extracted {
    let mut out = Extracted::default();
    let mut seen: HashSet<(EntityKind, String)> = HashSet::new();
    let mut push = |out: &mut Extracted, kind: EntityKind, name: String| {
        if name.is_empty() || name.chars().count() > MAX_NAME_CHARS || out.entities.len() >= MAX_PER_EVENT {
            return;
        }
        if seen.insert((kind, name.to_lowercase())) {
            if kind == EntityKind::Guid {
                out.guids += 1;
            }
            out.entities.push(Entity { kind, name });
        }
    };
    for text in texts {
        let text: String = text.chars().take(MAX_SCAN_CHARS).collect();
        let (masked, urls) = urls(&text);
        for (kind, name) in urls {
            push(&mut out, kind, name);
        }
        let mut prev = String::new();
        for tok in tokens(&masked) {
            let found = classify_token(&tok, &prev);
            prev = tok.trim_matches(|c: char| !c.is_alphanumeric()).to_lowercase();
            for (kind, name) in found {
                push(&mut out, kind, name);
            }
        }
    }
    out
}

fn is_delim(c: char) -> bool {
    c.is_whitespace()
        || matches!(
            c,
            ',' | ';' | '<' | '>' | '(' | ')' | '[' | ']' | '{' | '}' | '|' | '\'' | '`' | '='
        )
}

/// Tokens: split at delimiters, but a double-quoted stretch (a path with spaces) is one token.
fn tokens(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut chars = text.chars().peekable();
    let flush = |cur: &mut String, out: &mut Vec<String>| {
        if !cur.is_empty() && cur.chars().count() <= MAX_TOKEN {
            out.push(std::mem::take(cur));
        }
        cur.clear();
    };
    while let Some(c) = chars.next() {
        if c == '"' {
            flush(&mut cur, &mut out);
            let mut quoted = String::new();
            let mut closed = false;
            let mut n = 0;
            for q in chars.by_ref() {
                n += 1;
                if q == '"' {
                    closed = true;
                    break;
                }
                if n > MAX_TOKEN {
                    break;
                }
                quoted.push(q);
            }
            if closed {
                // A quoted stretch with spaces is one token; without, it is split as usual.
                if quoted.contains(' ') && !quoted.trim().is_empty() {
                    out.push(quoted.trim().to_string());
                } else {
                    for t in quoted.split(is_delim).filter(|t| !t.is_empty()) {
                        out.push(t.to_string());
                    }
                }
            }
        } else if is_delim(c) {
            flush(&mut cur, &mut out);
        } else {
            cur.push(c);
        }
    }
    flush(&mut cur, &mut out);
    out
}

/// URLs in `text` as (host and URL entities), and the text with them blanked.
fn urls(text: &str) -> (String, Vec<(EntityKind, String)>) {
    let mut found = Vec::new();
    let mut masked = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let before = &rest[..at];
        let scheme_len = before
            .chars()
            .rev()
            .take_while(|c| c.is_ascii_alphanumeric() || matches!(c, '+' | '.' | '-'))
            .count();
        let scheme_start = at - before.chars().rev().take(scheme_len).map(char::len_utf8).sum::<usize>();
        let scheme = &rest[scheme_start..at];
        let after = &rest[at + 3..];
        let end = after
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | ')' | '}' | '`'))
            .unwrap_or(after.len());
        let url = &after[..end];
        masked.push_str(&rest[..scheme_start]);
        masked.push(' ');
        if scheme.is_empty() || !scheme.starts_with(|c: char| c.is_ascii_alphabetic()) || url.is_empty() {
            masked.push_str(&rest[scheme_start..at + 3]);
            rest = after;
            continue;
        }
        if let Some((host, shown)) = shape(scheme, url) {
            let kind = if host.parse::<std::net::IpAddr>().is_ok() {
                EntityKind::Ip
            } else {
                EntityKind::Host
            };
            found.push((EntityKind::Url, shown));
            found.push((kind, host));
        }
        rest = &after[end..];
    }
    masked.push_str(rest);
    (masked, found)
}

/// The host and `scheme://host[:port]/path` of a URL's text after `://` (no user info, query or fragment).
fn shape(scheme: &str, after: &str) -> Option<(String, String)> {
    let cut = after.find(['/', '?', '#']).unwrap_or(after.len());
    let authority = &after[..cut];
    let hostport = authority.rsplit('@').next().unwrap_or(authority);
    let (host, port) = match hostport.strip_prefix('[') {
        Some(v6) => {
            let (h, p) = v6.split_once(']')?;
            (h.to_string(), p.to_string())
        }
        None => match hostport.rsplit_once(':') {
            Some((h, p)) if p.chars().all(|c| c.is_ascii_digit()) => (h.to_string(), format!(":{p}")),
            _ => (hostport.to_string(), String::new()),
        },
    };
    if host.is_empty() || !host.chars().all(|c| c.is_alphanumeric() || matches!(c, '.' | '-' | '_' | ':')) {
        return None;
    }
    let path_end = after[cut..].find(['?', '#']).map(|i| cut + i).unwrap_or(after.len());
    let path = &after[cut..path_end];
    let host = host.to_lowercase();
    let hostshown = if host.contains(':') { format!("[{host}]") } else { host.clone() };
    Some((host.clone(), format!("{}://{hostshown}{port}{path}", scheme.to_lowercase())))
}
