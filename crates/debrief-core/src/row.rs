//! One row interface (redesign R2): [`RowRef`] reads an event the same way whether it is an in-memory
//! [`Record`] or a row of an Arrow [`EventBatch`](crate::batch::EventBatch), so conditions, grouping,
//! extractors and analyses can move from `&Record` to batches one at a time.
//!
//! Field names are interned once per query or rule ([`FieldKeys`]); a batch resolves each interned key to
//! its declared column or the `extra` map once ([`BatchSlots`]), not per row.

use crate::batch::{BatchRow, EventSchema, FieldValue};
use crate::model::{value_number, value_text, Fields, Level, MarkerMask, Record};
use serde_json::Value;
use std::borrow::Cow;
use std::collections::HashMap;
use std::sync::Arc;

/// A built-in pseudo-field.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Pseudo {
    Level,
    Message,
    Exception,
    Template,
    Source,
    Line,
    /// The event's id (`@id`): lets a query select a set of events, e.g. the rows of a SQL console result.
    Id,
}

impl Pseudo {
    pub fn parse(name: &str) -> Option<Pseudo> {
        Some(match name {
            "@level" => Pseudo::Level,
            "@message" => Pseudo::Message,
            "@exception" => Pseudo::Exception,
            "@template" => Pseudo::Template,
            "@source" => Pseudo::Source,
            "@line" => Pseudo::Line,
            "@id" => Pseudo::Id,
            _ => return None,
        })
    }
}

/// An interned field name: its index in a [`FieldKeys`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Sym(pub u32);

/// A field a row can be asked for.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FieldKey {
    Pseudo(Pseudo),
    Named { sym: Sym, name: Arc<str> },
}

impl FieldKey {
    pub fn name(&self) -> &str {
        match self {
            FieldKey::Pseudo(p) => match p {
                Pseudo::Level => "@level",
                Pseudo::Message => "@message",
                Pseudo::Exception => "@exception",
                Pseudo::Template => "@template",
                Pseudo::Source => "@source",
                Pseudo::Line => "@line",
                Pseudo::Id => "@id",
            },
            FieldKey::Named { name, .. } => name,
        }
    }
}

/// Interns field names into [`FieldKey`]s.
#[derive(Debug, Default, Clone)]
pub struct FieldKeys {
    names: Vec<Arc<str>>,
    index: HashMap<Arc<str>, Sym>,
}

impl FieldKeys {
    pub fn new() -> FieldKeys {
        FieldKeys::default()
    }

    /// The key of `name`: a pseudo-field, or the named field (interned on first use).
    pub fn key(&mut self, name: &str) -> FieldKey {
        if let Some(p) = Pseudo::parse(name) {
            return FieldKey::Pseudo(p);
        }
        if let Some((n, sym)) = self.index.get_key_value(name) {
            return FieldKey::Named {
                sym: *sym,
                name: n.clone(),
            };
        }
        let sym = Sym(self.names.len() as u32);
        let name: Arc<str> = Arc::from(name);
        self.names.push(name.clone());
        self.index.insert(name.clone(), sym);
        FieldKey::Named { sym, name }
    }

    /// Interned names, by symbol.
    pub fn names(&self) -> &[Arc<str>] {
        &self.names
    }
}

