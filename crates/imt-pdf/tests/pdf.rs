//! PDF smoke tests: a valid document whose text can be extracted.

use imt_core::redact::RuleHits;
use imt_core::report::{AnnotationItem, EventLine, ReportRenderer, ResolvedBlock, ResolvedReport, ResolvedSection, Theme};
use imt_pdf::{render_pdf, PdfRenderer};
use serde_json::json;
use std::collections::HashMap;

fn report() -> ResolvedReport {
    let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="400" height="120" viewBox="0 0 400 120"><rect width="400" height="120" fill="#eee"/><path d="M0 100 L100 40 L200 80 L400 10" stroke="red" fill="none"/><text x="10" y="20" font-size="12" font-family="sans-serif">Errors per minute</text></svg>"##;
    let long = "word ".repeat(400);
    ResolvedReport {
        title: "Checkout outage (14 Oct)".into(),
        summary: "# Summary\nPayments **failed** for *20 minutes*; see `PaymentGateway` and [status](https://status.example).\n\n- first\n- second\n1. one\n> quoted\n```\ncode line\n```\n---".into(),
        generated_at: 1_791_972_000_000,
        redacted: true,
        redaction: vec![RuleHits { rule: "email".into(), count: 2 }, RuleHits { rule: "ipv4".into(), count: 0 }],
        sections: vec![
            ResolvedSection {
                title: "Timeline".into(),
                blocks: vec![
                    ResolvedBlock::Events {
                        caption: "Bookmarked events".into(),
                        events: vec![
                            EventLine { id: 1, ts: Some(1_791_972_000_123), level: "Error".into(), source: "web01/app.log".into(), message: "Order 88 failed for <email>".into(), exception: Some(format!("System.Exception: boom\n{}", "   at X()\n".repeat(60))) },
                            EventLine { id: 2, ts: None, level: "Warning".into(), source: "s".into(), message: "Ünïcödé ✓ 日本".into(), exception: None },
                        ],
                        missing: 1,
                    },
                    ResolvedBlock::Figure { caption: "Errors".into(), title: "Error rate".into(), svg: Some(svg.into()) },
                    ResolvedBlock::Figure { caption: String::new(), title: String::new(), svg: Some("<svg><broken".into()) },
                    ResolvedBlock::Figure { caption: String::new(), title: "Tall".into(), svg: Some(r#"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="5000"><rect width="10" height="5000"/></svg>"#.into()) },
                    ResolvedBlock::Figure { caption: String::new(), title: String::new(), svg: None },
                ],
            },
            ResolvedSection {
                title: "Analysis".into(),
                blocks: vec![
                    ResolvedBlock::Table {
                        caption: "Top sources".into(),
                        columns: vec!["SourceContext".into(), "count()".into()],
                        rows: (0..80).map(|i| vec![format!("Orders.Api.Controller{i}"), i.to_string()]).chain([vec![long.clone()]]).collect(),
                    },
                    ResolvedBlock::Finding { severity: "critical".into(), title: "Error spike".into(), detail: "12x the usual rate".into() },
                    ResolvedBlock::Annotations { caption: "Reboots".into(), items: vec![AnnotationItem { from: 1, to: Some(2), label: "reboot".into(), kind: "windows.reboots".into() }, AnnotationItem { from: 3, to: None, label: "x".into(), kind: String::new() }] },
                    ResolvedBlock::Text { markdown: long },
                    ResolvedBlock::Unsupported { kind: "future".into() },
                ],
            },
        ],
    }
}

fn find(h: &[u8], n: &[u8]) -> Option<usize> {
    h.windows(n.len()).position(|w| w == n)
}

/// The objects of an (uncompressed-xref) PDF by number.
fn objects(pdf: &[u8]) -> HashMap<u32, &[u8]> {
    let mut out = HashMap::new();
    let mut rest = pdf;
    while let Some(i) = find(rest, b" 0 obj") {
        let num_start = rest[..i].iter().rposition(|b| !b.is_ascii_digit()).map_or(0, |p| p + 1);
        let num: u32 = std::str::from_utf8(&rest[num_start..i]).unwrap().parse().unwrap();
        let body = &rest[i + 6..];
        let end = find(body, b"endobj").unwrap();
        out.insert(num, &body[..end]);
        rest = &body[end..];
    }
    out
}

/// The inflated stream of an object.
fn stream(obj: &[u8]) -> Vec<u8> {
    let start = find(obj, b"stream\n").unwrap() + 7;
    let end = find(obj, b"endstream").unwrap();
    let data = obj[start..end].strip_suffix(b"\n").unwrap_or(&obj[start..end]);
    miniz_oxide::inflate::decompress_to_vec_zlib(data).unwrap()
}

/// Every Flate stream of the document, inflated and lossily decoded.
fn inflated(pdf: &[u8]) -> String {
    let streams = objects(pdf).into_values().filter(|o| find(o, b"/Filter /FlateDecode").is_some());
    streams.map(|o| String::from_utf8_lossy(&stream(o)).into_owned()).collect()
}

/// The object number after `key` (`/Key N 0 R`).
fn reference(obj: &[u8], key: &[u8]) -> Option<u32> {
    let at = find(obj, key)? + key.len();
    let digits: String = obj[at..]
        .iter()
        .skip_while(|b| b.is_ascii_whitespace())
        .take_while(|b| b.is_ascii_digit())
        .map(|&b| char::from(b))
        .collect();
    digits.parse().ok()
}

/// A ToUnicode CMap: CID to text (bfchar entries).
fn cmap(data: &[u8]) -> HashMap<u16, String> {
    let text = String::from_utf8_lossy(data);
    let mut out = HashMap::new();
    for section in text.split("beginbfchar").skip(1) {
        let section = section.split("endbfchar").next().unwrap();
        let hex: Vec<&str> = section.split(['<', '>']).map(str::trim).filter(|s| !s.is_empty()).collect();
        for pair in hex.chunks(2) {
            let cid = u16::from_str_radix(pair[0], 16).unwrap();
            let units: Vec<u16> = (0..pair[1].len() / 4)
                .map(|i| u16::from_str_radix(&pair[1][i * 4..i * 4 + 4], 16).unwrap())
                .collect();
            out.insert(cid, String::from_utf16(&units).unwrap());
        }
    }
    out
}

/// The bytes of a PDF string token at the start of `s` (literal or hex) and its length.
fn pdf_string(s: &[u8]) -> (Vec<u8>, usize) {
    let mut out = Vec::new();
    if s[0] == b'<' {
        let end = s.iter().position(|&b| b == b'>').unwrap();
        let hex: Vec<u8> = s[1..end].iter().copied().filter(u8::is_ascii_hexdigit).collect();
        for pair in hex.chunks(2) {
            out.push(u8::from_str_radix(std::str::from_utf8(pair).unwrap(), 16).unwrap());
        }
        return (out, end + 1);
    }
    let (mut i, mut depth) = (1, 1);
    while i < s.len() {
        match s[i] {
            b'\\' => {
                i += 1;
                match s[i] {
                    b'n' => out.push(b'\n'),
                    b'r' => out.push(b'\r'),
                    b't' => out.push(b'\t'),
                    b'b' => out.push(8),
                    b'f' => out.push(12),
                    b'0'..=b'7' => {
                        let n = s[i..].iter().take(3).take_while(|b| (b'0'..=b'7').contains(b)).count();
                        out.push(u8::from_str_radix(std::str::from_utf8(&s[i..i + n]).unwrap(), 8).unwrap());
                        i += n - 1;
                    }
                    other => out.push(other),
                }
            }
            b'(' => {
                depth += 1;
                out.push(b'(');
            }
            b')' => {
                depth -= 1;
                if depth == 0 {
                    return (out, i + 1);
                }
                out.push(b')');
            }
            b => out.push(b),
        }
        i += 1;
    }
    (out, i)
}

/// The text a PDF reader extracts from the pages, through each report font's
/// ToUnicode CMap; one line per text-showing operation.
fn text_of(pdf: &[u8]) -> String {
    let objs = objects(pdf);
    let mut out = String::new();
    let mut pages: Vec<u32> = objs
        .iter()
        .filter(|(_, o)| find(o, b"/Type /Page\n").is_some() || find(o, b"/Type /Page ").is_some())
        .map(|(n, _)| *n)
        .collect();
    pages.sort_unstable();
    for page in pages {
        let o = objs[&page];
        let fonts_at = find(o, b"/Font <<").unwrap();
        let dict = &o[fonts_at..fonts_at + find(&o[fonts_at..], b">>").unwrap()];
        let mut maps = HashMap::new();
        for f in [b"/F1".as_slice(), b"/F2", b"/F3", b"/F4"] {
            if let Some(type0) = reference(dict, f) {
                maps.insert(
                    f[1..].to_vec(),
                    cmap(&stream(objs[&reference(objs[&type0], b"/ToUnicode").unwrap()])),
                );
            }
        }
        let content = stream(objs[&reference(o, b"/Contents").unwrap()]);
        let mut font: Vec<u8> = Vec::new();
        let mut i = 0;
        while i < content.len() {
            if content[i] == b'/' {
                let name: Vec<u8> = content[i + 1..].iter().take_while(|b| b.is_ascii_alphanumeric()).copied().collect();
                i += name.len() + 1;
                font = name;
            } else if content[i] == b'(' || (content[i] == b'<' && content.get(i + 1) != Some(&b'<')) {
                let (bytes, len) = pdf_string(&content[i..]);
                i += len;
                let map = &maps[&font];
                for cid in bytes.chunks(2) {
                    out.push_str(&map[&u16::from_be_bytes([cid[0], cid[1]])]);
                }
                out.push('\n');
            } else {
                i += 1;
            }
        }
    }
    out
}

#[test]
fn writes_a_valid_pdf_with_extractable_text_and_vector_figures() {
    for theme in [Theme::Light, Theme::Dark] {
        let pdf = render_pdf(&report(), theme).unwrap();
        assert!(pdf.starts_with(b"%PDF-"));
        assert!(pdf.ends_with(b"%%EOF") || pdf.ends_with(b"%%EOF\n"));
        let text = text_of(&pdf);
        for needle in [
            "Checkout outage (14 Oct)",
            "Order 88 failed for <email>",
            "Errors",
            "SourceContext",
            "CRITICAL \nError spike\n",
            "Redacted: 2 values masked",
            "page 1 of",
            // Unicode beyond Windows-1252 and the documented fallback for CJK.
            "Ünïcödé ✓ \u{FFFD}\u{FFFD}",
        ] {
            assert!(text.contains(needle), "{needle} missing");
        }
        // Several pages, embedded subset fonts (no standard fonts) and the figure.
        assert!(text.contains("page 2 of"));
        for needle in [
            b"/Subtype /Type0".as_slice(),
            b"/CIDFontType2",
            b"/Identity-H",
            b"/ToUnicode",
            b"/FontFile2",
            b"+IMTReportSans-Bold",
            b"+IMTReportMono-Regular",
        ] {
            assert!(find(&pdf, needle).is_some(), "{}", String::from_utf8_lossy(needle));
        }
        assert!(find(&pdf, b"/Helvetica").is_none() && find(&pdf, b"/WinAnsiEncoding").is_none());
        assert!(find(&pdf, b"/XObject").is_some() || find(&pdf, b"/Subtype /Form").is_some());
        let content = inflated(&pdf);
        assert!(content.contains("/X1 Do"));
    }
}

#[test]
fn the_renderer_writes_to_any_output() {
    let mut out = Vec::new();
    PdfRenderer.render(&report(), Theme::Light, &mut out).unwrap();
    assert!(out.starts_with(b"%PDF-"));
    assert_eq!((PdfRenderer.id(), PdfRenderer.extension()), ("pdf", "pdf"));
    let empty = ResolvedReport {
        title: String::new(),
        summary: String::new(),
        generated_at: 0,
        redacted: false,
        redaction: vec![],
        sections: vec![],
    };
    assert!(render_pdf(&empty, Theme::Light).unwrap().starts_with(b"%PDF-"));
    let _ = json!(null);
}

#[test]
#[ignore]
fn dump_for_review() {
    let mut r = report();
    r.sections.remove(0);
    r.sections[0].blocks.insert(0, report().sections[0].blocks[1].clone());
    r.summary.clear();
    std::fs::write("/private/tmp/claude-501/dump.pdf", render_pdf(&r, Theme::Dark).unwrap()).unwrap();
}

fn text_report(message: &str) -> ResolvedReport {
    ResolvedReport {
        title: "Ελληνικά".into(),
        summary: String::new(),
        generated_at: 0,
        redacted: false,
        redaction: vec![],
        sections: vec![ResolvedSection {
            title: "S".into(),
            blocks: vec![ResolvedBlock::Text { markdown: message.into() }],
        }],
    }
}

#[test]
fn greek_and_cyrillic_render_through_the_embedded_font() {
    let pdf = render_pdf(&text_report("Σφάλμα σύνδεσης: Ошибка `κώδικας`"), Theme::Light).unwrap();
    let text = text_of(&pdf);
    assert!(text.contains("Ελληνικά"), "{text}");
    assert!(text.contains("Σφάλμα σύνδεσης: Ошибка "), "{text}");
    assert!(text.contains("κώδικας"), "{text}");
    // The ToUnicode CMaps hold the code points (UTF-16BE): Σ U+03A3, Ж-family О U+041E.
    let cmaps = inflated(&pdf);
    assert!(cmaps.contains("<03A3>") && cmaps.contains("<041E>"), "{cmaps}");
    // No replacement glyph was needed.
    assert!(!text.contains('\u{FFFD}'));
    // Subsets: far smaller than the ~250 KB of bundled fonts.
    assert!(pdf.len() < 60_000, "{}", pdf.len());
}

#[test]
fn cjk_and_emoji_fall_back_to_the_replacement_character() {
    let pdf = render_pdf(&text_report("日本語 ok 👍🏽\u{200D}🔥 ✓"), Theme::Dark).unwrap();
    let text = text_of(&pdf);
    assert!(text.contains("\u{FFFD}\u{FFFD}\u{FFFD} ok \u{FFFD}\u{FFFD} ✓"), "{text}");
    assert!(inflated(&pdf).contains("<FFFD>"));
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(48))]

    #[test]
    fn any_text_renders_without_panicking(message in "\\PC{0,200}", title in proptest::prelude::any::<String>()) {
        let mut r = text_report(&message);
        r.title = title;
        let pdf = render_pdf(&r, Theme::Light).unwrap();
        proptest::prop_assert!(pdf.starts_with(b"%PDF-"));
        // Every glyph the pages show decodes through a ToUnicode CMap.
        let _ = text_of(&pdf);
    }
}
