//! Lays a resolved report out on A4 pages: flowing text with line breaking,
//! tables, event lists and embedded SVG figures.

use crate::fonts::{compress, Font, Subsets};
use anyhow::{anyhow, Result};
use imt_core::report::markdown::{self, Inline, MdBlock};
use imt_core::report::{format_ts, ResolvedBlock, ResolvedReport, Theme};
use pdf_writer::{Chunk, Content, Filter, Finish, Name, Pdf, Rect, Ref, Str, TextStr};
use std::collections::HashMap;
use std::sync::{Arc, OnceLock};

const PAGE_W: f32 = 595.28;
const PAGE_H: f32 = 841.89;
const MARGIN: f32 = 50.0;
const WIDTH: f32 = PAGE_W - 2.0 * MARGIN;
/// Most lines of an exception printed under an event.
const MAX_EXCEPTION_LINES: usize = 40;
/// Most lines of one table cell.
const MAX_CELL_LINES: usize = 6;

type Rgb = [f32; 3];

struct Colors {
    background: Option<Rgb>,
    text: Rgb,
    muted: Rgb,
    line: Rgb,
    panel: Rgb,
    accent: Rgb,
    error: Rgb,
    warning: Rgb,
}

fn colors(theme: Theme) -> Colors {
    match theme {
        Theme::Light => Colors {
            background: None,
            text: [0.11, 0.14, 0.19],
            muted: [0.36, 0.39, 0.46],
            line: [0.85, 0.87, 0.91],
            panel: [0.96, 0.97, 0.98],
            accent: [0.18, 0.44, 0.87],
            error: [0.84, 0.2, 0.29],
            warning: [0.72, 0.47, 0.12],
        },
        Theme::Dark => Colors {
            background: Some([0.08, 0.09, 0.11]),
            text: [0.9, 0.91, 0.94],
            muted: [0.6, 0.64, 0.7],
            line: [0.17, 0.2, 0.24],
            panel: [0.11, 0.13, 0.16],
            accent: [0.48, 0.65, 1.0],
            error: [1.0, 0.45, 0.52],
            warning: [0.95, 0.72, 0.35],
        },
    }
}

/// A run of text in one font and colour.
#[derive(Clone)]
struct Run {
    text: String,
    font: Font,
    color: Rgb,
}

struct Page {
    content: Content,
    xobjects: Vec<(String, Ref)>,
}

struct Doc {
    colors: Colors,
    pages: Vec<Page>,
    y: f32,
    alloc: Ref,
    chunks: Vec<Chunk>,
    figures: usize,
    /// The glyphs printed so far, embedded as font subsets at the end.
    subsets: Subsets,
}

/// The system fonts for the text in SVG figures, loaded once.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut db = usvg::fontdb::Database::new();
            db.load_system_fonts();
            Arc::new(db)
        })
        .clone()
}

impl Doc {
    fn new(theme: Theme) -> Doc {
        let mut d = Doc {
            colors: colors(theme),
            pages: Vec::new(),
            y: 0.0,
            // 1 catalog, 2 page tree, 3 info.
            alloc: Ref::new(4),
            chunks: Vec::new(),
            figures: 0,
            subsets: Subsets::default(),
        };
        d.new_page();
        d
    }

    fn page(&mut self) -> &mut Page {
        if self.pages.is_empty() {
            self.new_page();
        }
        let last = self.pages.len() - 1;
        &mut self.pages[last]
    }

    fn new_page(&mut self) {
        let mut content = Content::new();
        if let Some([r, g, b]) = self.colors.background {
            content.set_fill_rgb(r, g, b).rect(0.0, 0.0, PAGE_W, PAGE_H).fill_nonzero();
        }
        self.pages.push(Page {
            content,
            xobjects: Vec::new(),
        });
        self.y = PAGE_H - MARGIN;
    }