/// Read access to one event.
pub trait RowRef {
    /// The event's id (a [`Record`]'s position in the in-memory store, an `EventId` in the database).
    fn id(&self) -> u64;
    fn ts(&self) -> Option<i64>;
    fn level(&self) -> Level;
    fn source(&self) -> u32;
    /// The source's display name (empty for a bare [`Record`]).
    fn source_name(&self) -> &str;
    fn line(&self) -> u32;
    fn markers(&self) -> MarkerMask;
    fn message(&self) -> &str;
    fn template(&self) -> Option<&str>;
    fn exception(&self) -> Option<&str>;
    /// A named (non-pseudo) field as text.
    fn named_text(&self, sym: Sym, name: &str) -> Option<Cow<'_, str>>;
    /// A named field as a number (numbers, numeric text, booleans as 1/0).
    fn named_number(&self, sym: Sym, name: &str) -> Option<f64>;
    /// A named field as a JSON value.
    fn named_value(&self, sym: Sym, name: &str) -> Option<Value>;
    /// Every named field with its value.
    fn for_each_field(&self, f: &mut dyn FnMut(&str, Cow<'_, Value>));

    /// A field or pseudo-field as text (as [`Record::field_text`]).
    fn text(&self, key: &FieldKey) -> Option<Cow<'_, str>> {
        match key {
            FieldKey::Pseudo(p) => match p {
                Pseudo::Level => Some(Cow::Borrowed(self.level().as_str())),
                Pseudo::Message => Some(Cow::Borrowed(self.message())),
                Pseudo::Exception => self.exception().map(Cow::Borrowed),
                Pseudo::Template => self.template().map(Cow::Borrowed),
                Pseudo::Source => Some(Cow::Borrowed(self.source_name())),
                Pseudo::Line => Some(Cow::Owned(self.line().to_string())),
                Pseudo::Id => Some(Cow::Owned(self.id().to_string())),
            },
            FieldKey::Named { sym, name } => self.named_text(*sym, name),
        }
    }

    /// A field as a number (as [`Record::field_number`]).
    fn number(&self, key: &FieldKey) -> Option<f64> {
        match key {
            FieldKey::Pseudo(Pseudo::Line) => Some(self.line() as f64),
            FieldKey::Pseudo(Pseudo::Id) => Some(self.id() as f64),
            FieldKey::Pseudo(_) => None,
            FieldKey::Named { sym, name } => self.named_number(*sym, name),
        }
    }

    /// A field as a JSON value: `@line` and `@id` are numbers, the other pseudo-fields text.
    fn value(&self, key: &FieldKey) -> Option<Value> {
        match key {
            FieldKey::Pseudo(Pseudo::Line) => Some(Value::from(self.line())),
            FieldKey::Pseudo(Pseudo::Id) => Some(Value::from(self.id())),
            FieldKey::Pseudo(_) => self.text(key).map(|t| Value::String(t.into_owned())),
            FieldKey::Named { sym, name } => self.named_value(*sym, name),
        }
    }

    /// A field by name, as text (for code that does not intern its names).
    fn text_of(&self, name: &str) -> Option<Cow<'_, str>> {
        match Pseudo::parse(name) {
            Some(p) => self.text(&FieldKey::Pseudo(p)),
            None => self.named_text(Sym(u32::MAX), name),
        }
    }

    /// A field by name, as a number.
    fn number_of(&self, name: &str) -> Option<f64> {
        match Pseudo::parse(name) {
            Some(p) => self.number(&FieldKey::Pseudo(p)),
            None => self.named_number(Sym(u32::MAX), name),
        }
    }
}

/// A bare record (its source name is unknown, so `@source` is empty).
impl RowRef for Record {
    fn id(&self) -> u64 {
        u64::from(self.id)
    }
    fn ts(&self) -> Option<i64> {
        self.ts
    }
    fn level(&self) -> Level {
        self.level
    }
    fn source(&self) -> u32 {
        self.source
    }
    fn source_name(&self) -> &str {
        ""
    }
    fn line(&self) -> u32 {
        self.line
    }
    fn markers(&self) -> MarkerMask {
        self.markers
    }
    fn message(&self) -> &str {
        &self.message
    }
    fn template(&self) -> Option<&str> {
        self.template.as_deref()
    }
    fn exception(&self) -> Option<&str> {
        self.exception.as_deref()
    }
    fn named_text(&self, _sym: Sym, name: &str) -> Option<Cow<'_, str>> {
        self.fields.get(name).map(value_text)
    }
    fn named_number(&self, _sym: Sym, name: &str) -> Option<f64> {
        self.fields.get(name).and_then(value_number)
    }
    fn named_value(&self, _sym: Sym, name: &str) -> Option<Value> {
        self.fields.get(name).cloned()
    }
    fn for_each_field(&self, f: &mut dyn FnMut(&str, Cow<'_, Value>)) {
        for (k, v) in &self.fields {
            f(k, Cow::Borrowed(v));
        }
    }
}

