//! The Store pane (ADR 0030): the reports about the store, its maintenance, and the read-only SQL console with
//! every refusal and limit triggered.

use crate::common;

use common::{samples, store_with, TempDir};
use debrief_core::ports::{
    Console, FieldStorage, Ingest, Inspect, Maintain, MaintenanceAction, PageQuery, Query, SegmentsRequest, SqlExportRequest,
    SqlExportResult, SqlIdsRequest, SqlNote, SqlPageRequest, SqlRequest, SqlResult, StoreActivity, StoreError, StoreResult,
    IMPORT_BUDGET_BYTES, QUERY_BUDGET_BYTES, SQL_MAX_ROWS, SQL_PAGE_ROWS,
};
use debrief_core::ql::QueryControl;
use debrief_core::{PluginRegistry, Profile};
use debrief_store_duck::DuckStore;
use std::time::Duration;

fn opened(tag: &str) -> (TempDir, DuckStore, u64) {
    let dir = TempDir::new(tag);
    let (store, report) = store_with(&dir, Profile::default_profile(), &[samples().join("serilog-clef")]).unwrap();
    (dir, store, report.events as u64)
}

fn run(store: &DuckStore, sql: &str) -> StoreResult<SqlResult> {
    store.sql_run(&SqlRequest {
        sql: sql.into(),
        ..SqlRequest::default()
    })
}

/// The editor's error for `sql` (a refusal or an invalid query).
fn refused(store: &DuckStore, sql: &str) -> debrief_core::ql::QlError {
    match run(store, sql) {
        Err(StoreError::Query(q)) => q,
        other => panic!("{sql}: {other:?}"),
    }
}

#[test]
fn the_reports_describe_the_store() {
    let (dir, store, events) = opened("pane-reports");
    store.wait_for_index(Duration::from_secs(60)).unwrap();
    let o = store.store_overview().unwrap();
    assert_eq!(o.events, events);
    assert!(o.sources >= 1 && o.segments >= 1 && o.size_bytes > 0, "{o:?}");
    assert_eq!(std::fs::canonicalize(&o.dir).unwrap(), std::fs::canonicalize(&dir.0).unwrap());
    assert_eq!(o.activity, StoreActivity::Ready);
    assert_eq!(o.format_version, debrief_store_duck::FORMAT_VERSION);
    let last = o.last_import.expect("the import is recorded");
    assert_eq!(last.events, events);
    assert!(last.finished_ms > 0);

    let page = store.store_segments(&SegmentsRequest::default()).unwrap();
    assert_eq!(page.total, o.segments);
    let rows: u64 = page.rows.iter().map(|s| s.rows).sum();
    assert_eq!(rows, events, "every event is in a segment");
    for s in &page.rows {
        assert!(
            s.size_bytes > 0 && s.indexed && s.source_count >= 1 && !s.sources.is_empty(),
            "{s:?}"
        );
        assert!(s.compression.to_uppercase().contains("ZSTD"), "{s:?}");
        assert!(s.from.is_some() && s.from <= s.to, "{s:?}");
        assert_eq!(s.live, s.rows);
    }
    let past = store.store_segments(&SegmentsRequest { offset: 10_000, limit: 5 }).unwrap();
    assert!(past.rows.is_empty() && past.total == o.segments);

    let f = store.store_fields().unwrap();
    assert_eq!(f.events, events);
    assert!(!f.fields.is_empty() && !f.truncated);
    for field in &f.fields {
        assert!(field.events > 0 && field.fill_rate > 0.0 && field.fill_rate <= 1.0, "{field:?}");
        assert!(field.segments >= 1 && field.promoted_segments <= field.segments, "{field:?}");
        match field.storage {
            FieldStorage::Promoted => assert_eq!(field.promoted_segments, field.segments),
            FieldStorage::Partly => assert!(field.promoted_segments > 0 && field.promoted_segments < field.segments),
            FieldStorage::Overflow => assert_eq!(field.promoted_segments, 0),
            FieldStorage::Declared => {}
        }
    }
    assert!(f.fields.windows(2).all(|w| w[0].events >= w[1].events), "most common first");

    let i = store.store_index().unwrap();
    assert_eq!((i.docs, i.events), (events, events));
    assert!(!i.catching_up && i.pending_segments == 0 && i.error.is_none(), "{i:?}");
    assert!(
        i.size_bytes > 0 && i.index_segments >= 1 && i.writer_heap_bytes > 0 && i.last_commit_ms.is_some(),
        "{i:?}"
    );

    let m = store.store_memory().unwrap();
    assert_eq!(
        (m.import_budget_bytes, m.query_budget_bytes),
        (IMPORT_BUDGET_BYTES, QUERY_BUDGET_BYTES)
    );
    assert_eq!(m.duck_limit, debrief_store_duck::MEMORY_LIMIT);
    assert_eq!(m.duck_limit_bytes, 192_000_000);
    assert!(m.console_limit_bytes < m.query_budget_bytes);
    assert!(m.worker.is_none() && m.running_jobs.is_empty() && !m.importing);
}

