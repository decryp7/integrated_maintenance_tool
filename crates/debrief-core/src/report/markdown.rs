//! The Markdown subset of report text blocks, parsed into a typed outline
//! that the HTML and PDF renderers draw (never passed through as markup).
//!
//! Supported: `#`/`##`/`###` headings, paragraphs, `-`/`*` and `1.` lists,
//! `>` quotes, fenced code blocks, `---` rules, and inline `**strong**`,
//! `*em*`/`_em_`, `` `code` `` and `[text](https://...)` links.

/// A block of text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdBlock {
    Heading(u8, Vec<Inline>),
    Paragraph(Vec<Inline>),
    List { ordered: bool, items: Vec<Vec<Inline>> },
    Quote(Vec<Inline>),
    Code(String),
    Rule,
}

/// A run of text with one style.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Inline {
    Text(String),
    Strong(String),
    Em(String),
    Code(String),
    /// Only `http(s)` and `mailto` targets become links; others stay text.
    Link {
        text: String,
        url: String,
    },
}

impl Inline {
    /// The visible text.
    pub fn text(&self) -> &str {
        match self {
            Inline::Text(t) | Inline::Strong(t) | Inline::Em(t) | Inline::Code(t) => t,
            Inline::Link { text, .. } => text,
        }
    }
}

/// Parses the supported subset; anything else is plain text.
pub fn parse(text: &str) -> Vec<MdBlock> {
    let mut out = Vec::new();
    let mut para: Vec<&str> = Vec::new();
    let mut list: Option<(bool, Vec<Vec<Inline>>)> = None;
    let mut code: Option<Vec<&str>> = None;
    let flush_para = |para: &mut Vec<&str>, out: &mut Vec<MdBlock>| {
        if !para.is_empty() {
            out.push(MdBlock::Paragraph(inline(&para.join(" "))));
            para.clear();
        }
    };
    let flush_list = |list: &mut Option<(bool, Vec<Vec<Inline>>)>, out: &mut Vec<MdBlock>| {
        if let Some((ordered, items)) = list.take() {
            out.push(MdBlock::List { ordered, items });
        }
    };
    for line in text.lines() {
        if let Some(lines) = code.as_mut() {
            if line.trim_start().starts_with("```") {
                out.push(MdBlock::Code(lines.join("\n")));
                code = None;
            } else {
                lines.push(line);
            }
            continue;
        }
        let t = line.trim();
        if t.starts_with("```") {
            flush_para(&mut para, &mut out);
            flush_list(&mut list, &mut out);
            code = Some(Vec::new());
            continue;
        }
        if t.is_empty() {
            flush_para(&mut para, &mut out);
            flush_list(&mut list, &mut out);
            continue;
        }
        if let Some(item) = list_item(t) {
            flush_para(&mut para, &mut out);
            match &mut list {
                Some((ordered, items)) if *ordered == item.0 => items.push(inline(item.1)),
                _ => {
                    flush_list(&mut list, &mut out);
                    list = Some((item.0, vec![inline(item.1)]));
                }
            }
            continue;
        }
        flush_list(&mut list, &mut out);
        if t == "---" || t == "***" {
            flush_para(&mut para, &mut out);
            out.push(MdBlock::Rule);
        } else if let Some((level, rest)) = heading(t) {
            flush_para(&mut para, &mut out);
            out.push(MdBlock::Heading(level, inline(rest)));
        } else if let Some(rest) = t.strip_prefix('>') {
            flush_para(&mut para, &mut out);
            out.push(MdBlock::Quote(inline(rest.trim())));
        } else {
            para.push(t);
        }
    }
    if let Some(lines) = code {
        out.push(MdBlock::Code(lines.join("\n")));
    }
    flush_para(&mut para, &mut out);
    flush_list(&mut list, &mut out);
    out
}

fn heading(t: &str) -> Option<(u8, &str)> {
    let level = t.chars().take_while(|&c| c == '#').count();
    let rest = &t[level..];
    ((1..=3).contains(&level) && rest.starts_with(' ')).then(|| (level as u8, rest.trim()))
}

fn list_item(t: &str) -> Option<(bool, &str)> {
    if let Some(rest) = t.strip_prefix("- ").or_else(|| t.strip_prefix("* ")) {
        return Some((false, rest.trim()));
    }
    let digits = t.chars().take_while(char::is_ascii_digit).count();
    let rest = t[digits..].strip_prefix(". ")?;
    (digits > 0).then(|| (true, rest.trim()))
}

