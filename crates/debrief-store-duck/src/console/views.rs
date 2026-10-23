//! The console's stable surface (ADR 0030): the `debrief` schema with the views `events`, `sources`, `fields` and
//! `notes`, documented here (the editor's completion and `docs/user-guide.md` list the same columns). Internal
//! tables (`segments`, `segment_fields`, `events_wide`, the promoted `p_*` columns) are not in the console's
//! database at all, so the store's layout can change without breaking saved SQL.

use crate::schema::parquet_from;
use crate::sql::{ident, lit};
use debrief_core::batch::EventSchema;
use debrief_core::ports::{SqlView, SqlViewColumn};

/// `(name, type, description)` of each view's columns, in order.
type Columns = &'static [(&'static str, &'static str, &'static str)];

const EVENTS: Columns = &[
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

const SOURCES: Columns = &[
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

const FIELDS: Columns = &[
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

const NOTES: Columns = &[
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
const VIEWS: [(&str, &str, Columns); 4] = [
    (
        "events",
        "Every event of the store's sources, in no particular order (add ORDER BY ts).",
        EVENTS,
    ),
    ("sources", "The sources (files and archive members) in the store.", SOURCES),
    ("fields", "The fields the events carry, with how they are stored.", FIELDS),
    ("notes", "The window's notes.", NOTES),
];

/// The views as the editor completes them.
pub fn schema() -> Vec<SqlView> {
    VIEWS
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

/// `CREATE TABLE` of the tables behind `sources`, `fields` and `notes` (filled by the console, not by queries).
pub fn tables_ddl() -> String {
    VIEWS[1..]
        .iter()
        .map(|(name, _, cols)| {
            let cols = cols
                .iter()
                .map(|(n, t, _)| format!("{} {t}", ident(n)))
                .collect::<Vec<_>>()
                .join(", ");
            format!("CREATE OR REPLACE TABLE debrief.{name} ({cols});")
        })
        .collect()
}

/// `CREATE OR REPLACE VIEW debrief.events` over the segment files `files` (absolute paths): the events of the
/// sources in `debrief.sources`, with the declared columns of `schema` folded into `fields`.
pub fn events_view(schema: &EventSchema, files: &[String]) -> String {
    let declared: Vec<String> = schema
        .declared()
        .iter()
        .filter_map(|c| Some(format!("{}: CAST(e.{} AS VARCHAR)", lit(&c.name)?, ident(&c.column))))
        .collect();
    let fields = if declared.is_empty() {
        "e.extra".to_string()
    } else {
        format!("map_concat(MAP {{{}}}, e.extra)", declared.join(", "))
    };
    let level = "CASE e.level WHEN 0 THEN 'Verbose' WHEN 1 THEN 'Debug' WHEN 2 THEN 'Information' WHEN 3 THEN 'Warning' WHEN 4 THEN 'Error' ELSE 'Fatal' END";
    let select = format!(
        "SELECT e.id::UBIGINT AS id, epoch_ms(e.ts) AS ts, {level} AS level, e.source::UINTEGER AS source, e.line::UINTEGER AS line, \
         e.byte_offset::UBIGINT AS byte_offset, e.message, e.template, e.exception, {fields} AS fields"
    );
    if files.is_empty() {
        // The same columns, no rows.
        return "CREATE OR REPLACE VIEW debrief.events AS SELECT NULL::UBIGINT AS id, NULL::TIMESTAMP AS ts, NULL::VARCHAR AS level, \
                NULL::UINTEGER AS source, NULL::UINTEGER AS line, NULL::UBIGINT AS byte_offset, NULL::VARCHAR AS message, \
                NULL::VARCHAR AS template, NULL::VARCHAR AS exception, NULL::MAP(VARCHAR, VARCHAR) AS fields WHERE false;"
            .to_string();
    }
    format!(
        "CREATE OR REPLACE VIEW debrief.events AS {select} FROM {} e WHERE e.source IN (SELECT id FROM debrief.sources);",
        parquet_from(files)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use debrief_core::batch::ColumnType;

    #[test]
    fn the_views_are_documented_and_built_from_the_segments() {
        let s = schema();
        assert_eq!(
            s.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
            ["events", "sources", "fields", "notes"]
        );
        assert!(s
            .iter()
            .all(|v| !v.description.is_empty() && v.columns.iter().all(|c| !c.description.is_empty())));
        let ddl = tables_ddl();
        assert!(
            ddl.contains("CREATE OR REPLACE TABLE debrief.sources") && ddl.contains("\"tags\" VARCHAR[]"),
            "{ddl}"
        );
        assert!(!ddl.contains("debrief.events"));
        let plain = EventSchema::plain();
        assert!(events_view(&plain, &[]).contains("WHERE false"));
        let v = events_view(&plain, &["C:/s/a.parquet".into()]);
        assert!(
            v.contains("e.extra AS fields") && v.contains("read_parquet(['C:/s/a.parquet']"),
            "{v}"
        );
        let typed = EventSchema::new([("Elapsed", ColumnType::Float64)]).unwrap();
        let v = events_view(&typed, &["a".into()]);
        assert!(
            v.contains("map_concat(MAP {'Elapsed': CAST(e.\"f_Elapsed\" AS VARCHAR)}, e.extra)"),
            "{v}"
        );
    }
}