    /// Starts a new page unless `h` points fit above the bottom margin.
    fn ensure(&mut self, h: f32) {
        if self.y - h < MARGIN && self.y < PAGE_H - MARGIN {
            self.new_page();
        }
    }

    fn space(&mut self, h: f32) {
        self.y -= h;
    }

    fn draw(&mut self, x: f32, y: f32, run: &Run, size: f32) {
        let [r, g, b] = run.color;
        let cids = self.subsets.encode(run.font, &run.text);
        self.page()
            .content
            .begin_text()
            .set_font(Name(run.font.resource()), size)
            .set_fill_rgb(r, g, b)
            .next_line(x, y)
            .show(Str(&cids))
            .end_text();
    }

    fn rule(&mut self, color: Rgb) {
        let y = self.y;
        let [r, g, b] = color;
        self.page()
            .content
            .set_stroke_rgb(r, g, b)
            .set_line_width(0.6)
            .move_to(MARGIN, y)
            .line_to(PAGE_W - MARGIN, y)
            .stroke();
    }

    /// Flowing text: wraps `runs` into `width` and draws them at `x`.
    fn flow(&mut self, runs: &[Run], size: f32, x: f32, width: f32) {
        let leading = size * 1.35;
        for line in wrap(runs, size, width) {
            self.ensure(leading);
            self.y -= leading;
            let mut cx = x;
            let y = self.y + size * 0.3;
            for run in &line {
                self.draw(cx, y, run, size);
                cx += run.font.width(&run.text, size);
            }
        }
    }

    fn text(&mut self, text: &str, font: Font, size: f32, color: Rgb, indent: f32) {
        let run = Run {
            text: text.to_string(),
            font,
            color,
        };
        self.flow(&[run], size, MARGIN + indent, WIDTH - indent);
    }

    fn markdown(&mut self, text: &str) {
        let c = self.colors.text;
        for block in markdown::parse(text) {
            match block {
                MdBlock::Heading(level, inl) => {
                    self.space(6.0);
                    let size = [15.0, 13.0, 12.0][usize::from(level.clamp(1, 3)) - 1];
                    let runs: Vec<Run> = inl.iter().map(|i| self.run(i, Font::Bold)).collect();
                    self.flow(&runs, size, MARGIN, WIDTH);
                }
                MdBlock::Paragraph(inl) => {
                    let runs: Vec<Run> = inl.iter().map(|i| self.run(i, Font::Regular)).collect();
                    self.flow(&runs, 10.5, MARGIN, WIDTH);
                    self.space(4.0);
                }
                MdBlock::List { ordered, items } => {
                    for (n, item) in items.iter().enumerate() {
                        let bullet = if ordered { format!("{}.", n + 1) } else { "•".into() };
                        let mut runs = vec![Run {
                            text: format!("{bullet} "),
                            font: Font::Regular,
                            color: c,
                        }];
                        runs.extend(item.iter().map(|i| self.run(i, Font::Regular)));
                        self.flow(&runs, 10.5, MARGIN + 12.0, WIDTH - 12.0);
                    }
                    self.space(4.0);
                }
                MdBlock::Quote(inl) => {
                    let muted = self.colors.muted;
                    let runs: Vec<Run> = inl
                        .iter()
                        .map(|i| Run {
                            text: i.text().into(),
                            font: Font::Italic,
                            color: muted,
                        })
                        .collect();
                    self.flow(&runs, 10.5, MARGIN + 14.0, WIDTH - 14.0);
                    self.space(4.0);
                }
                MdBlock::Code(code) => {
                    for line in code.lines() {
                        self.text(line, Font::Mono, 8.5, c, 8.0);
                    }
                    self.space(4.0);
                }
                MdBlock::Rule => {
                    self.space(6.0);
                    self.rule(self.colors.line);
                    self.space(6.0);
                }
            }
        }
    }