#[test]
fn maintenance_compacts_reindexes_and_clears_the_cache() {
    let (_dir, store, events) = opened("pane-maintain");
    store.wait_for_index(Duration::from_secs(60)).unwrap();
    // A cached analysis to clear.
    let req = debrief_core::api::QueryRequest {
        query: debrief_core::ports::Filter::default(),
    };
    debrief_core::ports::Analysis::anomalies(&store, &req).unwrap();
    assert!(store.store_memory().unwrap().cached_analyses >= 1);
    let cleared = store.maintain(MaintenanceAction::ClearCachedAnalyses).unwrap();
    assert!(cleared.message.starts_with("Cleared"), "{cleared:?}");
    assert_eq!(store.store_memory().unwrap().cached_analyses, 0);

    let before = store.store_overview().unwrap();
    let compacted = store.maintain(MaintenanceAction::Compact).unwrap();
    assert_eq!(compacted.action, MaintenanceAction::Compact);
    let after = store.store_overview().unwrap();
    assert_eq!(after.events, events, "compaction keeps every event");
    assert!(after.segments <= before.segments);
    if before.segments > 1 {
        assert!(compacted.message.starts_with("Compacted"), "{compacted:?}");
        assert!(after.data_version > before.data_version);
    }
    let page = store
        .page(&PageQuery {
            limit: 3,
            ..PageQuery::default()
        })
        .unwrap();
    assert_eq!(page.rows.len(), 3, "the store reads after compaction");

    let rebuilt = store.maintain(MaintenanceAction::RebuildIndex).unwrap();
    assert!(rebuilt.message.contains("again"), "{rebuilt:?}");
    let status = store.wait_for_index(Duration::from_secs(60)).unwrap();
    assert!(status.ready && status.indexed == events, "{status:?}");
}

#[test]
fn maintenance_waits_for_imports() {
    let logs = TempDir::new("pane-busy-logs");
    let mut text = String::new();
    for i in 0..200_000 {
        text.push_str(&format!("2026-10-14 10:{:02}:{:02} INFO line {i}\n", (i / 60) % 60, i % 60));
    }
    std::fs::write(logs.0.join("big.log"), text).unwrap();
    let dir = TempDir::new("pane-busy");
    let store = DuckStore::create(&dir.0, Profile::default_profile(), PluginRegistry::new()).unwrap();
    let session = store.open(std::slice::from_ref(&logs.0)).unwrap();
    let busy = store.maintain(MaintenanceAction::Compact);
    let overview = store.store_overview().unwrap();
    session.wait().unwrap();
    match busy {
        Err(StoreError::Busy(m)) => {
            assert!(m.contains("import"), "{m}");
            assert_eq!(overview.activity, StoreActivity::Importing);
        }
        // The import finished before the call (a fast machine): then compaction ran.
        Ok(r) => assert_eq!(r.action, MaintenanceAction::Compact),
        Err(e) => panic!("{e}"),
    }
}

