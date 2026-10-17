//! A small bounded HTML to text extractor. Scripts, styles and other non-content elements are dropped;
//! it never panics and runs in linear time on any input.

fn find_ci(hay: &str, needle: &str, from: usize) -> Option<usize> {
    let h = hay.as_bytes();
    let n = needle.as_bytes();
    if n.is_empty() || h.len() < n.len() || from > h.len() - n.len() {
        return None;
    }
    (from..=h.len() - n.len()).find(|&i| h[i..i + n.len()].eq_ignore_ascii_case(n))
}

const SKIP: &[&str] = &["script", "style", "noscript", "svg", "template", "iframe", "object", "canvas"];
const BLOCK: &[&str] = &[
    "p",
    "div",
    "br",
    "li",
    "tr",
    "h1",
    "h2",
    "h3",
    "h4",
    "h5",
    "h6",
    "section",
    "article",
    "pre",
    "table",
    "ul",
    "ol",
    "hr",
    "blockquote",
    "header",
    "footer",
    "dd",
    "dt",
    "dl",
    "form",
    "main",
    "aside",
];
const MAX_TAG: usize = 4096;

pub fn decode_entities(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find('&') {
        out.push_str(&rest[..i]);
        rest = &rest[i..];
        let end = rest.char_indices().take(12).find(|(_, c)| *c == ';').map(|(j, _)| j);
        let decoded = end.and_then(|j| {
            let name = &rest[1..j];
            let ch = match name {
                "amp" => Some('&'),
                "lt" => Some('<'),
                "gt" => Some('>'),
                "quot" => Some('"'),
                "apos" => Some('\''),
                "nbsp" => Some(' '),
                _ => name
                    .strip_prefix('#')
                    .and_then(|n| match n.strip_prefix(['x', 'X']) {
                        Some(h) => u32::from_str_radix(h, 16).ok(),
                        None => n.parse().ok(),
                    })
                    .and_then(char::from_u32),
            };
            ch.map(|c| (c, j))
        });
        match decoded {
            Some((c, j)) => {
                out.push(c);
                rest = &rest[j + 1..];
            }
            None => {
                out.push('&');
                rest = &rest[1..];
            }
        }
    }
    out.push_str(rest);
    out
}

/// Returns (title, text) with at most `max_chars` characters of text.
pub fn html_to_text(html: &str, max_chars: usize) -> (String, String) {
    let mut out = String::new();
    let mut title = String::new();
    let mut i = 0;
    let b = html.as_bytes();
    let limit = max_chars.saturating_mul(4).max(1024);
    while i < b.len() && out.len() < limit {
        let Some(lt) = html[i..].find('<').map(|p| p + i) else {
            out.push_str(&decode_entities(&html[i..]));
            break;
        };
        out.push_str(&decode_entities(&html[i..lt]));
        let rest = &html[lt..];
        if rest.starts_with("<!--") {
            i = rest.find("-->").map_or(b.len(), |e| lt + e + 3);
            continue;
        }
        if rest.starts_with("<![CDATA[") {
            i = rest.find("]]>").map_or(b.len(), |e| lt + e + 3);
            continue;
        }
        // The end of the tag, respecting quoted attribute values; a runaway '<' is plain text.
        let mut quote = 0u8;
        let mut end = None;
        for (k, &c) in rest.as_bytes().iter().enumerate().take(MAX_TAG) {
            match (quote, c) {
                (0, b'"') | (0, b'\'') => quote = c,
                (q, c) if q != 0 && q == c => quote = 0,
                (0, b'>') => {
                    end = Some(k);
                    break;
                }
                _ => {}
            }
        }
        let Some(end) = end else {
            out.push('<');
            i = lt + 1;
            continue;
        };
        let tag = &rest[1..end];
        i = lt + end + 1;
        if tag.starts_with('!') || tag.starts_with('?') {
            continue;
        }
        let closing = tag.starts_with('/');
        let name: String = tag
            .trim_start_matches('/')
            .chars()
            .take_while(|c| c.is_ascii_alphanumeric())
            .collect::<String>()
            .to_ascii_lowercase();
        if name.is_empty() {
            // "a < b": a stray '<' is text, not a tag.
            out.push('<');
            i = lt + 1;
            continue;
        }
        if name == "title" && !closing {
            let close = find_ci(html, "</title", i).unwrap_or(b.len());
            title = decode_entities(&html[i..close]).split_whitespace().collect::<Vec<_>>().join(" ");
            i = html[close..].find('>').map_or(b.len(), |p| close + p + 1);
            continue;
        }
        if !closing && SKIP.contains(&name.as_str()) && !tag.ends_with('/') {
            let close_tag = format!("</{name}");
            i = match find_ci(html, &close_tag, i) {
                Some(c) => html[c..].find('>').map_or(b.len(), |p| c + p + 1),
                None => b.len(),
            };
            continue;
        }
        if BLOCK.contains(&name.as_str()) {
            if !out.ends_with("\n\n") {
                out.push('\n');
            }
        } else if name == "td" || name == "th" {
            out.push(' ');
        }
    }
    (title, normalise(&out, max_chars))
}