    fn run(&self, i: &Inline, plain: Font) -> Run {
        let c = self.colors.text;
        match i {
            Inline::Text(t) => Run {
                text: t.clone(),
                font: plain,
                color: c,
            },
            Inline::Strong(t) => Run {
                text: t.clone(),
                font: Font::Bold,
                color: c,
            },
            Inline::Em(t) => Run {
                text: t.clone(),
                font: Font::Italic,
                color: c,
            },
            Inline::Code(t) => Run {
                text: t.clone(),
                font: Font::Mono,
                color: c,
            },
            Inline::Link { text, url } => Run {
                text: format!("{text} ({url})"),
                font: plain,
                color: self.colors.accent,
            },
        }
    }

    fn caption(&mut self, text: &str) {
        if !text.is_empty() {
            self.text(text, Font::Italic, 8.5, self.colors.muted, 0.0);
        }
    }

    fn level_color(&self, level: &str) -> Rgb {
        match level {
            "Error" | "Fatal" | "critical" => self.colors.error,
            "Warning" | "warning" => self.colors.warning,
            _ => self.colors.muted,
        }
    }

    fn block(&mut self, b: &ResolvedBlock) {
        match b {
            ResolvedBlock::Text { markdown } => self.markdown(markdown),
            ResolvedBlock::Events { caption, events, missing } => {
                self.caption(caption);
                for e in events {
                    self.space(3.0);
                    let header = [
                        Run {
                            text: format!("{}  ", format_ts(e.ts)),
                            font: Font::Mono,
                            color: self.colors.muted,
                        },
                        Run {
                            text: format!("{}  ", e.level),
                            font: Font::Bold,
                            color: self.level_color(&e.level),
                        },
                        Run {
                            text: e.source.clone(),
                            font: Font::Regular,
                            color: self.colors.muted,
                        },
                    ];
                    self.flow(&header, 8.0, MARGIN, WIDTH);
                    self.text(&e.message, Font::Regular, 9.5, self.colors.text, 0.0);
                    if let Some(x) = &e.exception {
                        let lines: Vec<&str> = x.lines().collect();
                        for line in lines.iter().take(MAX_EXCEPTION_LINES) {
                            self.text(line, Font::Mono, 7.5, self.colors.muted, 8.0);
                        }
                        if lines.len() > MAX_EXCEPTION_LINES {
                            self.text(
                                &format!("… {} more lines", lines.len() - MAX_EXCEPTION_LINES),
                                Font::Italic,
                                7.5,
                                self.colors.muted,
                                8.0,
                            );
                        }
                    }
                }
                if *missing > 0 {
                    self.caption(&format!("{missing} event(s) are no longer in the workspace."));
                }
            }
            ResolvedBlock::Figure { caption, title, svg } => {
                if !title.is_empty() {
                    self.text(title, Font::Bold, 11.0, self.colors.text, 0.0);
                    self.space(2.0);
                }
                match svg.as_deref().map(|s| self.figure(s)) {
                    Some(Ok(())) => {}
                    Some(Err(_)) => self.caption("(the snapshot could not be drawn)"),
                    None => self.caption("(no snapshot)"),
                }
                self.caption(caption);
            }
            ResolvedBlock::Table { caption, columns, rows } => {
                self.caption(caption);
                self.table(columns, rows);
            }
            ResolvedBlock::Finding { severity, title, detail } => {
                let header = [
                    Run {
                        text: format!("{} ", severity.to_uppercase()),
                        font: Font::Bold,
                        color: self.level_color(severity),
                    },
                    Run {
                        text: title.clone(),
                        font: Font::Bold,
                        color: self.colors.text,
                    },
                ];
                self.flow(&header, 10.5, MARGIN, WIDTH);
                self.text(detail, Font::Regular, 9.5, self.colors.text, 0.0);
            }
            ResolvedBlock::Annotations { caption, items } => {
                self.caption(caption);
                for a in items {
                    let to = a.to.map(|t| format!(" – {}", format_ts(Some(t)))).unwrap_or_default();
                    let kind = if a.kind.is_empty() {
                        String::new()
                    } else {
                        format!(" ({})", a.kind)
                    };
                    let text = format!("• {}{to}: {}{kind}", format_ts(Some(a.from)), a.label);
                    self.text(&text, Font::Regular, 9.5, self.colors.text, 6.0);
                }
            }
            ResolvedBlock::Unsupported { kind } => self.text(
                &format!("A \"{kind}\" block this IMT cannot show."),
                Font::Italic,
                9.5,
                self.colors.muted,
                0.0,
            ),
        }
    }