#[test]
fn the_console_answers_from_the_documented_views() {
    let (_dir, store, events) = opened("pane-sql");
    let n = run(&store, "SELECT count(*) AS n FROM events").unwrap();
    assert_eq!(n.columns[0].name, "n");
    assert_eq!(n.rows, vec![vec![serde_json::json!(events)]]);
    assert!(!n.event_ids && !n.truncated && n.plan.is_none());

    let first = store
        .page(&PageQuery {
            limit: 5,
            ..PageQuery::default()
        })
        .unwrap();
    let r = run(&store, "SELECT id, ts, level, message FROM debrief.events ORDER BY ts, id LIMIT 5;").unwrap();
    assert!(r.event_ids, "the rows carry event ids");
    assert_eq!(
        r.columns.iter().map(|c| c.ty.as_str()).collect::<Vec<_>>(),
        ["UBIGINT", "TIMESTAMP", "VARCHAR", "VARCHAR"]
    );
    let ids: Vec<u64> = r.rows.iter().map(|row| row[0].as_u64().unwrap()).collect();
    // The ids are the store's: the grid shows the same events (ties in time may come in another order).
    let views = store
        .events(&debrief_core::ports::EventsRequest {
            ids: ids.iter().map(|i| debrief_core::batch::EventId(*i)).collect(),
            group_by: vec![],
        })
        .unwrap();
    assert_eq!(
        views.iter().map(|v| v.message.as_str()).collect::<Vec<_>>(),
        r.rows.iter().map(|row| row[3].as_str().unwrap()).collect::<Vec<_>>()
    );
    assert_eq!(views[0].ts, first.rows[0].ts, "the earliest event first");
    assert!(
        r.rows[0][1].as_str().unwrap().starts_with("20"),
        "timestamps as text: {:?}",
        r.rows[0][1]
    );

    let fields = run(&store, "SELECT key, storage, type FROM fields ORDER BY events DESC").unwrap();
    assert!(!fields.rows.is_empty());
    let key = fields.rows[0][0].as_str().unwrap().to_string();
    let with = run(&store, &format!("SELECT count(*) FROM events WHERE fields['{key}'] IS NOT NULL")).unwrap();
    assert!(with.rows[0][0].as_u64().unwrap() > 0, "fields[...] reads the event's fields");
    let sources = run(&store, "SELECT id, name, events FROM sources ORDER BY id").unwrap();
    let total: u64 = sources.rows.iter().map(|r| r[2].as_u64().unwrap()).sum();
    assert_eq!(total, events);
    let joined = run(
        &store,
        "SELECT s.name, count(*) FROM events e JOIN sources s ON s.id = e.source GROUP BY ALL ORDER BY 2 DESC",
    )
    .unwrap();
    assert_eq!(joined.rows.len(), sources.rows.len());
    // Lists and maps come as text.
    let nested = run(&store, "SELECT [1, 2] AS l, fields FROM events LIMIT 1").unwrap();
    assert_eq!(nested.rows[0][0], "[1, 2]");
    assert!(nested.rows[0][1].is_string());

    // Notes come with the run.
    let note = SqlNote {
        id: "n1".into(),
        anchor: "event".into(),
        event_id: Some(ids[0]),
        severity: "problem".into(),
        tags: vec!["db".into(), "slow".into()],
        text: "Look here".into(),
        created: 1_760_000_000_000,
        updated: 1_760_000_000_000,
        ..SqlNote::default()
    };
    let r = store
        .sql_run(&SqlRequest {
            sql: "SELECT n.text, n.tags, e.message FROM notes n JOIN events e ON e.id = n.event_id".into(),
            explain: false,
            notes: vec![note.clone()],
        })
        .unwrap();
    assert_eq!(r.rows.len(), 1);
    let (id, path) = (
        sources.rows[0][0].as_u64().unwrap(),
        run(&store, "SELECT path FROM sources ORDER BY id LIMIT 1").unwrap().rows[0][0].clone(),
    );
    let on_source = SqlNote {
        id: "n2".into(),
        anchor: "source".into(),
        source_path: path.as_str().map(str::to_string),
        ..SqlNote::default()
    };
    let s = store
        .sql_run(&SqlRequest {
            sql: "SELECT id, source_id FROM notes ORDER BY id".into(),
            explain: false,
            notes: vec![note, on_source],
        })
        .unwrap();
    assert_eq!(s.rows[1][1], id, "a source note names its source");
    assert!(s.rows[0][1].is_null());
    assert_eq!(r.rows[0][0], "Look here");
    assert_eq!(r.rows[0][1], "[db, slow]");
    assert_eq!(
        run(&store, "SELECT count(*) FROM notes").unwrap().rows[0][0],
        0,
        "the next run's notes replace them"
    );

    // The internal tables are not there.
    for internal in ["segments", "segment_fields", "events_wide", "meta", "analysis_cache"] {
        let e = refused(&store, &format!("SELECT * FROM {internal}"));
        assert!(e.message.contains("Catalog Error"), "{internal}: {}", e.message);
    }
    // The views follow the store.
    store.clear().unwrap();
    assert_eq!(run(&store, "SELECT count(*) FROM events").unwrap().rows[0][0], 0);
}