/// Forwards every method but `id` and `source_name` to the row `$e` (an expression of `$s`, which is
/// `self`).
macro_rules! forward_row {
    ($s:ident => $e:expr) => {
        fn ts(&self) -> Option<i64> {
            let $s = self;
            $e.ts()
        }
        fn level(&self) -> Level {
            let $s = self;
            $e.level()
        }
        fn source(&self) -> u32 {
            let $s = self;
            $e.source()
        }
        fn line(&self) -> u32 {
            let $s = self;
            $e.line()
        }
        fn markers(&self) -> MarkerMask {
            let $s = self;
            $e.markers()
        }
        fn message(&self) -> &str {
            let $s = self;
            $e.message()
        }
        fn template(&self) -> Option<&str> {
            let $s = self;
            $e.template()
        }
        fn exception(&self) -> Option<&str> {
            let $s = self;
            $e.exception()
        }
        fn named_text(&self, sym: Sym, name: &str) -> Option<Cow<'_, str>> {
            let $s = self;
            $e.named_text(sym, name)
        }
        fn named_number(&self, sym: Sym, name: &str) -> Option<f64> {
            let $s = self;
            $e.named_number(sym, name)
        }
        fn named_value(&self, sym: Sym, name: &str) -> Option<Value> {
            let $s = self;
            $e.named_value(sym, name)
        }
        fn for_each_field(&self, f: &mut dyn FnMut(&str, Cow<'_, Value>)) {
            let $s = self;
            $e.for_each_field(f)
        }
    };
}

/// A [`Record`] with its source name.
#[derive(Debug, Clone, Copy)]
pub struct RecordRow<'a> {
    pub rec: &'a Record,
    pub source: &'a str,
}

impl RowRef for RecordRow<'_> {
    fn id(&self) -> u64 {
        u64::from(self.rec.id)
    }
    fn source_name(&self) -> &str {
        self.source
    }
    forward_row!(s => s.rec);
}

/// An owned event: a [`Record`], its store id and its source name, for code that streams events out
/// of a store (a scan yields these one at a time; nothing borrows the store).
#[derive(Debug, Clone)]
pub struct OwnedRow {
    pub id: u64,
    pub rec: Record,
    pub source: Arc<str>,
}

impl RowRef for OwnedRow {
    fn id(&self) -> u64 {
        self.id
    }
    fn source_name(&self) -> &str {
        &self.source
    }
    forward_row!(s => s.rec);
}

impl<T: RowRef + ?Sized> RowRef for &T {
    fn id(&self) -> u64 {
        (**self).id()
    }
    fn source_name(&self) -> &str {
        (**self).source_name()
    }
    forward_row!(s => (**s));
}

/// Where an interned field lives in one batch schema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Slot {
    Declared(usize),
    Extra,
}

/// Interned keys resolved against a batch schema. Each slot remembers its name, so a key interned by
/// other [`FieldKeys`] (whose symbol means another field here) is still read correctly, by name.
#[derive(Debug, Clone, Default)]
pub struct BatchSlots {
    slots: Vec<(Arc<str>, Slot)>,
}

impl BatchSlots {
    pub fn resolve(keys: &FieldKeys, schema: &EventSchema) -> BatchSlots {
        let slots = keys
            .names()
            .iter()
            .map(|n| (n.clone(), schema.declared_index(n).map_or(Slot::Extra, Slot::Declared)))
            .collect();
        BatchSlots { slots }
    }
}

/// A row of an event batch with its source name and resolved keys.
#[derive(Debug, Clone, Copy)]
pub struct ArrowRow<'a> {
    pub row: BatchRow<'a>,
    pub source: &'a str,
    pub slots: &'a BatchSlots,
}

impl<'a> ArrowRow<'a> {
    fn value(&self, sym: Sym, name: &str) -> Option<FieldValue<'a>> {
        match self.slots.slots.get(sym.0 as usize) {
            Some((n, Slot::Declared(i))) if &**n == name => self.row.declared(*i).or_else(|| self.row.extra(name).map(FieldValue::Text)),
            Some((n, Slot::Extra)) if &**n == name => self.row.extra(name).map(FieldValue::Text),
            // A key interned after the slots were resolved, or by other keys.
            _ => self.row.field(name),
        }
    }
}

