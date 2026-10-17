//! Custom marker icons carried by profiles (`[[icon]]`). Imported artwork is
//! untrusted: SVG is rebuilt from an allowlist of plain drawing elements and
//! attributes (no scripts, links, styles, external references or event
//! handlers) and PNG is checked and size-capped, so an icon can never execute
//! anything in the UI.

use anyhow::{anyhow, Result};
use base64::Engine;
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, Writer};
use serde::{Deserialize, Serialize};

/// Largest accepted icon source (SVG text or decoded PNG), in bytes.
pub const MAX_ICON_BYTES: usize = 16 * 1024;

const ELEMENTS: &[&str] = &[
    "svg", "g", "path", "circle", "ellipse", "rect", "line", "polyline", "polygon", "title",
];
const ATTRIBUTES: &[&str] = &[
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

/// An `[[icon]]` entry as written in a profile.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IconCfg {
    pub name: String,
    #[serde(default)]
    pub svg: Option<String>,
    /// Base64-encoded PNG.
    #[serde(default)]
    pub png: Option<String>,
}

/// A validated icon, safe to render.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CustomIcon {
    pub name: String,
    /// Sanitised SVG markup.
    pub svg: Option<String>,
    /// `data:image/png;base64,...`
    pub png_data_url: Option<String>,
}

pub fn compile(cfg: &IconCfg) -> Result<CustomIcon> {
    let name = cfg.name.trim();
    if name.is_empty() || !name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_') {
        return Err(anyhow!("icon name \"{name}\" must use letters, digits, '-' or '_'"));
    }
    if crate::profile::MARKER_ICONS.contains(&name) {
        return Err(anyhow!("icon \"{name}\" clashes with a built-in icon"));
    }
    match (&cfg.svg, &cfg.png) {
        (Some(svg), None) => Ok(CustomIcon {
            name: name.into(),
            svg: Some(sanitize_svg(svg)?),
            png_data_url: None,
        }),
        (None, Some(png)) => Ok(CustomIcon {
            name: name.into(),
            svg: None,
            png_data_url: Some(validate_png(png)?),
        }),
        _ => Err(anyhow!("icon \"{name}\" needs exactly one of `svg` or `png`")),
    }
}

/// Rebuilds SVG markup keeping only allowed elements and attributes.
pub fn sanitize_svg(svg: &str) -> Result<String> {
    if svg.len() > MAX_ICON_BYTES {
        return Err(anyhow!("SVG icons are limited to {} KB", MAX_ICON_BYTES / 1024));
    }
    let mut reader = Reader::from_str(svg);
    reader.config_mut().trim_text(true);
    let mut writer = Writer::new(Vec::new());
    // Depth inside a dropped element (its whole subtree is dropped).
    let mut skipping = 0usize;
    let mut saw_root = false;
    loop {
        let event = reader.read_event().map_err(|e| anyhow!("not valid SVG: {e}"))?;
        match event {
            // Inside a dropped element: track nesting, emit nothing.
            Event::Start(_) if skipping > 0 => skipping += 1,
            Event::End(_) if skipping > 0 => skipping -= 1,
            Event::Empty(_) | Event::Text(_) if skipping > 0 => {}
            Event::Start(e) => match clean(&e, &mut saw_root)? {
                Some(el) => writer.write_event(Event::Start(el))?,
                None => skipping = 1,
            },
            Event::Empty(e) => {
                if let Some(el) = clean(&e, &mut saw_root)? {
                    writer.write_event(Event::Empty(el))?;
                }
            }
            Event::End(e) => writer.write_event(Event::End(e))?,
            Event::Text(t) => writer.write_event(Event::Text(t))?,
            Event::Eof => break,
            // Comments, processing instructions, doctypes, CDATA: dropped.
            _ => {}
        }
    }
    if !saw_root {
        return Err(anyhow!("an SVG icon must have an <svg> root element"));
    }
    Ok(String::from_utf8(writer.into_inner())?)
}

/// The element with only allowed attributes, or `None` to drop it (and its children).
fn clean<'a>(e: &BytesStart<'a>, saw_root: &mut bool) -> Result<Option<BytesStart<'static>>> {
    let name = String::from(e.local_name().as_ref());
    if !*saw_root {
        if name != "svg" {
            return Err(anyhow!("an SVG icon must have an <svg> root element"));
        }
        *saw_root = true;
    }
    if !ELEMENTS.contains(&name.as_str()) {
        return Ok(None);
    }
    let mut out = BytesStart::new(name);
    for attr in e.attributes().flatten() {
        let key = String::from(attr.key.as_ref());
        let value = attr
            .normalized_value(quick_xml::XmlVersion::Implicit1_0)
            .map(|v| v.into_owned())
            .unwrap_or_default();
        let lower = value.to_ascii_lowercase();
        let dangerous = lower.contains("url(") || lower.contains("javascript:") || lower.contains("expression(") || lower.contains("data:");
        if ATTRIBUTES.contains(&key.as_str()) && !dangerous {
            out.push_attribute((key.as_str(), value.as_str()));
        }
    }
    Ok(Some(out))
}