#[test]
fn the_console_refuses_writes_files_settings_and_several_statements() {
    let (_dir, store, _) = opened("pane-sql-refuse");
    for (sql, says) in [
        ("COPY (SELECT * FROM events) TO 'out.csv'", "COPY"),
        ("ATTACH 'other.duckdb' AS o", "ATTACH"),
        ("INSTALL httpfs", "INSTALL"),
        ("LOAD httpfs", "LOAD"),
        ("SET enable_external_access = true", "SET"),
        ("PRAGMA database_list", "PRAGMA"),
        ("CALL pragma_version()", "CALL"),
        ("CREATE TABLE t AS SELECT * FROM events", "CREATE"),
        ("INSERT INTO sources VALUES (1)", "INSERT"),
        ("UPDATE sources SET name = 'x'", "UPDATE"),
        ("DELETE FROM sources", "DELETE"),
        ("DROP VIEW events", "DROP"),
        ("SELECT 1; DROP VIEW events", "one statement"),
        ("SELECT 1; SELECT 2", "one statement"),
        ("SELECT * FROM read_csv('C:/Windows/win.ini')", "read_csv"),
        ("SELECT * FROM read_text('/etc/passwd')", "read_text"),
        ("SELECT * FROM 'C:/data/secret.parquet'", "Files cannot be read"),
        ("SELECT * FROM 'https://example.com/x.csv'", "Files cannot be read"),
        ("SELECT * FROM glob('*')", "glob"),
        ("SELECT * FROM parquet_metadata('x')", "parquet_metadata"),
        ("SELECT * FROM duckdb_settings()", "duckdb_settings"),
        ("SELECT * FROM query('SELECT 1')", "query"),
        ("SELECT id FROM events WHERE id IN (SELECT 1 FROM read_json('x'))", "read_json"),
        ("WITH t AS (SELECT * FROM read_blob('x')) SELECT * FROM t", "read_blob"),
        ("EXPLAIN ANALYZE SELECT 1", "Explain switch"),
    ] {
        let e = refused(&store, sql);
        assert!(e.message.contains(says), "{sql}: {}", e.message);
    }
    let e = refused(&store, "  copy events to 'x'");
    assert_eq!((e.offset, e.len), (2, 4), "the keyword is underlined");
    let e = refused(&store, "SELECT * FROM events, read_csv('x')");
    assert_eq!(&"SELECT * FROM events, read_csv('x')"[e.offset..e.offset + e.len], "read_csv");
    let e = refused(&store, "SELEC 1");
    assert!(
        e.message.starts_with("Syntax error") || e.message.contains("read-only"),
        "{}",
        e.message
    );
    let long = format!("SELECT {}1", " ".repeat(debrief_core::ports::MAX_SQL_LEN));
    assert!(refused(&store, &long).message.contains("longer than"));
    // Allowed table functions run.
    assert_eq!(run(&store, "SELECT count(*) FROM range(10)").unwrap().rows[0][0], 10);
    assert_eq!(run(&store, "SELECT * FROM unnest([1, 2, 3])").unwrap().total, 3);
    assert_eq!(run(&store, "DESCRIBE events").unwrap().total, 10, "the view's columns");
}

