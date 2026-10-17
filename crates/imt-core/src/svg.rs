//! Sanitising untrusted SVG: the markup is rebuilt from an allowlist of
//! plain drawing elements and attributes (no scripts, links, external
//! references, foreign content or event handlers), so it can never execute
//! anything where it is shown: marker icons in the UI, chart and timeline
//! snapshots in reports and exports.

use anyhow::{anyhow, Result};
use quick_xml::events::{BytesStart, BytesText, Event};
use quick_xml::{Reader, Writer};

/// What a kind of SVG may contain.
pub struct Policy {
    /// "SVG icons", "SVG images": used in messages.
    what: &'static str,
    /// "an SVG icon", "an SVG image".
    one: &'static str,
    max_bytes: usize,
    elements: &'static [&'static str],
    attributes: &'static [&'static str],
    /// `url(#id)` references to the document's own definitions (clip paths,
    /// gradients) and a filtered `style` attribute are kept.
    figures: bool,
}

const DRAWING: &[&str] = &[
    "svg", "g", "path", "circle", "ellipse", "rect", "line", "polyline", "polygon", "title",
];
const DRAWING_ATTRIBUTES: &[&str] = &[
    "xmlns",
    "viewBox",
    "width",
    "height",
    "d",
    "cx",
    "cy",
    "r",
    "rx",
    "ry",
    "x",
    "y",
    "x1",
    "y1",
    "x2",
    "y2",
    "points",
    "fill",
    "fill-rule",
    "fill-opacity",
    "stroke",
    "stroke-width",
    "stroke-linecap",
    "stroke-linejoin",
    "stroke-opacity",
    "stroke-dasharray",
    "clip-rule",
    "opacity",
    "transform",
];

/// Marker icons from profiles: shapes only, 16 KB.
pub const ICON: Policy = Policy {
    what: "SVG icons",
    one: "an SVG icon",
    max_bytes: 16 * 1024,
    elements: DRAWING,
    attributes: DRAWING_ATTRIBUTES,
    figures: false,
};

/// Charts and timelines rendered by the UI: shapes, text, clip paths and
/// gradients, 4 MiB.
pub const FIGURE: Policy = Policy {
    what: "SVG images",
    one: "an SVG image",
    max_bytes: 4 << 20,
    elements: &[
        "svg",
        "g",
        "path",
        "circle",
        "ellipse",
        "rect",
        "line",
        "polyline",
        "polygon",
        "title",
        "desc",
        "text",
        "tspan",
        "defs",
        "clipPath",
        "linearGradient",
        "radialGradient",
        "stop",
    ],
    attributes: &[
        "xmlns",
        "viewBox",
        "width",
        "height",
        "d",
        "cx",
        "cy",
        "r",
        "rx",
        "ry",
        "fx",
        "fy",
        "x",
        "y",
        "dx",
        "dy",
        "x1",
        "y1",
        "x2",
        "y2",
        "points",
        "fill",
        "fill-rule",
        "fill-opacity",
        "stroke",
        "stroke-width",
        "stroke-linecap",
        "stroke-linejoin",
        "stroke-opacity",
        "stroke-dasharray",
        "stroke-dashoffset",
        "stroke-miterlimit",
        "clip-rule",
        "clip-path",
        "opacity",
        "transform",
        "id",
        "class",
        "preserveAspectRatio",
        "font-family",
        "font-size",
        "font-weight",
        "font-style",
        "text-anchor",
        "dominant-baseline",
        "alignment-baseline",
        "letter-spacing",
        "paint-order",
        "offset",
        "stop-color",
        "stop-opacity",
        "gradientUnits",
        "gradientTransform",
        "clipPathUnits",
        "visibility",
        "style",
    ],
    figures: true,
};

/// `style` properties a figure may set.
const STYLE_PROPERTIES: &[&str] = &[
    "font",
    "font-family",
    "font-size",
    "font-weight",
    "font-style",
    "fill",
    "fill-opacity",
    "stroke",
    "stroke-width",
    "stroke-opacity",
    "opacity",
    "text-anchor",
    "dominant-baseline",
    "white-space",
    "letter-spacing",
    "paint-order",
];

