//! Self-contained HTML reports: inline CSS and SVG, no scripts, no external
//! resources (a restrictive CSP says so too), every text escaped.

use super::markdown::{self, Inline, MdBlock};
use super::{format_ts, ReportRenderer, ResolvedBlock, ResolvedReport, Theme};
use anyhow::Result;
use std::fmt::Write as _;

/// Escapes text for HTML element content and attribute values.
pub fn escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

pub struct HtmlRenderer;

impl ReportRenderer for HtmlRenderer {
    fn id(&self) -> &'static str {
        "html"
    }

    fn extension(&self) -> &'static str {
        "html"
    }

    fn render(&self, report: &ResolvedReport, theme: Theme, out: &mut dyn std::io::Write) -> Result<()> {
        out.write_all(render(report, theme).as_bytes())?;
        Ok(out.flush()?)
    }
}

const LIGHT: &str = "--bg:#ffffff;--text:#1d2330;--muted:#5b6475;--line:#d9dee7;--panel:#f5f7fa;--accent:#2f6fde;--code:#eef1f6";
const DARK: &str = "--bg:#14171d;--text:#e6e9ef;--muted:#9aa3b2;--line:#2c323d;--panel:#1b2028;--accent:#7aa7ff;--code:#222833";

const CSS: &str = "*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:14px/1.5 -apple-system,'Segoe UI',Roboto,Helvetica,Arial,sans-serif}\
main{max-width:960px;margin:0 auto;padding:32px 24px}h1{font-size:26px;margin:0 0 4px}h2{font-size:20px;margin:28px 0 8px;border-bottom:1px solid var(--line);padding-bottom:4px}\
h3{font-size:16px;margin:16px 0 6px}.meta{color:var(--muted);font-size:12px;margin-bottom:16px}.block{margin:14px 0}.caption{color:var(--muted);font-size:12px;margin:4px 0}\
figure{margin:0;padding:8px;border:1px solid var(--line);border-radius:6px;background:var(--panel)}figure svg{max-width:100%;height:auto;display:block}\
table{border-collapse:collapse;width:100%;font-size:12px}th,td{border:1px solid var(--line);padding:4px 6px;text-align:left;vertical-align:top;overflow-wrap:anywhere}th{background:var(--panel)}\
code,pre{font-family:'JetBrains Mono',Consolas,Menlo,monospace;background:var(--code);border-radius:4px}code{padding:0 3px}pre{padding:8px;overflow-x:auto;white-space:pre-wrap}\
blockquote{margin:6px 0;padding:2px 10px;border-left:3px solid var(--line);color:var(--muted)}a{color:var(--accent)}.lvl{font-weight:600}\
.lvl-Error,.lvl-Fatal,.sev-critical{color:#d6334b}.lvl-Warning,.sev-warning{color:#b7791f}.finding{border:1px solid var(--line);border-radius:6px;padding:8px 10px;background:var(--panel)}\
.redaction{border:1px dashed var(--line);border-radius:6px;padding:6px 10px;font-size:12px;color:var(--muted)}.unsupported{color:var(--muted);font-style:italic}\
@media print{main{max-width:none;padding:0}figure,table,.finding{break-inside:avoid}}";

/// The report as one HTML document.
pub fn render(report: &ResolvedReport, theme: Theme) -> String {
    let vars = match theme {
        Theme::Light => LIGHT,
        Theme::Dark => DARK,
    };
    let mut h = String::new();
    let _ = write!(
        h,
        "<!DOCTYPE html>\n<html lang=\"en\"><head><meta charset=\"utf-8\">\
<meta http-equiv=\"Content-Security-Policy\" content=\"default-src 'none'; style-src 'unsafe-inline'; img-src data:\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\"><meta name=\"generator\" content=\"Debrief\">\
<title>{}</title><style>:root{{{vars}}}{CSS}</style></head><body><main>",
        escape(&report.title)
    );
    let _ = write!(h, "<h1>{}</h1>", escape(&report.title));
    let _ = write!(
        h,
        "<p class=\"meta\">Generated {}</p>",
        escape(&format_ts(Some(report.generated_at)))
    );
    if report.redacted {
        let total: usize = report.redaction.iter().map(|r| r.count).sum();
        let rules: Vec<String> = report
            .redaction
            .iter()
            .filter(|r| r.count > 0)
            .map(|r| format!("{} ({})", escape(&r.rule), r.count))
            .collect();
        let _ = write!(
            h,
            "<p class=\"redaction\">Redacted: {total} value{} masked{}{}.</p>",
            if total == 1 { "" } else { "s" },
            if rules.is_empty() { "" } else { " — " },
            rules.join(", ")
        );
    }
    markdown_html(&mut h, &report.summary);
    for s in &report.sections {
        let _ = write!(h, "<section><h2>{}</h2>", escape(&s.title));
        for b in &s.blocks {
            h.push_str("<div class=\"block\">");
            block_html(&mut h, b);
            h.push_str("</div>");
        }
        h.push_str("</section>");
    }
    h.push_str("</main></body></html>\n");
    h
}