#[test]
fn the_console_keeps_ten_thousand_rows_in_pages() {
    let (_dir, store, _) = opened("pane-sql-pages");
    let r = run(&store, "SELECT range AS n FROM range(25000)").unwrap();
    assert_eq!(r.total, SQL_MAX_ROWS as u64);
    assert!(r.truncated);
    assert_eq!(r.rows.len(), SQL_PAGE_ROWS);
    let last = store
        .sql_page(&SqlPageRequest {
            result: r.result,
            offset: SQL_MAX_ROWS - SQL_PAGE_ROWS,
        })
        .unwrap();
    assert_eq!(last.rows.len(), SQL_PAGE_ROWS);
    assert_eq!(last.rows.last().unwrap()[0], (SQL_MAX_ROWS - 1) as u64);
    let beyond = store
        .sql_page(&SqlPageRequest {
            result: r.result,
            offset: SQL_MAX_ROWS,
        })
        .unwrap();
    assert!(beyond.rows.is_empty());
    let small = run(&store, "SELECT 1").unwrap();
    assert!(!small.truncated && small.total == 1);
    // Old results are let go.
    for _ in 0..4 {
        run(&store, "SELECT 1").unwrap();
    }
    assert!(matches!(
        store.sql_page(&SqlPageRequest {
            result: r.result,
            offset: 0
        }),
        Err(StoreError::NotFound(_))
    ));
}

#[test]
fn the_console_stops_at_its_time_and_memory_limits_and_when_cancelled() {
    let (_dir, store, _) = opened("pane-sql-limits");
    store.set_sql_time_limit(Duration::from_millis(300));
    let started = std::time::Instant::now();
    match run(&store, "SELECT count(*) FROM range(1000000000000) a WHERE a.range % 7 = 3") {
        Err(StoreError::Invalid(m)) => assert!(m.starts_with("Stopped after 300 ms"), "{m}"),
        other => panic!("{other:?}"),
    }
    assert!(started.elapsed() < Duration::from_secs(20), "{:?}", started.elapsed());
    store.set_sql_time_limit(Duration::from_secs(60));
    match run(&store, "SELECT length(string_agg(repeat('x', 10000), '')) FROM range(100000)") {
        Err(StoreError::Invalid(m)) => assert!(m.contains("memory"), "{m}"),
        other => panic!("{other:?}"),
    }
    // Cancelled from another thread while it runs.
    let control = QueryControl::new();
    let c = control.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        c.cancel();
    });
    let started = std::time::Instant::now();
    let out = control.scope(|| run(&store, "SELECT count(*) FROM range(1000000000000) a WHERE a.range % 7 = 3"));
    canceller.join().unwrap();
    assert!(matches!(out, Err(StoreError::Cancelled)), "{out:?}");
    assert!(started.elapsed() < Duration::from_secs(20));
    // Cancelled before it starts.
    let control = QueryControl::new();
    control.cancel();
    assert!(matches!(control.scope(|| run(&store, "SELECT 1")), Err(StoreError::Cancelled)));
    // And it still works afterwards.
    assert_eq!(run(&store, "SELECT 42").unwrap().rows[0][0], 42);
}

#[test]
fn the_console_explains_and_lists_its_schema() {
    let (_dir, store, _) = opened("pane-sql-explain");
    let plan = store
        .sql_run(&SqlRequest {
            sql: "SELECT level, count(*) FROM events GROUP BY level".into(),
            explain: true,
            notes: vec![],
        })
        .unwrap();
    let text = plan.plan.expect("a plan");
    assert!(text.to_uppercase().contains("PARQUET") || text.contains("AGGREGATE"), "{text}");
    assert!(plan.rows.is_empty() && plan.columns.is_empty());
    assert!(
        run(&store, "explain select 1").unwrap().plan.is_some(),
        "a written EXPLAIN shows the plan too"
    );
    let schema = store.sql_schema().unwrap();
    assert_eq!(schema.version, debrief_core::ports::SQL_VIEWS_VERSION);
    assert_eq!(
        schema.views.iter().map(|v| v.name.as_str()).collect::<Vec<_>>(),
        ["events", "sources", "fields", "notes"]
    );
    assert!(schema.functions.iter().any(|f| f == "count") && schema.functions.iter().any(|f| f == "regexp_matches"));
    assert!(schema.table_functions.contains(&"range".to_string()));
    assert_eq!((schema.max_rows, schema.page_rows), (SQL_MAX_ROWS, SQL_PAGE_ROWS));
    // Every documented column is a column of its view.
    for v in &schema.views {
        let r = run(&store, &format!("SELECT * FROM {} LIMIT 0", v.name)).unwrap();
        assert_eq!(
            r.columns.iter().map(|c| (c.name.as_str(), c.ty.as_str())).collect::<Vec<_>>(),
            v.columns.iter().map(|c| (c.name.as_str(), c.ty.as_str())).collect::<Vec<_>>(),
            "{}",
            v.name
        );
    }
}