/// Rebuilds SVG markup keeping only what `policy` allows.
pub fn sanitize(svg: &str, policy: &Policy) -> Result<String> {
    sanitize_text(svg, policy, &mut |t| t.to_string())
}

/// [`sanitize`], passing the text of `<text>`/`<title>` nodes through `text`
/// (e.g. redaction).
pub fn sanitize_text(svg: &str, policy: &Policy, text: &mut dyn FnMut(&str) -> String) -> Result<String> {
    if svg.len() > policy.max_bytes {
        return Err(anyhow!("{} are limited to {} KB", policy.what, policy.max_bytes / 1024));
    }
    let mut reader = Reader::from_str(svg);
    // Figures keep the spaces of their text; icons have none worth keeping.
    reader.config_mut().trim_text(!policy.figures);
    let mut writer = Writer::new(Vec::new());
    // Depth inside a dropped element (its whole subtree is dropped).
    let mut skipping = 0usize;
    let mut saw_root = false;
    // Text (and resolved references) of the current text run.
    let mut pending = String::new();
    loop {
        let event = reader.read_event().map_err(|e| anyhow!("not valid SVG: {e}"))?;
        match &event {
            Event::Text(_) | Event::GeneralRef(_) => {}
            _ if !pending.is_empty() => {
                writer.write_event(Event::Text(BytesText::new(&text(&pending))))?;
                pending.clear();
            }
            _ => {}
        }
        match event {
            // Inside a dropped element: track nesting, emit nothing.
            Event::Start(_) if skipping > 0 => skipping += 1,
            Event::End(_) if skipping > 0 => skipping -= 1,
            Event::Empty(_) | Event::Text(_) | Event::GeneralRef(_) if skipping > 0 => {}
            Event::Start(e) => match clean(&e, &mut saw_root, policy)? {
                Some(el) => writer.write_event(Event::Start(el))?,
                None => skipping = 1,
            },
            Event::Empty(e) => {
                if let Some(el) = clean(&e, &mut saw_root, policy)? {
                    writer.write_event(Event::Empty(el))?;
                }
            }
            Event::End(e) => writer.write_event(Event::End(e))?,
            Event::Text(t) => pending.push_str(&t.xml10_content()),
            Event::GeneralRef(r) => pending.push_str(&resolve_ref(&r)),
            Event::Eof => break,
            // Comments, processing instructions, doctypes, CDATA: dropped.
            _ => {}
        }
    }
    if !saw_root {
        return Err(anyhow!("{} must have an <svg> root element", policy.one));
    }
    Ok(String::from_utf8(writer.into_inner())?)
}

/// `&amp;` and the other predefined entities, `&#65;` / `&#x41;`; anything
/// else (a custom entity) is dropped.
fn resolve_ref(r: &quick_xml::events::BytesRef<'_>) -> String {
    if let Ok(Some(c)) = r.resolve_char_ref() {
        return c.to_string();
    }
    match r.xml10_content().as_ref() {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => "",
    }
    .to_string()
}