/// Text of a stored value, rendered as [`value_text`] renders the JSON value it came from.
pub fn field_text(v: FieldValue<'_>) -> Cow<'_, str> {
    match v {
        FieldValue::Text(s) => Cow::Borrowed(s),
        other => Cow::Owned(value_text(&json_value(other)).into_owned()),
    }
}

/// Number of a stored value, as [`value_number`] reads the JSON value it came from.
pub fn field_number(v: FieldValue<'_>) -> Option<f64> {
    value_number(&json_value(v))
}

/// The JSON value of a stored field (text stays text).
pub fn json_value(v: FieldValue<'_>) -> Value {
    match v {
        FieldValue::Text(s) => Value::String(s.to_string()),
        FieldValue::Int(i) => Value::from(i),
        FieldValue::Float(f) => Value::from(f),
        FieldValue::Bool(b) => Value::Bool(b),
    }
}

fn level_of(code: u8) -> Level {
    Level::ALL.get(code as usize).copied().unwrap_or(Level::Fatal)
}

impl RowRef for ArrowRow<'_> {
    fn id(&self) -> u64 {
        self.row.id().0
    }
    fn ts(&self) -> Option<i64> {
        self.row.ts()
    }
    fn level(&self) -> Level {
        level_of(self.row.level().get())
    }
    fn source(&self) -> u32 {
        self.row.source().0
    }
    fn source_name(&self) -> &str {
        self.source
    }
    fn line(&self) -> u32 {
        self.row.line()
    }
    fn markers(&self) -> MarkerMask {
        self.row.markers()
    }
    fn message(&self) -> &str {
        self.row.message()
    }
    fn template(&self) -> Option<&str> {
        self.row.template()
    }
    fn exception(&self) -> Option<&str> {
        self.row.exception()
    }
    fn named_text(&self, sym: Sym, name: &str) -> Option<Cow<'_, str>> {
        self.value(sym, name).map(field_text)
    }
    fn named_number(&self, sym: Sym, name: &str) -> Option<f64> {
        self.value(sym, name).and_then(field_number)
    }
    fn named_value(&self, sym: Sym, name: &str) -> Option<Value> {
        self.value(sym, name).map(json_value)
    }
    fn for_each_field(&self, f: &mut dyn FnMut(&str, Cow<'_, Value>)) {
        for (name, v) in self.row.fields() {
            f(name, Cow::Owned(json_value(v)));
        }
    }
}

