//! Saving a TOML document over a hand-written one without losing what the
//! user wrote around the values: comments, blank lines, key order, value
//! formatting and the keys they left out (defaults).
//!
//! It is a three-way merge of text: `existing` is the file, `base` the
//! canonical TOML of what it means (parsed and serialised again), `fresh` the
//! document to save. Only what differs between `base` and `fresh` is written
//! into `existing`, in place: changed values keep the comments around them,
//! removed keys go, new keys are appended. Pure text in, text out.

use toml_edit::{ArrayOfTables, DocumentMut, InlineTable, Item, Table, Value};

/// `existing` updated to the values of `fresh`, touching only what differs
/// from `base` (pass `""` when the meaning of `existing` is unknown). Fails
/// only when one of the texts is not valid TOML.
pub fn update_preserving(existing: &str, base: &str, fresh: &str) -> Result<String, String> {
    let mut doc: DocumentMut = existing.parse().map_err(|e| format!("the existing file is not valid TOML: {e}"))?;
    let base: DocumentMut = base.parse().map_err(|e| format!("invalid TOML: {e}"))?;
    let new: DocumentMut = fresh.parse().map_err(|e| format!("invalid TOML: {e}"))?;
    merge_table(doc.as_table_mut(), Some(base.as_table()), new.as_table());
    Ok(doc.to_string())
}

/// What an item means, whatever its spelling or layout (section or inline table, quotes, number format).
fn meaning(item: &Item) -> Option<toml::Value> {
    let mut d = DocumentMut::new();
    let mut item = item.clone();
    detach(&mut item);
    d.insert("v", item);
    toml::from_str::<toml::Table>(&d.to_string()).ok()?.remove("v")
}

fn equal(a: &Item, b: &Item) -> bool {
    meaning(a).is_some_and(|m| Some(m) == meaning(b))
}

/// Forgets where a table came from, so it is written right after its parent in the new document.
fn detach(item: &mut Item) {
    match item {
        Item::Table(t) => detach_table(t),
        Item::ArrayOfTables(a) => a.iter_mut().for_each(detach_table),
        _ => {}
    }
}

fn detach_table(t: &mut Table) {
    t.set_position(None);
    t.iter_mut().for_each(|(_, item)| detach(item));
}

fn detached(item: &Item) -> Item {
    let mut item = item.clone();
    detach(&mut item);
    item
}

fn merge_table(old: &mut Table, base: Option<&Table>, new: &Table) {
    let gone: Vec<String> = old.iter().map(|(k, _)| k.to_string()).filter(|k| !new.contains_key(k)).collect();
    for k in gone {
        old.remove(&k);
    }
    for (k, item) in new.iter() {
        let b = base.and_then(|b| b.get(k));
        if b.is_some_and(|b| equal(b, item)) {
            // Means the same as before (perhaps a default the file leaves out): left as written.
            continue;
        }
        match old.get_mut(k) {
            Some(slot) => merge_item(slot, b, item),
            None => {
                old.insert(k, detached(item));
            }
        }
    }
}

fn merge_item(old: &mut Item, base: Option<&Item>, new: &Item) {
    match (old, new) {
        (Item::Table(o), Item::Table(n)) => merge_table(o, base.and_then(Item::as_table), n),
        (Item::ArrayOfTables(o), Item::ArrayOfTables(n)) => merge_tables(o, base.and_then(Item::as_array_of_tables), n),
        // Written inline in the file, a section in the new document: stays inline.
        (Item::Value(Value::InlineTable(o)), Item::Table(n)) => {
            let b = base.and_then(as_inline);
            merge_inline(o, b.as_ref(), &n.clone().into_inline_table());
        }
        (Item::Value(o), Item::Value(n)) => merge_value(o, base.and_then(as_inline).as_ref(), n),
        // Any other change of kind: written anew.
        (slot, n) => *slot = detached(n),
    }
}

/// An item as an inline table (sections are converted).
fn as_inline(item: &Item) -> Option<InlineTable> {
    match item {
        Item::Table(t) => Some(t.clone().into_inline_table()),
        other => other.as_inline_table().cloned(),
    }
}

/// `[[x]]` entries are matched by position.
fn merge_tables(old: &mut ArrayOfTables, base: Option<&ArrayOfTables>, new: &ArrayOfTables) {
    while old.len() > new.len() {
        old.remove(old.len() - 1);
    }
    for (i, n) in new.iter().enumerate() {
        match old.get_mut(i) {
            Some(o) => merge_table(o, base.and_then(|b| b.get(i)), n),
            None => {
                let mut t = n.clone();
                detach_table(&mut t);
                old.push(t);
            }
        }
    }
}

fn merge_inline(old: &mut InlineTable, base: Option<&InlineTable>, new: &InlineTable) {
    let gone: Vec<String> = old.iter().map(|(k, _)| k.to_string()).filter(|k| !new.contains_key(k)).collect();
    for k in gone {
        old.remove(&k);
    }
    for (k, v) in new.iter() {
        let b = base.and_then(|b| b.get(k));
        if b.is_some_and(|b| equal(&Item::Value(b.clone()), &Item::Value(v.clone()))) {
            continue;
        }
        match old.get_mut(k) {
            Some(slot) => {
                let b = b.and_then(Value::as_inline_table);
                merge_value(slot, b, v)
            }
            None => {
                old.insert(k, v.clone());
            }
        }
    }
}