    fn table(&mut self, columns: &[String], rows: &[Vec<String>]) {
        let n = columns.len().max(1);
        let col = WIDTH / n as f32;
        let size = 8.0;
        let leading = size * 1.3;
        let cells = |row: &[String], font: Font| -> Vec<Vec<String>> {
            (0..n)
                .map(|i| {
                    let text = row.get(i).map(String::as_str).unwrap_or("");
                    let run = Run {
                        text: text.into(),
                        font,
                        color: [0.0; 3],
                    };
                    let mut lines: Vec<String> = wrap(&[run], size, col - 6.0)
                        .into_iter()
                        .map(|l| l.iter().map(|r| r.text.as_str()).collect())
                        .collect();
                    if lines.len() > MAX_CELL_LINES {
                        lines.truncate(MAX_CELL_LINES);
                        lines[MAX_CELL_LINES - 1].push('…');
                    }
                    lines
                })
                .collect()
        };
        let draw_row = |doc: &mut Doc, cells: Vec<Vec<String>>, font: Font, fill: Option<Rgb>| {
            let h = cells.iter().map(Vec::len).max().unwrap_or(1).max(1) as f32 * leading + 4.0;
            doc.ensure(h);
            let top = doc.y;
            let [lr, lg, lb] = doc.colors.line;
            let page = doc.page();
            if let Some([r, g, b]) = fill {
                page.content.set_fill_rgb(r, g, b).rect(MARGIN, top - h, WIDTH, h).fill_nonzero();
            }
            page.content
                .set_stroke_rgb(lr, lg, lb)
                .set_line_width(0.5)
                .rect(MARGIN, top - h, WIDTH, h)
                .stroke();
            let color = doc.colors.text;
            for (i, lines) in cells.iter().enumerate() {
                for (j, line) in lines.iter().enumerate() {
                    let run = Run {
                        text: line.clone(),
                        font,
                        color,
                    };
                    doc.draw(
                        MARGIN + i as f32 * col + 3.0,
                        top - 2.0 - (j as f32 + 1.0) * leading + size * 0.3,
                        &run,
                        size,
                    );
                }
            }
            doc.y -= h;
        };
        let header = cells(columns, Font::Bold);
        let panel = self.colors.panel;
        draw_row(self, header, Font::Bold, Some(panel));
        for row in rows {
            let c = cells(row, Font::Regular);
            draw_row(self, c, Font::Regular, None);
        }
    }

    /// Embeds a (sanitised) SVG as a vector XObject scaled to the text width.
    fn figure(&mut self, svg: &str) -> Result<()> {
        let options = usvg::Options {
            fontdb: fonts(),
            ..Default::default()
        };
        let tree = usvg::Tree::from_str(svg, &options).map_err(|e| anyhow!("{e}"))?;
        let (chunk, id) = svg2pdf::to_chunk(&tree, svg2pdf::ConversionOptions::default()).map_err(|e| anyhow!("{e:?}"))?;
        let mut map = HashMap::new();
        let alloc = &mut self.alloc;
        let chunk = chunk.renumber(|old| *map.entry(old).or_insert_with(|| alloc.bump()));
        let id = *map.get(&id).ok_or_else(|| anyhow!("the figure has no root object"))?;
        self.chunks.push(chunk);
        let (w, h) = (tree.size().width().max(1.0), tree.size().height().max(1.0));
        let mut scale = WIDTH / w;
        let max_h = PAGE_H - 2.0 * MARGIN - 40.0;
        if h * scale > max_h {
            scale = max_h / h;
        }
        let (tw, th) = (w * scale, h * scale);
        self.ensure(th + 4.0);
        self.figures += 1;
        let name = format!("X{}", self.figures);
        let y = self.y - th;
        let page = self.page();
        page.content
            .save_state()
            .transform([tw, 0.0, 0.0, th, MARGIN, y])
            .x_object(Name(name.as_bytes()))
            .restore_state();
        page.xobjects.push((name, id));
        self.y -= th + 4.0;
        Ok(())
    }
}