/// The element with only allowed attributes, or `None` to drop it (and its children).
fn clean(e: &BytesStart<'_>, saw_root: &mut bool, policy: &Policy) -> Result<Option<BytesStart<'static>>> {
    let name = String::from(e.local_name().as_ref());
    if !*saw_root {
        if name != "svg" {
            return Err(anyhow!("{} must have an <svg> root element", policy.one));
        }
        *saw_root = true;
    }
    if !policy.elements.contains(&name.as_str()) {
        return Ok(None);
    }
    let mut out = BytesStart::new(name);
    for attr in e.attributes().flatten() {
        let key = String::from(attr.key.as_ref());
        if !policy.attributes.contains(&key.as_str()) {
            continue;
        }
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map(|v| v.into_owned())
            .unwrap_or_default();
        let value = if key == "style" { clean_style(&value) } else { Some(value) };
        if let Some(value) = value.filter(|v| safe_value(v, policy.figures)) {
            out.push_attribute((key.as_str(), value.as_str()));
        }
    }
    Ok(Some(out))
}

/// No script, data or external URLs; with `local_urls`, `url(#id)` is fine.
fn safe_value(value: &str, local_urls: bool) -> bool {
    let lower = value.to_ascii_lowercase();
    if lower.contains("javascript:") || lower.contains("expression(") || lower.contains("data:") || lower.contains('\\') {
        return false;
    }
    let mut rest = lower.as_str();
    while let Some(i) = rest.find("url(") {
        let target = rest[i + 4..].trim_start_matches([' ', '\'', '"']);
        if !local_urls || !target.starts_with('#') {
            return false;
        }
        rest = &rest[i + 4..];
    }
    true
}

/// Keeps the allowed declarations of a `style` attribute.
fn clean_style(style: &str) -> Option<String> {
    let kept: Vec<String> = style
        .split(';')
        .filter_map(|decl| {
            let (prop, value) = decl.split_once(':')?;
            let prop = prop.trim().to_ascii_lowercase();
            let value = value.trim();
            (STYLE_PROPERTIES.contains(&prop.as_str()) && !value.contains('@') && safe_value(value, true))
                .then(|| format!("{prop}:{value}"))
        })
        .collect();
    (!kept.is_empty()).then(|| kept.join(";"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn figures_keep_text_clip_paths_gradients_and_safe_styles() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="10" height="10">
            <defs><clipPath id="c0"><rect width="5" height="5"/></clipPath>
            <linearGradient id="g"><stop offset="0" stop-color="red"/></linearGradient></defs>
            <g clip-path="url(#c0)"><path d="M0 0" fill="url(#g)" style="stroke:red; behavior:url(x.htc); font: 12px sans-serif; fill:url(http://x)"/></g>
            <text x="1" y="2" text-anchor="middle" style="@import x">A &amp; B &#65;&#x42; &custom; &lt;b&gt;</text>
            <style>text { fill: red }</style>
            <path d="M1 1" fill="url( 'https://evil')"/>
            <rect style="behavior:url(x)" width="1"/>
        </svg>"##;
        let out = sanitize(svg, &FIGURE).unwrap();
        assert!(out.contains(r##"<clipPath id="c0">"##), "{out}");
        assert!(
            out.contains(r##"clip-path="url(#c0)""##) && out.contains(r##"fill="url(#g)""##),
            "{out}"
        );
        assert!(out.contains(r#"style="stroke:red;font:12px sans-serif""#), "{out}");
        assert!(
            out.contains("<text x=\"1\" y=\"2\" text-anchor=\"middle\">A &amp; B AB  &lt;b&gt;</text>"),
            "{out}"
        );
        assert!(
            !out.contains("<style") && !out.contains("evil") && !out.contains("htc") && !out.contains("http://x"),
            "{out}"
        );
        assert!(out.contains("<rect width=\"1\"/>"), "{out}");
    }

    #[test]
    fn figure_text_passes_through_the_callback() {
        let out = sanitize_text(
            "<svg><text>mail a@b.io</text><title>t</title><script>x</script></svg>",
            &FIGURE,
            &mut |t| t.replace("a@b.io", "<email>"),
        )
        .unwrap();
        assert_eq!(out, "<svg><text>mail &lt;email&gt;</text><title>t</title></svg>");
    }

    #[test]
    fn figures_are_size_limited_and_need_a_root() {
        let big = format!("<svg>{}</svg>", "<g/>".repeat(1 << 20));
        assert!(sanitize(&big, &FIGURE).unwrap_err().to_string().contains("SVG images are limited"));
        assert!(sanitize("<p/>", &FIGURE).unwrap_err().to_string().contains("an SVG image must"));
        assert!(sanitize("", &FIGURE).unwrap_err().to_string().contains("an SVG image must"));
    }

    #[test]
    fn values_and_styles() {
        assert!(safe_value("url(#a) url(#b)", true));
        assert!(!safe_value("url(#a)", false));
        assert!(!safe_value("a\\b", true));
        assert_eq!(clean_style("color: red"), None);
        assert_eq!(clean_style("fill: red; nonsense"), Some("fill:red".into()));
    }
}
