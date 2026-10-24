# SQL views for the assistant

The assistant's `run_sql` tool is a guarded fallback for questions the query language cannot express (joins across sources, window functions, percentiles of gaps between events). It runs through the Store pane's SQL console, so the console's guardrails apply unchanged (ADR 0030): exactly one read-only statement from an allow-list, no file, network or extension functions, a 30 s time limit, a 10,000 row cap (the model sees at most 50 rows) and a memory limit. The console's database holds only the views below; the store's internal tables are not in it. The `notes` view exists in the console but stays empty for the assistant.

The tool description the model reads is generated from the same table (`debrief_core::ports::SQL_VIEW_TABLE`), and a test (`crates/debrief-ai/tests/it/sql_tool.rs`) fails when this page and the table disagree. Columns are stable: added columns keep the version (`SQL_VIEWS_VERSION`), removing or re-typing one bumps it.

## events

Every event of the store's sources, in no particular order (add ORDER BY ts).

| Column | Type | Meaning |
| --- | --- | --- |
| `id` | `UBIGINT` | The event's id (stable for the store; "Open in Events" uses it). |
| `ts` | `TIMESTAMP` | Event time (UTC); NULL for untimed events. |
| `level` | `VARCHAR` | Verbose, Debug, Information, Warning, Error or Fatal. |
| `source` | `UINTEGER` | The source's id (join sources.id). |
| `line` | `UINTEGER` | Line number in the source. |
| `byte_offset` | `UBIGINT` | Byte offset of the event in the source. |
| `message` | `VARCHAR` | The rendered message. |
| `template` | `VARCHAR` | The message template, when the format has one. |
| `exception` | `VARCHAR` | The exception text, if any. |
| `fields` | `MAP(VARCHAR, VARCHAR)` | Every other field as text: fields['RequestId']. |

## sources

The sources (files and archive members) in the store.

| Column | Type | Meaning |
| --- | --- | --- |
| `id` | `UINTEGER` | The source's id. |
| `name` | `VARCHAR` | File name (archive members: the member's name). |
| `path` | `VARCHAR` | Full path. |
| `importer` | `VARCHAR` | The importer that read it. |
| `parser` | `VARCHAR` | The parser that read it. |
| `events` | `UBIGINT` | Events read. |
| `skipped` | `UBIGINT` | Lines skipped. |
| `first_ts` | `TIMESTAMP` | Earliest event time. |
| `last_ts` | `TIMESTAMP` | Latest event time. |
| `bytes` | `UBIGINT` | Bytes read. |

## fields

The fields the events carry, with how they are stored.

| Column | Type | Meaning |
| --- | --- | --- |
| `key` | `VARCHAR` | The field's name (events.fields[key]). |
| `type` | `VARCHAR` | text, integer, number or boolean. |
| `storage` | `VARCHAR` | declared, promoted, partly or overflow (how fast filters on it are). |
| `events` | `UBIGINT` | Events that have it. |
| `fill_rate` | `DOUBLE` | events over all events, 0 to 1. |
| `approx_distinct` | `UBIGINT` | Approximate distinct values (at least this many). |

## When the assistant uses SQL

Only when `run_query` cannot say it. A level, field or name filter goes through `find_values` and `run_query`. When SQL is used the chat says so and shows the statement in the same chip as query-language queries.