/// Breaks runs into lines of at most `width` points (words longer than a
/// line are broken by character).
fn wrap(runs: &[Run], size: f32, width: f32) -> Vec<Vec<Run>> {
    let mut lines: Vec<Vec<Run>> = Vec::new();
    for text_line in split_lines(runs) {
        let mut line: Vec<Run> = Vec::new();
        let mut used = 0.0;
        for run in text_line {
            for word in run.text.split_inclusive(' ') {
                let w = run.font.width(word, size);
                let fit = run.font.width(word.trim_end(), size);
                if used + fit > width && !line.is_empty() {
                    lines.push(std::mem::take(&mut line));
                    used = 0.0;
                }
                if fit > width {
                    // Hard-break a word wider than the line.
                    let mut piece = String::new();
                    for c in word.chars() {
                        let cw = run.font.width(&c.to_string(), size);
                        if used + run.font.width(&piece, size) + cw > width && !piece.is_empty() {
                            push(&mut line, &run, std::mem::take(&mut piece));
                            lines.push(std::mem::take(&mut line));
                            used = 0.0;
                        }
                        piece.push(c);
                    }
                    used += run.font.width(&piece, size);
                    push(&mut line, &run, piece);
                    continue;
                }
                push(&mut line, &run, word.to_string());
                used += w;
            }
        }
        lines.push(line);
    }
    lines
}

/// Runs split at newlines into lines of runs.
fn split_lines(runs: &[Run]) -> Vec<Vec<Run>> {
    let mut out = vec![Vec::new()];
    for run in runs {
        for (i, part) in run.text.split('\n').enumerate() {
            if i > 0 {
                out.push(Vec::new());
            }
            if !part.is_empty() {
                out.last_mut().unwrap_or_else(|| unreachable!("never empty")).push(Run {
                    text: part.trim_end_matches('\r').to_string(),
                    ..run.clone()
                });
            }
        }
    }
    out
}

/// Appends text to a line, merging with the last run when the style matches.
fn push(line: &mut Vec<Run>, style: &Run, text: String) {
    match line.last_mut() {
        Some(last) if last.font == style.font && last.color == style.color => last.text.push_str(&text),
        _ => line.push(Run { text, ..style.clone() }),
    }
}

/// The report as a PDF document.
pub fn render_pdf(report: &ResolvedReport, theme: Theme) -> Result<Vec<u8>> {
    let mut doc = Doc::new(theme);
    let c = doc.colors.text;
    doc.text(&report.title, Font::Bold, 20.0, c, 0.0);
    doc.text(
        &format!("Generated {}", format_ts(Some(report.generated_at))),
        Font::Regular,
        9.0,
        doc.colors.muted,
        0.0,
    );
    if report.redacted {
        let total: usize = report.redaction.iter().map(|r| r.count).sum();
        let rules: Vec<String> = report
            .redaction
            .iter()
            .filter(|r| r.count > 0)
            .map(|r| format!("{} ({})", r.rule, r.count))
            .collect();
        let note = format!(
            "Redacted: {total} value{} masked{}{}.",
            if total == 1 { "" } else { "s" },
            if rules.is_empty() { "" } else { ": " },
            rules.join(", ")
        );
        doc.text(&note, Font::Italic, 9.0, doc.colors.muted, 0.0);
    }
    doc.space(8.0);
    doc.markdown(&report.summary);
    for s in &report.sections {
        doc.ensure(60.0);
        doc.space(10.0);
        doc.text(&s.title, Font::Bold, 16.0, c, 0.0);
        doc.space(3.0);
        doc.rule(doc.colors.line);
        doc.space(6.0);
        for b in &s.blocks {
            doc.block(b);
            doc.space(8.0);
        }
    }
    finish(doc, &report.title)
}