fn caption(h: &mut String, text: &str) {
    if !text.is_empty() {
        let _ = write!(h, "<p class=\"caption\">{}</p>", escape(text));
    }
}

fn block_html(h: &mut String, b: &ResolvedBlock) {
    match b {
        ResolvedBlock::Text { markdown } => markdown_html(h, markdown),
        ResolvedBlock::Events {
            caption: c,
            events,
            missing,
        } => {
            caption(h, c);
            h.push_str("<table><thead><tr><th>Time</th><th>Level</th><th>Source</th><th>Message</th></tr></thead><tbody>");
            for e in events {
                let _ = write!(
                    h,
                    "<tr><td>{}</td><td class=\"lvl lvl-{}\">{}</td><td>{}</td><td>{}",
                    escape(&format_ts(e.ts)),
                    escape(&e.level),
                    escape(&e.level),
                    escape(&e.source),
                    escape(&e.message)
                );
                if let Some(x) = &e.exception {
                    let _ = write!(h, "<pre>{}</pre>", escape(x));
                }
                h.push_str("</td></tr>");
            }
            h.push_str("</tbody></table>");
            if *missing > 0 {
                let _ = write!(h, "<p class=\"caption\">{missing} event(s) are no longer in the workspace.</p>");
            }
        }
        ResolvedBlock::Figure { caption: c, title, svg } => {
            h.push_str("<figure>");
            if !title.is_empty() {
                let _ = write!(h, "<h3>{}</h3>", escape(title));
            }
            match svg {
                // Sanitised by `crate::svg` when the report was resolved.
                Some(svg) => h.push_str(svg),
                None => h.push_str("<p class=\"unsupported\">No snapshot.</p>"),
            }
            h.push_str("</figure>");
            caption(h, c);
        }
        ResolvedBlock::Table { caption: c, columns, rows } => {
            caption(h, c);
            h.push_str("<table><thead><tr>");
            for col in columns {
                let _ = write!(h, "<th>{}</th>", escape(col));
            }
            h.push_str("</tr></thead><tbody>");
            for row in rows {
                h.push_str("<tr>");
                for cell in row {
                    let _ = write!(h, "<td>{}</td>", escape(cell));
                }
                h.push_str("</tr>");
            }
            h.push_str("</tbody></table>");
        }
        ResolvedBlock::Finding { severity, title, detail } => {
            let _ = write!(
                h,
                "<div class=\"finding\"><strong class=\"sev-{}\">{}</strong> {}<p>{}</p></div>",
                escape(severity),
                escape(&severity.to_uppercase()),
                escape(title),
                escape(detail)
            );
        }
        ResolvedBlock::Annotations { caption: c, items } => {
            caption(h, c);
            h.push_str("<ul>");
            for a in items {
                let to = a.to.map(|t| format!(" – {}", format_ts(Some(t)))).unwrap_or_default();
                let kind = if a.kind.is_empty() {
                    String::new()
                } else {
                    format!(" ({})", a.kind)
                };
                let _ = write!(
                    h,
                    "<li>{}{}: {}{}</li>",
                    escape(&format_ts(Some(a.from))),
                    escape(&to),
                    escape(&a.label),
                    escape(&kind)
                );
            }
            h.push_str("</ul>");
        }
        ResolvedBlock::Unsupported { kind } => {
            let _ = write!(
                h,
                "<p class=\"unsupported\">A \"{}\" block this Debrief cannot show.</p>",
                escape(kind)
            );
        }
    }
}

fn markdown_html(h: &mut String, text: &str) {
    for b in markdown::parse(text) {
        match b {
            MdBlock::Heading(level, inl) => {
                let tag = format!("h{}", level + 1);
                let _ = write!(h, "<{tag}>{}</{tag}>", inlines(&inl));
            }
            MdBlock::Paragraph(inl) => {
                let _ = write!(h, "<p>{}</p>", inlines(&inl));
            }
            MdBlock::List { ordered, items } => {
                let tag = if ordered { "ol" } else { "ul" };
                let _ = write!(h, "<{tag}>");
                for item in items {
                    let _ = write!(h, "<li>{}</li>", inlines(&item));
                }
                let _ = write!(h, "</{tag}>");
            }
            MdBlock::Quote(inl) => {
                let _ = write!(h, "<blockquote>{}</blockquote>", inlines(&inl));
            }
            MdBlock::Code(code) => {
                let _ = write!(h, "<pre><code>{}</code></pre>", escape(&code));
            }
            MdBlock::Rule => h.push_str("<hr>"),
        }
    }
}

fn inlines(items: &[Inline]) -> String {
    items
        .iter()
        .map(|i| match i {
            Inline::Text(t) => escape(t),
            Inline::Strong(t) => format!("<strong>{}</strong>", escape(t)),
            Inline::Em(t) => format!("<em>{}</em>", escape(t)),
            Inline::Code(t) => format!("<code>{}</code>", escape(t)),
            Inline::Link { text, url } => format!("<a href=\"{}\" rel=\"noopener noreferrer\">{}</a>", escape(url), escape(text)),
        })
        .collect()
}