/// The [`Record`] of a batch row, for code that still reads records. Ids past `u32::MAX` saturate.
pub fn to_record(row: &BatchRow<'_>) -> Record {
    let mut fields = Fields::new();
    for (name, value) in row.fields() {
        fields.insert(name.to_string(), json_value(value));
    }
    Record {
        id: u32::try_from(row.id().0).unwrap_or(u32::MAX),
        ts: row.ts(),
        level: level_of(row.level().get()),
        message: row.message().to_string(),
        template: row.template().map(str::to_string),
        exception: row.exception().map(str::to_string),
        fields,
        source: row.source().0,
        line: row.line(),
        markers: row.markers(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::batch::{ColumnType, EventBatchBuilder, EventId, EventTime, LevelCode, NewEvent, SourceId};
    use serde_json::json;

    fn record() -> Record {
        let mut r = Record::new(12);
        r.ts = Some(1000);
        r.level = Level::Warning;
        r.message = "slow".into();
        r.template = Some("slow {Ms}".into());
        r.markers = 0b10;
        r.source = 3;
        for (k, v) in [
            ("Ms", json!(250.5)),
            ("Count", json!(4)),
            ("Host", json!("WEB01")),
            ("Ok", json!(true)),
            ("Obj", json!({"a": 1})),
        ] {
            r.fields.insert(k.into(), v);
        }
        r
    }

    fn batch_of(r: &Record, schema: EventSchema) -> crate::batch::EventBatch {
        let mut b = EventBatchBuilder::new(Arc::new(schema));
        let ev = NewEvent {
            time: EventTime::parsed(r.ts),
            level: LevelCode::new(r.level.index() as u8).unwrap(),
            source: SourceId(r.source),
            line: r.line,
            byte_offset: 0,
            message: &r.message,
            template: r.template.as_deref(),
            exception: r.exception.as_deref(),
            markers: r.markers,
            raw: "",
        };
        let texts: Vec<(&str, Cow<str>)> = r.fields.iter().map(|(k, v)| (k.as_str(), value_text(v))).collect();
        let typed = [
            ("Ms", FieldValue::Float(250.5)),
            ("Count", FieldValue::Int(4)),
            ("Ok", FieldValue::Bool(true)),
        ];
        let fields = r.fields.keys().map(|k| {
            typed.iter().find(|(n, _)| n == k).map(|(n, v)| (*n, *v)).unwrap_or_else(|| {
                (
                    k.as_str(),
                    FieldValue::Text(texts.iter().find(|(n, _)| n == k).map(|(_, t)| t.as_ref()).unwrap_or("")),
                )
            })
        });
        b.push(&ev, fields).unwrap();
        b.finish().assign_ids(EventId(9)).unwrap()
    }

    #[test]
    fn record_and_arrow_rows_read_alike() {
        let r = record();
        for schema in [
            EventSchema::plain(),
            EventSchema::new([
                ("Ms", ColumnType::Float64),
                ("Count", ColumnType::Int64),
                ("Ok", ColumnType::Boolean),
            ])
            .unwrap(),
        ] {
            let batch = batch_of(&r, schema);
            let mut keys = FieldKeys::new();
            let names = [
                "@level",
                "@message",
                "@exception",
                "@template",
                "@source",
                "@line",
                "Ms",
                "Count",
                "Host",
                "Obj",
                "missing",
            ];
            let interned: Vec<FieldKey> = names.iter().map(|n| keys.key(n)).collect();
            let slots = BatchSlots::resolve(&keys, batch.schema());
            let a = ArrowRow {
                row: batch.row(0),
                source: "app.log",
                slots: &slots,
            };
            let b = RecordRow {
                rec: &r,
                source: "app.log",
            };
            for k in &interned {
                assert_eq!(a.text(k), b.text(k), "{}", k.name());
                assert_eq!(a.number(k), b.number(k), "{}", k.name());
            }
            // `Ok` is a bool: 1.0 as a declared column, text "true" (not numeric) in `extra`.
            let ok = keys.key("Ok");
            assert_eq!(a.text(&ok).as_deref(), Some("true"));
            assert_eq!(
                (a.ts(), a.level(), a.source(), a.line(), a.markers()),
                (Some(1000), Level::Warning, 3, 12, 0b10)
            );
            assert_eq!(
                (b.ts(), b.level(), b.source(), b.line(), b.markers()),
                (Some(1000), Level::Warning, 3, 12, 0b10)
            );
            assert_eq!(a.message(), b.message());
            let back = to_record(&batch.row(0));
            assert_eq!((back.id, back.ts, back.line, back.source), (9, Some(1000), 12, 3));
            assert_eq!(back.template.as_deref(), Some("slow {Ms}"));
            for (k, v) in &r.fields {
                assert_eq!(back.field_text(k, "s"), Some(value_text(v)), "{k}");
            }
        }
    }

    #[test]
    fn every_kind_of_row_reads_the_pseudo_fields_values_and_each_field_alike() {
        let mut r = record();
        r.id = 5;
        let mut keys = FieldKeys::new();
        let all: Vec<FieldKey> = [
            "@level",
            "@message",
            "@exception",
            "@template",
            "@source",
            "@line",
            "@id",
            "Ms",
            "Host",
            "Obj",
            "missing",
        ]
        .iter()
        .map(|n| keys.key(n))
        .collect();
        assert_eq!(keys.key("@id").name(), "@id");
        let record_row = RecordRow {
            rec: &r,
            source: "app.log",
        };
        let owned = OwnedRow {
            id: u64::from(r.id),
            rec: r.clone(),
            source: Arc::from("app.log"),
        };
        let by_ref = &owned;
        let batch = batch_of(&r, EventSchema::plain());
        let slots = BatchSlots::resolve(&keys, batch.schema());
        let arrow = ArrowRow {
            row: batch.row(0),
            source: "app.log",
            slots: &slots,
        };
        // @id is the event's id: a number to compare, text to show.
        let id = FieldKey::Pseudo(Pseudo::Id);
        assert_eq!(record_row.number(&id), Some(f64::from(r.id)));
        assert_eq!(record_row.text(&id).as_deref(), Some("5"));
        assert_eq!(record_row.value(&id), Some(json!(5)));
        assert_eq!(RowRef::value(&arrow, &id), Some(json!(9)), "the arrow row carries the store's id");
        let line = FieldKey::Pseudo(Pseudo::Line);
        assert_eq!(RowRef::value(&arrow, &line), Some(json!(12)));
        assert_eq!(RowRef::value(&arrow, &FieldKey::Pseudo(Pseudo::Source)), Some(json!("app.log")));
        // Rows over the same event answer alike through every wrapper.
        for k in &all {
            assert_eq!(owned.text(k), record_row.text(k), "{}", k.name());
            assert_eq!(by_ref.text(k), record_row.text(k), "{}", k.name());
            assert_eq!(owned.number(k), record_row.number(k), "{}", k.name());
            assert_eq!(by_ref.value(k), record_row.value(k), "{}", k.name());
        }
        assert_eq!((by_ref.id(), by_ref.source_name(), by_ref.line()), (u64::from(r.id), "app.log", 12));
        // A reference to a row is a row (through the blanket impl), and a bare record has no source name.
        assert_eq!(
            (RowRef::id(&by_ref), RowRef::source_name(&by_ref), RowRef::line(&by_ref)),
            (u64::from(r.id), "app.log", 12)
        );
        assert_eq!(
            (RowRef::id(&r), RowRef::source_name(&r), RowRef::source(&r)),
            (u64::from(r.id), "", 3)
        );
        assert_eq!(RowRef::text(&r, &FieldKey::Pseudo(Pseudo::Source)).as_deref(), Some(""));
        // Named values come back from the arrow row; unknown names are None.
        let ms = keys.key("Ms");
        assert_eq!(
            RowRef::value(&arrow, &ms),
            Some(json!("250.5")),
            "undeclared fields are kept as text"
        );
        assert_eq!(arrow.number(&ms), Some(250.5));
        assert_eq!(RowRef::value(&arrow, &keys.key("missing")), None);
        // Every field of the event is visited, with its value.
        let mut seen: Vec<String> = Vec::new();
        arrow.for_each_field(&mut |name, v| seen.push(format!("{name}={v}")));
        seen.sort();
        assert_eq!(seen.len(), r.fields.len(), "{seen:?}");
        assert!(seen.iter().any(|s| s.starts_with("Host=")), "{seen:?}");
        let mut from_record = 0;
        record_row.for_each_field(&mut |_, _| from_record += 1);
        assert_eq!(from_record, r.fields.len());
    }

    #[test]
    fn keys_are_interned_once() {
        let mut keys = FieldKeys::new();
        let a = keys.key("A");
        assert_eq!(keys.key("A"), a);
        assert_ne!(keys.key("B"), a);
        assert_eq!(keys.names().len(), 2);
        assert_eq!(keys.key("@line"), FieldKey::Pseudo(Pseudo::Line));
        assert_eq!(a.name(), "A");
        for p in ["@level", "@message", "@exception", "@template", "@source", "@line"] {
            assert_eq!(keys.key(p).name(), p);
        }
        // A key interned after resolving still reads by name.
        let r = record();
        let batch = batch_of(&r, EventSchema::plain());
        let slots = BatchSlots::resolve(&keys, batch.schema());
        let late = keys.key("Host");
        let row = ArrowRow {
            row: batch.row(0),
            source: "s",
            slots: &slots,
        };
        assert_eq!(row.text(&late).as_deref(), Some("WEB01"));
        assert_eq!(row.number(&FieldKey::Pseudo(Pseudo::Message)), None);
        assert_eq!(field_number(FieldValue::Text(" 2.5 ")), Some(2.5));
        assert_eq!(field_text(FieldValue::Float(2.0)), "2.0");
        assert_eq!(level_of(200), Level::Fatal);
    }
}