fn normalise(s: &str, max_chars: usize) -> String {
    let mut out = String::new();
    let mut newlines = 0;
    let mut space = false;
    for ch in s.chars() {
        if ch == '\n' {
            newlines += 1;
            space = false;
        } else if ch.is_whitespace() || ch.is_control() {
            space = true;
        } else {
            if newlines > 0 && !out.is_empty() {
                out.push_str(if newlines > 1 { "\n\n" } else { "\n" });
            } else if space && !out.is_empty() {
                out.push(' ');
            }
            newlines = 0;
            space = false;
            out.push(ch);
        }
        if out.chars().count() > max_chars + 16 {
            break;
        }
    }
    if out.chars().count() > max_chars {
        let cut: String = out.chars().take(max_chars).collect();
        format!("{cut}…[truncated]")
    } else {
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_text_and_title() {
        let (t, x) = html_to_text("<html><head><title>Err &amp; Fix</title><style>p{}</style></head><body><h1>Hello</h1><p>One <b>two</b></p><script>alert('x')</script><p>Three</p></body></html>", 1000);
        assert_eq!(t, "Err & Fix");
        assert_eq!(x, "Hello\n\nOne two\n\nThree");
    }

    #[test]
    fn drops_scripts_comments_and_cdata() {
        let (_, x) = html_to_text(
            "a<!-- hidden -->b<script>var x='</p>';</script>c<![CDATA[zzz]]>d<noscript>no</noscript>e<SCRIPT>1</SCRIPT>f",
            100,
        );
        assert_eq!(x, "abcdef");
    }

    #[test]
    fn survives_malformed_input() {
        for h in [
            "<",
            "<<<<",
            "<a href=\"unclosed",
            "<script>never closed",
            "a < b and c > d",
            "<!--",
            "<![CDATA[",
            "<title>open",
            "&;&#xZZ;&#99999999;&amp",
            "\u{0}\u{1}<p>\u{7f}x",
        ] {
            let _ = html_to_text(h, 50);
        }
        let (_, x) = html_to_text("a < b and c > d", 100);
        assert!(x.contains("a < b"));
        let (_, big) = html_to_text(&"<p>word </p>".repeat(100_000), 500);
        assert!(big.chars().count() <= 500 + 20 && big.ends_with("…[truncated]"));
        let (_, deep) = html_to_text(&format!("{}x", "<div>".repeat(50_000)), 100);
        assert_eq!(deep, "x");
    }

    #[test]
    fn entities() {
        assert_eq!(
            decode_entities("&lt;a&gt; &quot;q&quot; &#65;&#x42; &nbsp;&bogus; &"),
            "<a> \"q\" AB  &bogus; &"
        );
    }

    #[test]
    fn whitespace_is_collapsed_and_td_separated() {
        let (_, x) = html_to_text("<table><tr><td>a</td><td>b</td></tr></table>\n\n\n  <p>  c   d </p>", 100);
        assert_eq!(x, "a b\n\nc d");
    }

    #[test]
    fn closed_empty_skip_tags_do_not_swallow_the_page() {
        let (_, x) = html_to_text("x<svg/>y<template/>z", 100);
        assert_eq!(x, "xyz");
    }
}