/// Inline styles of one line of text.
pub fn inline(text: &str) -> Vec<Inline> {
    let mut out = Vec::new();
    let mut plain = String::new();
    let mut rest = text;
    while let Some(c) = rest.chars().next() {
        if let Some((node, len)) = styled(rest) {
            if !plain.is_empty() {
                out.push(Inline::Text(std::mem::take(&mut plain)));
            }
            out.push(node);
            rest = &rest[len..];
        } else {
            plain.push(c);
            rest = &rest[c.len_utf8()..];
        }
    }
    if !plain.is_empty() {
        out.push(Inline::Text(plain));
    }
    out
}

/// A styled run at the start of `s`, and how many bytes it spans.
fn styled(s: &str) -> Option<(Inline, usize)> {
    let delimited = |open: &str, close: &str| -> Option<(String, usize)> {
        let body = s.strip_prefix(open)?;
        let end = body.find(close)?;
        let inner = &body[..end];
        let tight = !inner.is_empty() && inner.trim() == inner;
        tight.then(|| (inner.to_string(), open.len() + end + close.len()))
    };
    if let Some((t, n)) = delimited("**", "**") {
        return Some((Inline::Strong(t), n));
    }
    if let Some((t, n)) = delimited("`", "`") {
        return Some((Inline::Code(t), n));
    }
    if let Some((t, n)) = delimited("*", "*").or_else(|| delimited("_", "_")) {
        return Some((Inline::Em(t), n));
    }
    let (label, n) = delimited("[", "]")?;
    let after = &s[n..];
    let target = after.strip_prefix('(')?;
    let end = target.find(')')?;
    let url = target[..end].trim().to_string();
    let lower = url.to_ascii_lowercase();
    let safe = lower.starts_with("https://") || lower.starts_with("http://") || lower.starts_with("mailto:");
    let node = if safe {
        Inline::Link { text: label, url }
    } else {
        Inline::Text(label)
    };
    Some((node, n + end + 2))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks() {
        let md = "# Title\nfirst line\nsecond line\n\n- a\n- **b**\n1. one\n2. two\n> quoted\n---\n```\ncode <x>\n  indented\n```\n## Sub\n####### not a heading\ntext\n```\nunterminated";
        let b = parse(md);
        assert_eq!(b[0], MdBlock::Heading(1, vec![Inline::Text("Title".into())]));
        assert_eq!(b[1], MdBlock::Paragraph(vec![Inline::Text("first line second line".into())]));
        assert_eq!(
            b[2],
            MdBlock::List {
                ordered: false,
                items: vec![vec![Inline::Text("a".into())], vec![Inline::Strong("b".into())]]
            }
        );
        assert!(matches!(&b[3], MdBlock::List { ordered: true, items } if items.len() == 2));
        assert_eq!(b[4], MdBlock::Quote(vec![Inline::Text("quoted".into())]));
        assert_eq!(b[5], MdBlock::Rule);
        assert_eq!(b[6], MdBlock::Code("code <x>\n  indented".into()));
        assert_eq!(b[7], MdBlock::Heading(2, vec![Inline::Text("Sub".into())]));
        assert_eq!(b[8], MdBlock::Paragraph(vec![Inline::Text("####### not a heading text".into())]));
        assert_eq!(b[9], MdBlock::Code("unterminated".into()));
        assert!(parse("").is_empty());
        assert_eq!(parse("- x\n\ny").len(), 2);
        assert_eq!(parse("p\n- x").len(), 2);
        assert_eq!(parse("p\n# h").len(), 2);
        assert_eq!(parse("p\n```\nc\n```").len(), 2);
        assert_eq!(parse("1. a\n- b").len(), 2);
        assert_eq!(parse("- a\ntext").len(), 2);
    }

    #[test]
    fn inlines_and_safe_links() {
        let i = inline("a **b** *c* _d_ `e` [f](https://x.io/?q=1) [g](javascript:alert(1)) [h](mailto:a@b) ** * [x] [y](z");
        assert_eq!(i[0], Inline::Text("a ".into()));
        assert_eq!(i[1], Inline::Strong("b".into()));
        assert_eq!(i[3], Inline::Em("c".into()));
        assert_eq!(i[5], Inline::Em("d".into()));
        assert_eq!(i[7], Inline::Code("e".into()));
        assert_eq!(
            i[9],
            Inline::Link {
                text: "f".into(),
                url: "https://x.io/?q=1".into()
            }
        );
        assert_eq!(i[11], Inline::Text("g".into()));
        assert!(matches!(&i[13], Inline::Link { url, .. } if url == "mailto:a@b"));
        let tail: String = i[14..].iter().map(Inline::text).collect();
        assert_eq!(tail, " ** * [x] [y](z");
        assert_eq!(
            Inline::Link {
                text: "t".into(),
                url: "u".into()
            }
            .text(),
            "t"
        );
        assert_eq!(inline("ünï *ç*")[1], Inline::Em("ç".into()));
    }
}
