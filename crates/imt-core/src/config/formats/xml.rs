//! XML (`.xml`, .NET `.config`, project files...). Elements are keys,
//! attributes are `@name` children, element text is the element's value.
//!
//! Repeated siblings are told apart by an identifying attribute when they
//! have one (`key`, `name` or `id`: `add[key=Timeout]`, as in .NET
//! `appSettings`), else by position (`add`, `add[1]`, `add[2]`...), so a
//! reordered `appSettings` does not show as a change of every entry.

use super::{by_extension, head_of};
use crate::config::{ConfigError, ConfigParser, ConfigTree, ConfigValue, Limits, PathSeg, TreeBuilder};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use std::collections::HashMap;

pub struct XmlParser;

const EXTENSIONS: [&str; 8] = ["xml", "config", "csproj", "props", "targets", "xaml", "manifest", "resx"];
/// Attributes that identify an element among its siblings, in order of preference.
const IDENTITY: [&str; 3] = ["key", "name", "id"];

impl ConfigParser for XmlParser {
    fn id(&self) -> &str {
        "xml"
    }

    fn name(&self) -> &str {
        "XML"
    }

    fn extensions(&self) -> Vec<String> {
        EXTENSIONS.iter().map(|e| e.to_string()).collect()
    }

    fn sniff(&self, file_name: &str, head: &str) -> u8 {
        let ext = by_extension(file_name, &EXTENSIONS);
        if ext > 0 {
            return ext;
        }
        let head = head_of(head);
        if head.starts_with("<?xml") {
            60
        } else if head.starts_with('<') {
            40
        } else {
            0
        }
    }

    fn parse(&self, content: &str, limits: &Limits) -> Result<ConfigTree, ConfigError> {
        let mut reader = Reader::from_str(content.trim_start_matches('\u{feff}'));
        let mut b = TreeBuilder::new(self.id(), limits);
        let mut stack: Vec<Frame> = Vec::new();
        let mut root_seen = HashMap::new();
        let syntax = |reader: &Reader<&[u8]>, e: &dyn std::fmt::Display| {
            ConfigError::syntax("XML", format!("at byte {}: {e}", reader.buffer_position()))
        };
        loop {
            let event = reader.read_event().map_err(|e| syntax(&reader, &e))?;
            match event {
                Event::Eof => break,
                Event::Start(_) | Event::Empty(_) if b.is_full() => {
                    // Stop reading: the rest of the document is not kept.
                    b.mark_truncated();
                    return Ok(b.finish());
                }
                Event::Start(e) => {
                    let frame = open(&mut b, &mut stack, &mut root_seen, &e).map_err(|err| match err {
                        Open::Config(c) => c,
                        Open::Attr(m) => syntax(&reader, &m),
                    })?;
                    stack.push(frame);
                }
                Event::Empty(e) => {
                    let frame = open(&mut b, &mut stack, &mut root_seen, &e).map_err(|err| match err {
                        Open::Config(c) => c,
                        Open::Attr(m) => syntax(&reader, &m),
                    })?;
                    close(&mut b, frame)?;
                }
                // check_end_names guarantees a matching open element.
                Event::End(_) => stack.pop().map(|frame| close(&mut b, frame)).transpose().map(|_| ())?,
                Event::Text(t) => append(&mut stack, &t.xml_content(XmlVersion::Implicit1_0)),
                Event::CData(c) => append(&mut stack, c.as_ref()),
                Event::GeneralRef(r) => {
                    let text = match r.resolve_char_ref() {
                        Ok(Some(c)) => c.to_string(),
                        _ => match r.as_ref() {
                            "amp" => "&".into(),
                            "lt" => "<".into(),
                            "gt" => ">".into(),
                            "quot" => "\"".into(),
                            "apos" => "'".into(),
                            other => format!("&{other};"),
                        },
                    };
                    append(&mut stack, &text);
                }
                _ => {}
            }
        }
        if let Some(open) = stack.last() {
            return Err(ConfigError::syntax("XML", format!("unexpected end of file inside <{}>", open.name)));
        }
        Ok(b.finish())
    }
}

/// An open element.
struct Frame {
    name: String,
    path: Vec<PathSeg>,
    text: String,
    /// Occurrences of each child name (and identity) so far.
    seen: HashMap<String, usize>,
}

enum Open {
    Config(ConfigError),
    Attr(String),
}

