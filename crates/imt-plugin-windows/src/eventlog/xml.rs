//! Rendered event XML (what `EvtRender(EvtRenderEventXml)` returns) to the
//! records the `.evtx` importer produces.
//!
//! The XML is first turned into the same JSON shape the `evtx` crate emits
//! (`#attributes`, `#text`, flattened named `Data`, `_N` suffixes for
//! repeated elements), so [`crate::evtx_import::to_record`] maps both sources
//! and the field names, levels and messages are identical: markers, the
//! Windows annotators and reboot detection work on live events unchanged.

use crate::evtx_import::to_record;
use anyhow::{anyhow, bail, Result};
use imt_plugin_api::model::{auto_value, Record};
use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};
use serde_json::{Map, Value};

/// Largest event XML accepted (the Event Log caps events at 64 KiB of data).
pub const MAX_XML_BYTES: usize = 1 << 20;
/// Deepest element nesting accepted.
const MAX_DEPTH: usize = 32;
/// Most elements in one event.
const MAX_ELEMENTS: usize = 20_000;
/// Longest formatted message kept (characters); longer ones end with `…`.
pub const MAX_MESSAGE_CHARS: usize = 16_000;

#[derive(Debug, Default)]
struct Node {
    name: String,
    attrs: Vec<(String, String)>,
    children: Vec<Node>,
    text: String,
}

impl Node {
    fn has_text(&self) -> bool {
        !self.text.is_empty()
    }

    fn attr(&self, name: &str) -> Option<&str> {
        self.attrs.iter().find(|(k, _)| k == name).map(|(_, v)| v.as_str())
    }
}

fn local_name(name: &str) -> String {
    match name.rsplit_once(':') {
        Some((_, local)) => local.to_string(),
        None => name.to_string(),
    }
}

fn open(e: &BytesStart<'_>) -> Node {
    let mut node = Node {
        name: local_name(e.local_name().as_ref()),
        ..Node::default()
    };
    for a in e.attributes().flatten() {
        let key: String = a.key.as_ref().to_string();
        if key == "xmlns" || key.starts_with("xmlns:") {
            continue;
        }
        let value = a
            .normalized_value(XmlVersion::Implicit1_0)
            .map(|v| v.into_owned())
            .unwrap_or_default();
        // Like the evtx crate: empty attributes are left out.
        if !value.is_empty() {
            node.attrs.push((local_name(&key), value));
        }
    }
    node
}

fn entity(name: &str) -> &'static str {
    match name {
        "amp" => "&",
        "lt" => "<",
        "gt" => ">",
        "quot" => "\"",
        "apos" => "'",
        _ => "",
    }
}

/// Parses the XML into a tree of elements. Never panics; malformed XML,
/// excessive size or nesting are errors.
fn parse_tree(xml: &str) -> Result<Node> {
    if xml.len() > MAX_XML_BYTES {
        bail!("the event XML is larger than {MAX_XML_BYTES} bytes");
    }
    let mut reader = Reader::from_str(xml);
    let mut stack: Vec<Node> = Vec::new();
    let mut elements = 0usize;
    let close = |stack: &mut Vec<Node>| -> Option<Node> {
        let mut done = stack.pop()?;
        if !done.children.is_empty() && done.text.trim().is_empty() {
            // Indentation between child elements is not content.
            done.text.clear();
        }
        match stack.last_mut() {
            Some(parent) => {
                parent.children.push(done);
                None
            }
            None => Some(done),
        }
    };
    loop {
        let event = reader.read_event().map_err(|e| anyhow!("malformed event XML: {e}"))?;
        match event {
            Event::Eof => bail!("the event XML ends before </Event>"),
            Event::Start(e) | Event::Empty(e) if stack.len() >= MAX_DEPTH => {
                let _ = e;
                bail!("the event XML nests deeper than {MAX_DEPTH} elements")
            }
            Event::Start(e) => {
                elements += 1;
                if elements > MAX_ELEMENTS {
                    bail!("the event XML has more than {MAX_ELEMENTS} elements");
                }
                stack.push(open(&e));
            }
            Event::Empty(e) => {
                elements += 1;
                if elements > MAX_ELEMENTS {
                    bail!("the event XML has more than {MAX_ELEMENTS} elements");
                }
                stack.push(open(&e));
                if let Some(root) = close(&mut stack) {
                    return Ok(root);
                }
            }
            Event::End(_) => {
                if stack.is_empty() {
                    bail!("malformed event XML: unexpected closing tag");
                }
                if let Some(root) = close(&mut stack) {
                    return Ok(root);
                }
            }
            Event::Text(t) => {
                if let Some(top) = stack.last_mut() {
                    // Raw text: line ends stay as the provider wrote them (CR LF), like the importer's.
                    top.text.push_str(&t);
                }
            }
            Event::GeneralRef(r) => {
                if let Some(top) = stack.last_mut() {
                    match r.resolve_char_ref() {
                        Ok(Some(c)) => top.text.push(c),
                        _ => top.text.push_str(entity(r.as_ref())),
                    }
                }
            }
            Event::CData(c) => {
                if let Some(top) = stack.last_mut() {
                    top.text.push_str(&c);
                }
            }
            _ => {}
        }
    }
}

fn is_container(name: &str) -> bool {
    name == "EventData" || name == "UserData"
}

