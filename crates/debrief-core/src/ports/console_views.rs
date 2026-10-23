//! The ONE table of the SQL views (ADR 0030): name, description and `(column, type, description)` of each. The store
//! adapter builds its sandbox tables and editor schema from it, the assistant's `run_sql` tool description is generated
//! from it ([`ai_views_text`]) and a test keeps `docs/ai/sql-views.md` in step with it. Adding or changing a column
//! happens here and nowhere else.

use super::{SqlView, SqlViewColumn};

/// `(name, type, description)` of each view's columns, in order.
pub type Columns = &'static [(&'static str, &'static str, &'static str)];

pub const EVENTS: Columns = &[
    (
        "id",
        "UBIGINT",
        "The event's id (stable for the store; \"Open in Events\" uses it).",
    ),
    ("ts", "TIMESTAMP", "Event time (UTC); NULL for untimed events."),
    ("level", "VARCHAR", "Verbose, Debug, Information, Warning, Error or Fatal."),
    ("source", "UINTEGER", "The source's id (join sources.id)."),
    ("line", "UINTEGER", "Line number in the source."),
    ("byte_offset", "UBIGINT", "Byte offset of the event in the source."),
    ("message", "VARCHAR", "The rendered message."),
    ("template", "VARCHAR", "The message template, when the format has one."),
    ("exception", "VARCHAR", "The exception text, if any."),
    ("fields", "MAP(VARCHAR, VARCHAR)", "Every other field as text: fields['RequestId']."),
];

pub const SOURCES: Columns = &[
    ("id", "UINTEGER", "The source's id."),
    ("name", "VARCHAR", "File name (archive members: the member's name)."),
    ("path", "VARCHAR", "Full path."),
    ("importer", "VARCHAR", "The importer that read it."),
    ("parser", "VARCHAR", "The parser that read it."),
    ("events", "UBIGINT", "Events read."),
    ("skipped", "UBIGINT", "Lines skipped."),
    ("first_ts", "TIMESTAMP", "Earliest event time."),
    ("last_ts", "TIMESTAMP", "Latest event time."),
    ("bytes", "UBIGINT", "Bytes read."),
];

pub const FIELDS: Columns = &[
    ("key", "VARCHAR", "The field's name (events.fields[key])."),
    ("type", "VARCHAR", "text, integer, number or boolean."),
    (
        "storage",
        "VARCHAR",
        "declared, promoted, partly or overflow (how fast filters on it are).",
    ),
    ("events", "UBIGINT", "Events that have it."),
    ("fill_rate", "DOUBLE", "events over all events, 0 to 1."),
    ("approx_distinct", "UBIGINT", "Approximate distinct values (at least this many)."),
];

pub const NOTES: Columns = &[
    ("id", "VARCHAR", "The note's id."),
    (
        "anchor",
        "VARCHAR",
        "event, range, instant, source, marker, chartPoint, project or image.",
    ),
    ("event_id", "UBIGINT", "The noted event (events.id), for event notes."),
    ("source_id", "UINTEGER", "The noted source (sources.id), for source notes."),
    (
        "from_ts",
        "TIMESTAMP",
        "Start of the noted time (event, instant, range, marker, chart point).",
    ),
    ("to_ts", "TIMESTAMP", "End of the noted range."),
    ("severity", "VARCHAR", "info, warning or problem."),
    ("tags", "VARCHAR[]", "Its tags."),
    ("text", "VARCHAR", "The Markdown text (at most 2,048 characters)."),
    ("author", "VARCHAR", "Who wrote it."),
    ("created", "TIMESTAMP", "When it was written."),
    ("updated", "TIMESTAMP", "When it was last changed."),
];

/// The views with their descriptions.
pub const SQL_VIEW_TABLE: [(&str, &str, Columns); 4] = [
    (
        "events",
        "Every event of the store's sources, in no particular order (add ORDER BY ts).",
        EVENTS,
    ),
    ("sources", "The sources (files and archive members) in the store.", SOURCES),
    ("fields", "The fields the events carry, with how they are stored.", FIELDS),
    ("notes", "The window's notes.", NOTES),
];

/// The views the assistant's `run_sql` tool may name: the notes are the user's own and never go to the model.
pub const AI_VIEWS: [&str; 3] = ["events", "sources", "fields"];

/// The views as the editor completes them.
pub fn sql_views() -> Vec<SqlView> {
    SQL_VIEW_TABLE
        .iter()
        .map(|(name, description, cols)| SqlView {
            name: name.to_string(),
            description: description.to_string(),
            columns: cols
                .iter()
                .map(|(n, t, d)| SqlViewColumn {
                    name: n.to_string(),
                    ty: t.to_string(),
                    description: d.to_string(),
                })
                .collect(),
        })
        .collect()
}

/// The [`AI_VIEWS`] as text for the model's tool description: `events(id UBIGINT, ts TIMESTAMP, ...)` with a line per column.
pub fn ai_views_text() -> String {
    let mut out = String::new();
    for (name, description, cols) in SQL_VIEW_TABLE.iter().filter(|(n, _, _)| AI_VIEWS.contains(n)) {
        out.push_str(&format!("{name}: {description}\n"));
        for (c, t, d) in cols.iter() {
            out.push_str(&format!("  {c} {t} - {d}\n"));
        }
    }
    out
}