#[test]
fn one_run_at_a_time() {
    let (_dir, store, _) = opened("pane-sql-busy");
    let s = store.clone();
    let long = std::thread::spawn(move || {
        s.set_sql_time_limit(Duration::from_millis(1500));
        run(&s, "SELECT count(*) FROM range(1000000000000) a WHERE a.range % 7 = 3")
    });
    std::thread::sleep(Duration::from_millis(400));
    let second = run(&store, "SELECT 1");
    let first = long.join().unwrap();
    assert!(matches!(first, Err(StoreError::Invalid(_))), "{first:?}");
    assert!(matches!(second, Err(StoreError::Busy(_))), "{second:?}");
}

#[test]
fn open_in_events_takes_every_id_of_a_result() {
    let (_dir, store, total) = opened("pane-sql-ids");
    let r = run(
        &store,
        "SELECT id, message FROM events WHERE level IN ('Warning', 'Error', 'Fatal') UNION ALL SELECT id, message FROM events WHERE level IN ('Warning', 'Error', 'Fatal')",
    )
    .unwrap();
    assert!(r.event_ids);
    let ids = store.sql_ids(&SqlIdsRequest { result: r.result }).unwrap();
    assert!(!ids.truncated);
    assert_eq!(ids.ids.len() as u64, r.total / 2, "repeats are left out");
    // `@id in (...)` filters the grid to exactly those events, through SQL and through the scan.
    let ql = format!("@id in ({})", ids.ids.iter().map(u64::to_string).collect::<Vec<_>>().join(", "));
    let page = store
        .page(&PageQuery {
            query: debrief_core::ports::Filter {
                ql: ql.clone(),
                ..Default::default()
            },
            limit: debrief_core::ports::MAX_PAGE_ROWS,
            ..PageQuery::default()
        })
        .unwrap();
    assert_eq!(page.total, ids.ids.len());
    let mut shown: Vec<u64> = page.rows.iter().map(|e| e.id.0).collect();
    shown.sort_unstable();
    let mut wanted = ids.ids.clone();
    wanted.sort_unstable();
    assert_eq!(shown.len(), wanted.len(), "{} ids", wanted.len());
    assert_eq!(shown, wanted);
    let one = store
        .page(&PageQuery {
            query: debrief_core::ports::Filter {
                ql: format!("@id = {} and @message ~ \".\"", ids.ids[0]),
                ..Default::default()
            },
            limit: 5,
            ..PageQuery::default()
        })
        .unwrap();
    assert_eq!(one.rows.iter().map(|e| e.id.0).collect::<Vec<_>>(), vec![ids.ids[0]]);
    assert!(store.check_query("@id > 3").unwrap().is_none());
    assert!(total > 0);
    // A result without an id column, and a result that is gone.
    let plain = run(&store, "SELECT 1 AS n").unwrap();
    assert!(matches!(
        store.sql_ids(&SqlIdsRequest { result: plain.result }),
        Err(StoreError::Invalid(_))
    ));
    assert!(matches!(
        store.sql_ids(&SqlIdsRequest { result: 999_999 }),
        Err(StoreError::NotFound(_))
    ));
}

fn export(store: &DuckStore, sql: &str, path: &std::path::Path, overwrite: bool) -> StoreResult<SqlExportResult> {
    store.sql_export(&SqlExportRequest {
        sql: sql.into(),
        path: path.to_string_lossy().into_owned(),
        overwrite,
        ..SqlExportRequest::default()
    })
}