fn attributes(node: &Node) -> Map<String, Value> {
    node.attrs.iter().map(|(k, v)| (k.clone(), auto_value(v))).collect()
}

/// An element's value: a scalar for text-only elements, `null` for empty
/// ones, otherwise an object.
fn element_value(node: &Node) -> Value {
    if node.children.is_empty() && node.attrs.is_empty() {
        return if node.has_text() { auto_value(&node.text) } else { Value::Null };
    }
    Value::Object(element_body(node, is_container(&node.name), false))
}

/// The value of one `Data` element inside `EventData` / `UserData`.
fn data_value(node: &Node) -> Value {
    if node.children.is_empty() {
        return if node.has_text() {
            auto_value(&node.text)
        } else {
            Value::String(String::new())
        };
    }
    Value::Object(element_body(node, false, true))
}

fn element_body(node: &Node, in_container: bool, omit_attributes: bool) -> Map<String, Value> {
    let mut out = Map::new();
    if !omit_attributes && !node.attrs.is_empty() {
        out.insert("#attributes".into(), Value::Object(attributes(node)));
    }
    if node.has_text() {
        out.insert("#text".into(), auto_value(&node.text));
    }
    let named = in_container
        && node
            .children
            .iter()
            .any(|c| c.name == "Data" && c.attr("Name").is_some_and(|n| !n.is_empty()));
    let positional: Vec<&Node> = if in_container && !named {
        node.children.iter().filter(|c| c.name == "Data").collect()
    } else {
        Vec::new()
    };
    let mut positional_done = false;
    let mut counts: Vec<(&str, u16)> = Vec::new();
    for child in &node.children {
        if in_container && child.name == "Data" {
            if named {
                if let Some(name) = child.attr("Name").filter(|n| !n.is_empty()) {
                    out.insert(name.to_string(), data_value(child));
                }
            } else if !positional_done {
                positional_done = true;
                let text = match positional.as_slice() {
                    [one] => data_value(one),
                    many => Value::Array(many.iter().map(|d| data_value(d)).collect()),
                };
                let mut wrapper = Map::new();
                wrapper.insert("#text".into(), text);
                out.insert("Data".into(), Value::Object(wrapper));
            }
            continue;
        }
        let suffix = match counts.iter_mut().find(|(n, _)| *n == child.name) {
            Some((_, c)) => {
                let s = *c;
                *c += 1;
                s
            }
            None => {
                counts.push((&child.name, 1));
                0
            }
        };
        let key = if suffix > 0 {
            format!("{}_{suffix}", child.name)
        } else {
            child.name.clone()
        };
        out.insert(key, element_value(child));
    }
    out
}

/// The event as the `evtx` crate's JSON (`{"Event": {"System": ...}}`).
pub fn event_json(xml: &str) -> Result<Value> {
    let root = parse_tree(xml)?;
    if root.name != "Event" {
        bail!(
            "the XML is not an event (root element <{}>)",
            root.name.chars().take(40).collect::<String>()
        );
    }
    let mut out = Map::new();
    out.insert("Event".into(), Value::Object(element_body(&root, false, false)));
    Ok(Value::Object(out))
}

/// `System/EventRecordID` of an event in [`event_json`] form.
pub fn record_id(json: &Value) -> Option<u64> {
    let v = json.get("Event")?.get("System")?.get("EventRecordID")?;
    let v = v.get("#text").unwrap_or(v);
    v.as_u64().or_else(|| v.as_str()?.trim().parse().ok())
}

/// `System/Provider/@Name`, read without building the whole event (to look up
/// the provider's message table before the event is mapped).
pub fn provider_name(xml: &str) -> Option<String> {
    let mut reader = Reader::from_str(xml);
    for _ in 0..64 {
        match reader.read_event() {
            Ok(Event::Start(e) | Event::Empty(e)) if e.local_name().as_ref() == "Provider" => {
                return open(&e).attr("Name").map(str::to_string);
            }
            Ok(Event::Eof) | Err(_) => return None,
            _ => {}
        }
    }
    None
}

/// Tidies a formatted message: trailing whitespace trimmed (messages end
/// with CR LF), bounded; `None` when nothing is left.
pub fn clean_message(message: &str) -> Option<String> {
    let t = message
        .trim_end_matches(['\r', '\n', ' ', '\t', '\0'])
        .trim_start_matches(['\r', '\n']);
    if t.trim().is_empty() {
        return None;
    }
    if t.chars().count() > MAX_MESSAGE_CHARS {
        let mut s: String = t.chars().take(MAX_MESSAGE_CHARS - 1).collect();
        s.push('\u{2026}');
        return Some(s);
    }
    Some(t.to_string())
}

/// One rendered event as a record with the importer's field names. The
/// message is the formatted one when the provider's message table had it,
/// otherwise the importer's `Provider EventId: k=v, ...` from the event data.
pub fn record_from_xml(xml: &str, message: Option<&str>, line: u32) -> Result<Record> {
    let json = event_json(xml)?;
    let id = record_id(&json).unwrap_or(0);
    let mut record = to_record(&json, id, line);
    if let Some(m) = message.and_then(clean_message) {
        record.message = m;
    }
    Ok(record)
}

#[cfg(test)]
mod tests;