/// Checks a base64 PNG (signature, size) and returns it as a data URL.
pub fn validate_png(b64: &str) -> Result<String> {
    let cleaned: String = b64.chars().filter(|c| !c.is_whitespace()).collect();
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(&cleaned)
        .map_err(|_| anyhow!("PNG icon is not valid base64"))?;
    if bytes.len() > MAX_ICON_BYTES {
        return Err(anyhow!("PNG icons are limited to {} KB", MAX_ICON_BYTES / 1024));
    }
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return Err(anyhow!("icon is not a PNG image"));
    }
    Ok(format!("data:image/png;base64,{cleaned}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_plain_drawing() {
        let svg = r#"<?xml version="1.0"?><!-- c --><svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24"><g fill="none" stroke="currentColor"><path d="M4 4h16"/><circle cx="12" cy="12" r="3"/></g></svg>"#;
        let out = sanitize_svg(svg).unwrap();
        assert!(
            out.starts_with("<svg xmlns=\"http://www.w3.org/2000/svg\" viewBox=\"0 0 24 24\">"),
            "{out}"
        );
        assert!(out.contains("<path d=\"M4 4h16\"/>") && out.contains("<circle cx=\"12\""));
        assert!(!out.contains("<!--") && !out.contains("<?xml"));
    }

    #[test]
    fn strips_anything_executable_or_external() {
        let svg = r##"<svg viewBox="0 0 10 10" onload="alert(1)" id="x" style="a">
            <script>alert(1)</script>
            <foreignObject><div>hi</div><script>x</script></foreignObject>
            <a href="https://evil"><path d="M0 0"/></a>
            <image href="data:image/png;base64,AAAA"/>
            <use href="#x"/>
            <rect width="5" height="5" fill="url(#g)" stroke="red" onclick="x"/>
            <path d="M1 1" transform="javascript:alert(1)"/>
            <title>ok</title>
        </svg>"##;
        let out = sanitize_svg(svg).unwrap();
        for bad in [
            "script",
            "onload",
            "onclick",
            "foreignObject",
            "href",
            "<a",
            "image",
            "<use",
            "url(",
            "javascript",
            "style=",
            "id=",
        ] {
            assert!(!out.contains(bad), "{bad} survived: {out}");
        }
        assert!(out.contains("<rect width=\"5\" height=\"5\" stroke=\"red\"/>"), "{out}");
        assert!(out.contains("<title>ok</title>"));
    }

    #[test]
    fn rejects_non_svg_and_oversized() {
        assert!(sanitize_svg("<html><svg/></html>").is_err());
        assert!(sanitize_svg("not xml <<<").is_err());
        assert!(sanitize_svg("").unwrap_err().to_string().contains("<svg> root"));
        assert!(sanitize_svg(&format!("<svg>{}</svg>", "<g/>".repeat(5000)))
            .unwrap_err()
            .to_string()
            .contains("limited"));
    }

    #[test]
    fn png_checks() {
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\nrest");
        assert!(validate_png(&png).unwrap().starts_with("data:image/png;base64,"));
        assert!(validate_png(&base64::engine::general_purpose::STANDARD.encode(b"GIF89a"))
            .unwrap_err()
            .to_string()
            .contains("not a PNG"));
        assert!(validate_png("!!!").is_err());
        let big = base64::engine::general_purpose::STANDARD.encode(vec![0u8; MAX_ICON_BYTES + 1]);
        assert!(validate_png(&big).unwrap_err().to_string().contains("limited"));
    }

    #[test]
    fn compile_checks_names_and_sources() {
        let ok = compile(&IconCfg {
            name: "acme-pay".into(),
            svg: Some("<svg viewBox='0 0 1 1'/>".into()),
            png: None,
        })
        .unwrap();
        assert_eq!(ok.name, "acme-pay");
        assert!(compile(&IconCfg {
            name: "bad name".into(),
            svg: Some("<svg/>".into()),
            png: None
        })
        .is_err());
        assert!(compile(&IconCfg {
            name: "bug".into(),
            svg: Some("<svg/>".into()),
            png: None
        })
        .unwrap_err()
        .to_string()
        .contains("built-in"));
        assert!(compile(&IconCfg {
            name: "x".into(),
            svg: None,
            png: None
        })
        .is_err());
        let png = base64::engine::general_purpose::STANDARD.encode(b"\x89PNG\r\n\x1a\n");
        assert!(compile(&IconCfg {
            name: "p".into(),
            svg: None,
            png: Some(png)
        })
        .unwrap()
        .png_data_url
        .is_some());
    }
}