fn finish(mut doc: Doc, title: &str) -> Result<Vec<u8>> {
    let catalog = Ref::new(1);
    let tree = Ref::new(2);
    let info = Ref::new(3);
    let mut pdf = Pdf::new();
    let total = doc.pages.len();
    let [r, g, b] = doc.colors.muted;
    let mut pages = std::mem::take(&mut doc.pages);
    for (i, page) in pages.iter_mut().enumerate() {
        let footer = format!("IMT incident report - page {} of {total}", i + 1);
        let cids = doc.subsets.encode(Font::Regular, &footer);
        page.content
            .begin_text()
            .set_font(Name(Font::Regular.resource()), 7.5)
            .set_fill_rgb(r, g, b)
            .next_line(MARGIN, MARGIN / 2.0)
            .show(Str(&cids))
            .end_text();
    }
    let mut alloc = doc.alloc;
    let font_refs = doc.subsets.write(&mut pdf, &mut alloc)?;
    let mut page_refs = Vec::with_capacity(total);
    for page in pages {
        let page_ref = alloc.bump();
        let content_ref = alloc.bump();
        page_refs.push(page_ref);
        let mut p = pdf.page(page_ref);
        p.media_box(Rect::new(0.0, 0.0, PAGE_W, PAGE_H)).parent(tree).contents(content_ref);
        let mut res = p.resources();
        let mut fonts = res.fonts();
        for (f, r) in &font_refs {
            fonts.pair(Name(f.resource()), *r);
        }
        fonts.finish();
        let mut xo = res.x_objects();
        for (name, r) in &page.xobjects {
            xo.pair(Name(name.as_bytes()), *r);
        }
        xo.finish();
        res.finish();
        p.finish();
        pdf.stream(content_ref, &compress(&page.content.finish()))
            .filter(Filter::FlateDecode);
    }
    pdf.catalog(catalog).pages(tree);
    pdf.pages(tree).kids(page_refs).count(total as i32);
    pdf.document_info(info).title(TextStr(title)).producer(TextStr("IMT"));
    for chunk in &doc.chunks {
        pdf.extend(chunk);
    }
    Ok(pdf.finish())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn runs(t: &str) -> Vec<Run> {
        vec![Run {
            text: t.into(),
            font: Font::Regular,
            color: [0.0; 3],
        }]
    }

    #[test]
    fn wraps_words_lines_and_long_words() {
        let lines = wrap(&runs("aaa bbb ccc"), 10.0, Font::Regular.width("aaa bbb", 10.0) + 1.0);
        let text: Vec<String> = lines.iter().map(|l| l.iter().map(|r| r.text.as_str()).collect()).collect();
        assert_eq!(text, ["aaa bbb ", "ccc"]);
        let lines = wrap(&runs("x\ny\r\n"), 10.0, 500.0);
        assert_eq!(lines.len(), 3);
        let long = "a".repeat(200);
        let lines = wrap(&runs(&long), 10.0, 100.0);
        assert!(lines.len() > 1);
        assert!(lines
            .iter()
            .all(|l| l.iter().map(|r| r.font.width(&r.text, 10.0)).sum::<f32>() <= 100.0 + 0.01));
        let mut mixed = runs("plain ");
        mixed.push(Run {
            text: "bold".into(),
            font: Font::Bold,
            color: [0.0; 3],
        });
        assert_eq!(wrap(&mixed, 10.0, 500.0)[0].len(), 2);
    }
}