fn merge_value(old: &mut Value, base: Option<&InlineTable>, new: &Value) {
    if let (Value::InlineTable(o), Value::InlineTable(n)) = (&mut *old, new) {
        merge_inline(o, base, n);
        return;
    }
    if equal(&Item::Value(old.clone()), &Item::Value(new.clone())) {
        return;
    }
    // A new value, in the place and with the comments of the old one.
    let decor = old.decor().clone();
    *old = new.clone();
    *old.decor_mut() = decor;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parsed(text: &str) -> toml::Value {
        toml::from_str(text).unwrap()
    }

    /// Merges with the existing text as its own base (every key written out).
    fn update(existing: &str, fresh: &str) -> String {
        update_preserving(existing, existing, fresh).unwrap()
    }

    #[test]
    fn values_change_in_place_and_comments_stay() {
        let existing =
            "# Top comment\n\nname = 'old' # trailing\n\n[server]\n# the port\nport = 0x50\nhosts = [ \"a\",\n  \"b\" ]  # kept layout\n";
        let fresh = "name = \"new\"\n\n[server]\nport = 80\nhosts = [\"a\", \"b\"]\n";
        let out = update(existing, fresh);
        assert_eq!(
            out,
            "# Top comment\n\nname = \"new\" # trailing\n\n[server]\n# the port\nport = 0x50\nhosts = [ \"a\",\n  \"b\" ]  # kept layout\n"
        );
        assert_eq!(parsed(&out), parsed(fresh));
        // Without a base, unchanged values still keep their spelling.
        assert_eq!(update_preserving(existing, "", fresh).unwrap(), out);
    }

    #[test]
    fn keys_and_tables_are_added_and_removed() {
        let existing = "a = 1 # one\nb = 2\n\n[t]\nx = 1\n\n[[list]]\nn = 1 # first\n\n[[list]]\nn = 2\n\n[[list]]\nn = 3\n";
        let fresh =
            "a = 1\nc = 3\n\n[t]\nx = 2\ny = { p = 1 }\n\n[t.deep]\nq = 1\n\n[u]\nz = true\n\n[[list]]\nn = 1\n\n[[list]]\nn = 20\n";
        let out = update(existing, fresh);
        assert_eq!(parsed(&out), parsed(fresh), "{out}");
        assert!(out.contains("a = 1 # one") && out.contains("n = 1 # first"), "{out}");
        assert!(!out.contains("b = 2"), "{out}");
        // More entries than before are appended, with their sub-tables in the right place.
        let longer = "a = 1\n\n[[list]]\nn = 1\n\n[[list]]\nn = 2\n\n[[list]]\nn = 3\n\n[[list]]\nn = 4\n[list.sub]\nm = 1\n";
        let out = update(existing, longer);
        assert_eq!(parsed(&out), parsed(longer), "{out}");
        assert!(out.contains("n = 1 # first"), "{out}");
    }

    #[test]
    fn defaults_left_out_stay_out() {
        // The base spells out a default the file omits; an unchanged default is not added.
        let existing = "# mine\nname = \"x\"\n";
        let base = "name = \"x\"\nenabled = true\n[opts]\nlevel = 1\n";
        assert_eq!(update_preserving(existing, base, base).unwrap(), existing);
        // A changed default is written.
        let fresh = "name = \"x\"\nenabled = false\n[opts]\nlevel = 1\n";
        let out = update_preserving(existing, base, fresh).unwrap();
        assert!(
            out.starts_with("# mine\n") && out.contains("enabled = false") && !out.contains("level"),
            "{out}"
        );
    }

    #[test]
    fn inline_tables_stay_inline() {
        let existing = "[[a]]\nkind = 'c'\nplan = { name = \"n\", days = 1 } # inline\n";
        let base = "[[a]]\nkind = \"c\"\n[a.plan]\nname = \"n\"\ndays = 1\nmsinfo = false\nsub = { x = 1 }\n";
        let fresh = "[[a]]\nkind = \"c\"\n[a.plan]\nname = \"m\"\ndays = 1\nmsinfo = true\nsub = { x = 2 }\n";
        let out = update_preserving(existing, base, fresh).unwrap();
        assert_eq!(parsed(&out), parsed(fresh), "{out}");
        assert!(out.contains("# inline") && out.contains("plan = {"), "{out}");
        // Inline in both: merged key by key; nested inline tables too.
        let existing = "p = { a = 1, b = 'x', gone = true, n = { k = 1, j = 2 } } # inline\n";
        let fresh = "p = { a = 2, b = \"x\", c = [1], n = { k = 3, j = 2 } }\n";
        let out = update(existing, fresh);
        assert_eq!(parsed(&out), parsed(fresh), "{out}");
        assert!(out.contains("b = 'x'") && out.contains("# inline"), "{out}");
    }

    #[test]
    fn changed_kinds_are_written_anew() {
        let existing = "q = 1\nr = [1, 2]\n[s]\nx = 1\n";
        let fresh = "r = [1, 3]\ns = 2\n[q]\na = 1\n";
        let out = update(existing, fresh);
        assert_eq!(parsed(&out), parsed(fresh), "{out}");
        assert_eq!(update(existing, existing), existing, "nothing changed, nothing rewritten");
    }

    #[test]
    fn invalid_toml_is_an_error() {
        assert!(update_preserving("a = ", "", "a = 1").unwrap_err().contains("existing file"));
        assert!(update_preserving("a = 1", "a = ", "a = 1").unwrap_err().starts_with("invalid TOML"));
        assert!(update_preserving("a = 1", "", "a = ").unwrap_err().starts_with("invalid TOML"));
    }
}