fn open(b: &mut TreeBuilder, stack: &mut [Frame], root_seen: &mut HashMap<String, usize>, e: &BytesStart) -> Result<Frame, Open> {
    let name = e.name().as_ref().to_string();
    let mut attrs = Vec::new();
    for a in e.attributes() {
        let a = a.map_err(|err| Open::Attr(err.to_string()))?;
        let key = a.key.as_ref().to_string();
        let value = a
            .normalized_value(XmlVersion::Implicit1_0)
            .map_err(|err| Open::Attr(err.to_string()))?
            .into_owned();
        attrs.push((key, value));
    }
    let identity = IDENTITY
        .iter()
        .find_map(|id| attrs.iter().find(|(k, _)| k == id).map(|(k, v)| format!("{name}[{k}={v}]")));
    let (parent, seen) = match stack.last_mut() {
        Some(f) => (f.path.clone(), &mut f.seen),
        None => (Vec::new(), root_seen),
    };
    let base = identity.unwrap_or_else(|| name.clone());
    let n = seen.entry(base.clone()).or_insert(0);
    let seg = match (*n, base == name) {
        (0, _) => base,
        (k, true) => format!("{name}[{k}]"),
        (k, false) => format!("{base}#{k}"),
    };
    *n += 1;
    let mut path = parent;
    path.push(PathSeg::Key(seg));
    b.check_depth(path.len() + 1).map_err(Open::Config)?;
    for (k, v) in attrs {
        let mut p = path.clone();
        p.push(PathSeg::Key(format!("@{k}")));
        b.push(p, ConfigValue::String(v)).map_err(Open::Config)?;
    }
    Ok(Frame {
        name,
        path,
        text: String::new(),
        seen: HashMap::new(),
    })
}

fn close(b: &mut TreeBuilder, frame: Frame) -> Result<(), ConfigError> {
    let text = frame.text.trim();
    let value = if text.is_empty() {
        ConfigValue::Section
    } else {
        ConfigValue::String(text.to_string())
    };
    b.push(frame.path, value)
}

fn append(stack: &mut [Frame], text: &str) {
    if let Some(f) = stack.last_mut() {
        f.text.push_str(text);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::display_path;

    fn shown(t: &ConfigTree) -> Vec<String> {
        let mut v: Vec<String> = t
            .nodes
            .iter()
            .map(|n| format!("{}={}", display_path(&n.path), n.value.text()))
            .collect();
        v.sort();
        v
    }

    #[test]
    fn reads_elements_attributes_and_text() {
        let t = XmlParser
            .parse(
                "\u{feff}<?xml version=\"1.0\"?><configuration><appSettings><add key=\"A\" value=\"1\"/><add key=\"A\" value=\"dup\"/>\
                 <add value=\"x\"/><add value=\"y\"/></appSettings><n>a &amp; b &#65; &custom; <![CDATA[<c>]]></n><!-- c --></configuration>",
                &Limits::default(),
            )
            .unwrap();
        assert_eq!(
            shown(&t),
            [
                "configuration/appSettings/add/@value=x",
                "configuration/appSettings/add=",
                "configuration/appSettings/add[1]/@value=y",
                "configuration/appSettings/add[1]=",
                "configuration/appSettings/add[key=A]#1/@key=A",
                "configuration/appSettings/add[key=A]#1/@value=dup",
                "configuration/appSettings/add[key=A]#1=",
                "configuration/appSettings/add[key=A]/@key=A",
                "configuration/appSettings/add[key=A]/@value=1",
                "configuration/appSettings/add[key=A]=",
                "configuration/appSettings=",
                "configuration/n=a & b A &custom; <c>",
                "configuration=",
            ]
        );
    }

    #[test]
    fn rejects_malformed_and_deep_input() {
        let limits = Limits::default();
        assert!(matches!(XmlParser.parse("<a><b></a>", &limits), Err(ConfigError::Syntax { .. })));
        assert!(XmlParser.parse("<a>", &limits).unwrap_err().to_string().contains("inside <a>"));
        assert!(XmlParser.parse("<a x=1/>", &limits).is_err());
        assert!(XmlParser.parse("<a x=\"&bad\"/>", &limits).is_err());
        assert!(XmlParser.parse("<a x=1></a>", &limits).is_err());
        let deep_empty = format!("{}<b/>", "<a>".repeat(127));
        assert_eq!(XmlParser.parse(&deep_empty, &limits), Err(ConfigError::TooDeep { max: 128 }));
        assert_eq!(XmlParser.parse(&"<a>".repeat(200), &limits), Err(ConfigError::TooDeep { max: 128 }));
        assert_eq!(XmlParser.parse("", &limits).unwrap().nodes.len(), 0);
    }

    #[test]
    fn truncates_at_the_node_limit() {
        let limits = Limits {
            max_nodes: 2,
            ..Limits::default()
        };
        let t = XmlParser.parse("<r><a/><b/><c/><d/></r>", &limits).unwrap();
        assert!(t.truncated);
        assert_eq!(t.nodes.len(), 2);
    }

    #[test]
    fn sniffs() {
        assert_eq!(XmlParser.sniff("web.config", ""), 80);
        assert_eq!(XmlParser.sniff("x", "<?xml"), 60);
        assert_eq!(XmlParser.sniff("x", "<r/>"), 40);
        assert_eq!(XmlParser.sniff("x", "r"), 0);
        assert_eq!((XmlParser.id(), XmlParser.name(), XmlParser.extensions().len()), ("xml", "XML", 8));
    }
}