/// The files in `dir` (a failed export leaves no partial file).
fn files_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

#[test]
fn sql_results_export_to_csv_whole_and_bounded() {
    let (dir, store, total) = opened("pane-sql-export");
    let out_dir = dir.0.join("out");
    std::fs::create_dir_all(&out_dir).unwrap();
    let out = out_dir.join("events.csv");
    // More rows than a run keeps, cells longer than a page shows, and a formula-looking text.
    let r = export(
        &store,
        "SELECT range AS n, repeat('x', 5000) AS long, '=1+1' AS f, NULL AS nothing FROM range(12000)",
        &out,
        false,
    )
    .unwrap();
    assert_eq!((r.rows, r.truncated), (12_000, false));
    assert_eq!(r.bytes, std::fs::metadata(&out).unwrap().len());
    let mut reader = csv::Reader::from_path(&out).unwrap();
    assert_eq!(reader.headers().unwrap().iter().collect::<Vec<_>>(), ["n", "long", "f", "nothing"]);
    let rows: Vec<csv::StringRecord> = reader.records().map(|r| r.unwrap()).collect();
    assert_eq!(rows.len(), 12_000);
    assert_eq!(&rows[11_999][0], "11999");
    assert_eq!(rows[0][1].len(), 5000);
    assert_eq!(&rows[0][2], "'=1+1");
    assert_eq!(&rows[0][3], "");
    // The events view exports with its ids.
    let ev = out_dir.join("ids.csv");
    assert_eq!(export(&store, "SELECT id, message FROM events", &ev, false).unwrap().rows, total);
    // An existing file is replaced only when asked.
    assert!(matches!(export(&store, "SELECT 1", &out, false), Err(StoreError::Invalid(_))));
    assert_eq!(export(&store, "SELECT 1 AS one", &out, true).unwrap().rows, 1);
    // Refusals of the console hold for exports too, and EXPLAIN has no rows.
    assert!(matches!(
        export(&store, "DELETE FROM events", &out_dir.join("x.csv"), false),
        Err(StoreError::Query(_))
    ));
    assert!(matches!(
        export(&store, "EXPLAIN SELECT 1", &out_dir.join("x.csv"), false),
        Err(StoreError::Invalid(_))
    ));
    // The row and byte limits cut the export, saying so.
    store.set_sql_export_limits(100, u64::MAX, Duration::from_secs(60));
    let cut = export(&store, "SELECT range FROM range(1000)", &out_dir.join("cut.csv"), false).unwrap();
    assert_eq!((cut.rows, cut.truncated), (100, true));
    store.set_sql_export_limits(u64::MAX, 4_000, Duration::from_secs(60));
    let cut = export(
        &store,
        "SELECT repeat('y', 100) FROM range(1000)",
        &out_dir.join("bytes.csv"),
        false,
    )
    .unwrap();
    assert!(cut.truncated && cut.rows > 0 && cut.rows < 1000, "{cut:?}");
    assert!(cut.bytes <= 4_000);
    // Over its time limit, or cancelled, it stops and leaves no file behind.
    store.set_sql_export_limits(u64::MAX, u64::MAX, Duration::from_millis(300));
    let slow = out_dir.join("slow.csv");
    match export(
        &store,
        "SELECT count(*) FROM range(1000000000000) a WHERE a.range % 7 = 3",
        &slow,
        false,
    ) {
        Err(StoreError::Invalid(m)) => assert!(m.starts_with("Stopped after 300 ms"), "{m}"),
        other => panic!("{other:?}"),
    }
    store.set_sql_export_limits(u64::MAX, u64::MAX, Duration::from_secs(60));
    let control = QueryControl::new();
    let c = control.clone();
    let canceller = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(300));
        c.cancel();
    });
    let out2 = control.scope(|| {
        export(
            &store,
            "SELECT a.range FROM range(1000000000000) a WHERE a.range % 7 = 3",
            &slow,
            false,
        )
    });
    canceller.join().unwrap();
    assert!(matches!(out2, Err(StoreError::Cancelled)), "{out2:?}");
    assert_eq!(files_in(&out_dir), ["bytes.csv", "cut.csv", "events.csv", "ids.csv"]);
}
