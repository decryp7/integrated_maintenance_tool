//! The console's stable surface (ADR 0030): the `debrief` schema with the views `events`, `sources`, `fields` and
//! `notes`, documented here (the editor's completion and `docs/user-guide.md` list the same columns). Internal
//! tables (`segments`, `segment_fields`, `events_wide`, the promoted `p_*` columns) are not in the console's
//! database at all, so the store's layout can change without breaking saved SQL.

use crate::schema::parquet_from;
use crate::sql::{ident, lit};
use debrief_core::batch::EventSchema;
use debrief_core::ports::{sql_views, SqlView, SQL_VIEW_TABLE};

/// The views as the editor completes them (the table lives in the port: `debrief_core::ports::SQL_VIEW_TABLE`).
pub fn schema() -> Vec<SqlView> {
    sql_views()
}

/// `CREATE TABLE` of the tables behind `sources`, `fields` and `notes` (filled by the console, not by queries).
pub fn tables_ddl() -> String {
    SQL_VIEW_TABLE[1..]
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
