//! DataFusion + Parquet adapter: each ingest chunk becomes one Parquet file (zstd, 64k-row row
//! groups, page statistics) of the same typed schema as the DuckDB table; queries are DataFusion
//! logical plans built with the DataFrame API (no SQL text), the way the query language would
//! compile. The full-text index lives next to the files.

use crate::corpus::Chunk;
use crate::fts::{Finished, Fts, FtsDoc, Indexer};
use crate::port::{Column, ColumnKind, FacetOut, Filter, PageOut, SearchOut, SpikeStore, StatsOut, TimelineOut, HISTOGRAM_BINS};
use crate::rows::{promoted, rest_json, Cell};
use anyhow::{Context, Result};
use datafusion::arrow::array::{
    Array, ArrayRef, AsArray, Float64Builder, Int64Builder, RecordBatch, StringBuilder, UInt32Builder, UInt64Builder, UInt8Builder,
};
use datafusion::arrow::datatypes::{DataType, Field, Float64Type, Int64Type, Schema, SchemaRef, UInt64Type};
use datafusion::common::ScalarValue;
use datafusion::datasource::file_format::parquet::ParquetFormat;
use datafusion::datasource::listing::ListingOptions;
use datafusion::execution::memory_pool::FairSpillPool;
use datafusion::execution::runtime_env::RuntimeEnvBuilder;
use datafusion::functions_aggregate::expr_fn::{approx_percentile_cont, avg, count, count_distinct, max, min};
use datafusion::logical_expr::expr::Like;
use datafusion::logical_expr::{cast, col, lit, when, Expr};
use datafusion::parquet::arrow::ArrowWriter;
use datafusion::parquet::basic::{Compression, ZstdLevel};
use datafusion::parquet::file::properties::{EnabledStatistics, WriterProperties};
use datafusion::prelude::{DataFrame, SessionConfig, SessionContext};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use crate::port::{MEMORY_LIMIT, THREADS};

const ROW_GROUP: usize = 64 * 1024;

fn schema() -> SchemaRef {
    let t = |n: &str| Field::new(n, DataType::Utf8, true);
    Arc::new(Schema::new(vec![
        Field::new("id", DataType::UInt64, false),
        Field::new("ts", DataType::Int64, true),
        Field::new("level", DataType::UInt8, false),
        Field::new("source", DataType::UInt32, false),
        Field::new("line", DataType::UInt32, false),
        Field::new("markers", DataType::UInt64, false),
        t("machine_name"),
        Field::new("process_id", DataType::Int64, true),
        t("source_context"),
        Field::new("status_code", DataType::Int64, true),
        Field::new("elapsed", DataType::Float64, true),
        Field::new("message", DataType::Utf8, false),
        t("template"),
        t("exception"),
        t("fields"),
    ]))
}

fn memory_limit_bytes() -> usize {
    MEMORY_LIMIT.trim_end_matches("GB").parse::<usize>().unwrap_or(2) << 30
}

/// Writes a new store: one Parquet file per chunk, `sources.json`, the full-text index on its own thread.
pub struct FusionIngest {
    dir: PathBuf,
    files: usize,
    fts: Indexer,
    props: WriterProperties,
    sources: Vec<serde_json::Value>,
}

impl FusionIngest {
    pub fn create(dir: &Path) -> Result<FusionIngest> {
        if dir.exists() {
            std::fs::remove_dir_all(dir)?;
        }
        std::fs::create_dir_all(dir.join("events"))?;
        let props = WriterProperties::builder()
            .set_compression(Compression::ZSTD(ZstdLevel::try_new(3)?))
            .set_max_row_group_row_count(Some(ROW_GROUP))
            .set_statistics_enabled(EnabledStatistics::Page)
            .build();
        Ok(FusionIngest {
            dir: dir.to_path_buf(),
            files: 0,
            fts: Indexer::start(&dir.join("fts"))?,
            props,
            sources: Vec::new(),
        })
    }

    pub fn append(&mut self, chunk: Chunk<'_>) -> Result<()> {
        let n = chunk.records.len();
        let (mut id, mut ts, mut level, mut source, mut line, mut markers) = (
            UInt64Builder::with_capacity(n),
            Int64Builder::with_capacity(n),
            UInt8Builder::with_capacity(n),
            UInt32Builder::with_capacity(n),
            UInt32Builder::with_capacity(n),
            UInt64Builder::with_capacity(n),
        );
        let (mut machine, mut pid, mut ctx, mut status, mut elapsed) = (
            StringBuilder::new(),
            Int64Builder::with_capacity(n),
            StringBuilder::new(),
            Int64Builder::with_capacity(n),
            Float64Builder::with_capacity(n),
        );
        let (mut message, mut template, mut exception, mut fields) = (
            StringBuilder::new(),
            StringBuilder::new(),
            StringBuilder::new(),
            StringBuilder::new(),
        );
        let mut docs = Vec::with_capacity(n);
        for r in chunk.records {
            let gid = chunk.id_base + r.id as u64;
            id.append_value(gid);
            ts.append_option(r.ts);
            level.append_value(r.level.index() as u8);
            source.append_value(chunk.source_base + r.source);
            line.append_value(r.line);
            markers.append_value(r.markers);
            let p = promoted(r);
            let text = |b: &mut StringBuilder, c: &Cell<'_>| match c {
                Cell::Text(s) => b.append_value(s),
                _ => b.append_null(),
            };
            let int = |b: &mut Int64Builder, c: &Cell<'_>| match c {
                Cell::Integer(i) => b.append_value(*i),
                _ => b.append_null(),
            };
            text(&mut machine, &p[0]);
            int(&mut pid, &p[1]);
            text(&mut ctx, &p[2]);
            int(&mut status, &p[3]);
            match p[4] {
                Cell::Float(f) => elapsed.append_value(f),
                _ => elapsed.append_null(),
            }
            message.append_value(&r.message);
            template.append_option(r.template.as_deref());
            exception.append_option(r.exception.as_deref());
            let rest = rest_json(r);
            fields.append_option((!rest.is_empty()).then_some(rest));
            docs.push(FtsDoc {
                id: gid,
                ts: r.ts.unwrap_or(i64::MAX),
                message: r.message.clone(),
                exception: r.exception.clone(),
            });
        }
        let columns: Vec<ArrayRef> = vec![
            Arc::new(id.finish()),
            Arc::new(ts.finish()),
            Arc::new(level.finish()),
            Arc::new(source.finish()),
            Arc::new(line.finish()),
            Arc::new(markers.finish()),
            Arc::new(machine.finish()),
            Arc::new(pid.finish()),
            Arc::new(ctx.finish()),
            Arc::new(status.finish()),
            Arc::new(elapsed.finish()),
            Arc::new(message.finish()),
            Arc::new(template.finish()),
            Arc::new(exception.finish()),
            Arc::new(fields.finish()),
        ];
        let batch = RecordBatch::try_new(schema(), columns)?;
        // Zero-padded names keep listing order = time order.
        let path = self.dir.join("events").join(format!("part-{:05}.parquet", self.files));
        let file = std::fs::File::create(&path)?;
        let mut w = ArrowWriter::try_new(file, schema(), Some(self.props.clone()))?;
        w.write(&batch)?;
        w.close()?;
        self.files += 1;
        self.sources.extend(
            chunk
                .sources
                .iter()
                .map(|s| serde_json::json!({ "id": s.id, "path": s.path, "name": s.name })),
        );
        self.fts.send(docs)
    }

    /// Waits for the full-text index.
    pub fn finish(self, started: std::time::Instant) -> Result<Finished> {
        // The source table is small: one JSON file next to the segments.
        std::fs::write(self.dir.join("sources.json"), serde_json::Value::Array(self.sources).to_string())?;
        let table_ms = started.elapsed().as_millis();
        let indexed_at_table = self.fts.progress();
        let report = self.fts.finish()?;
        Ok(Finished {
            table_ms,
            indexed_at_table,
            total_ms: started.elapsed().as_millis(),
            indexed: report.docs,
            cancelled: report.cancelled,
        })
    }
}

/// An opened store.
pub struct FusionStore {
    rt: tokio::runtime::Runtime,
    ctx: SessionContext,
    fts: Fts,
    pub total: u64,
    pub from: i64,
    pub to: i64,
}

impl FusionStore {
    /// The full-text index (for its progress after reopening).
    pub fn fts(&self) -> &Fts {
        &self.fts
    }

    pub fn open(dir: &Path) -> Result<FusionStore> {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(THREADS)
            .enable_all()
            .build()?;
        let mut config = SessionConfig::new().with_target_partitions(THREADS).with_collect_statistics(true);
        config.options_mut().execution.parquet.pushdown_filters = true;
        config.options_mut().execution.parquet.reorder_filters = true;
        let runtime = RuntimeEnvBuilder::new()
            .with_memory_pool(Arc::new(FairSpillPool::new(memory_limit_bytes())))
            .build_arc()?;
        let ctx = SessionContext::new_with_config_rt(config, runtime);
        let url = format!("{}/", dir.join("events").to_string_lossy().replace('\\', "/"));
        let opts = ListingOptions::new(Arc::new(ParquetFormat::default()))
            .with_file_extension(".parquet")
            .with_file_sort_order(vec![vec![col("id").sort(true, false)]]);
        rt.block_on(ctx.register_listing_table("events", &url, opts, Some(schema()), None))
            .with_context(|| format!("opening {}", dir.display()))?;
        let mut s = FusionStore {
            rt,
            ctx,
            fts: Fts::open(&dir.join("fts"))?,
            total: 0,
            from: 0,
            to: 0,
        };
        let b = s.collect(s.df()?.aggregate(vec![], vec![count(lit(1)), min(col("ts")), max(col("ts"))])?)?;
        if let Some(b) = b.first().filter(|b| b.num_rows() > 0) {
            s.total = b.column(0).as_primitive::<Int64Type>().value(0) as u64;
            s.from = b.column(1).as_primitive::<Int64Type>().value(0);
            s.to = b.column(2).as_primitive::<Int64Type>().value(0);
        }
        Ok(s)
    }

    fn df(&self) -> Result<DataFrame> {
        Ok(self.rt.block_on(self.ctx.table("events"))?)
    }

    fn filtered(&self, f: &Filter) -> Result<DataFrame> {
        let df = self.df()?;
        Ok(match filter_expr(f) {
            Some(e) => df.filter(e)?,
            None => df,
        })
    }

    fn collect(&self, df: DataFrame) -> Result<Vec<RecordBatch>> {
        Ok(self.rt.block_on(df.collect())?)
    }

    fn page_rows(&self, df: DataFrame) -> Result<PageOut> {
        let df = df.select_columns(&[
            "id",
            "ts",
            "level",
            "message",
            "template",
            "exception",
            "machine_name",
            "process_id",
            "source_context",
            "fields",
        ])?;
        let mut ids = Vec::new();
        let mut bytes = 0;
        for b in self.collect(df)? {
            ids.extend(b.column(0).as_primitive::<UInt64Type>().values().iter().copied());
            let m = b.column(3).as_string::<i32>();
            bytes += (0..m.len()).map(|i| m.value(i).len()).sum::<usize>();
        }
        Ok(PageOut {
            total: 0,
            ids,
            message_bytes: bytes,
        })
    }

    /// A query from the SQL console: no DDL, DML or other statements (SET, COPY ...), at most
    /// `max_rows` rows, cancelled after `timeout`.
    pub fn console(&self, sql: &str, timeout: std::time::Duration, max_rows: usize) -> Result<Vec<RecordBatch>> {
        use datafusion::execution::context::SQLOptions;
        let opts = SQLOptions::new()
            .with_allow_ddl(false)
            .with_allow_dml(false)
            .with_allow_statements(false);
        // The deadline is kept by this OS thread, not by tokio: a `tokio::time::timeout` (around
        // `block_on`, or around a spawned task) never fired while CPU-bound operators held every
        // worker thread (measured: a 2 s timeout returned after two hours). Aborting the task drops
        // the stream; DataFusion stops at its next yield point.
        let ctx = self.ctx.clone();
        let sql = sql.to_string();
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        let task = self.rt.spawn(async move {
            let run = async {
                let df = ctx.sql_with_options(&sql, opts).await?;
                df.limit(0, Some(max_rows))?.collect().await
            };
            let _ = tx.send(run.await);
        });
        match rx.recv_timeout(timeout) {
            Ok(r) => Ok(r?),
            Err(_) => {
                task.abort();
                anyhow::bail!("the query ran longer than {} s and was cancelled", timeout.as_secs_f32())
            }
        }
    }
}

fn like(c: &str, pattern: &str) -> Expr {
    Expr::Like(Like::new(
        false,
        Box::new(col(c)),
        Box::new(lit(pattern.to_string())),
        Some('\\'),
        true,
    ))
}

/// Compiles a filter to a DataFusion predicate.
pub fn filter_expr(f: &Filter) -> Option<Expr> {
    let mut parts = Vec::new();
    if !f.levels.is_empty() {
        parts.push(col("level").in_list(f.levels.iter().map(|l| lit(l.index() as u8)).collect(), false));
    }
    if let Some(t) = &f.text {
        let p = Filter::like_pattern(t.trim());
        parts.push(like("message", &p).or(like("exception", &p)).or(like("template", &p)));
    }
    if let Some((c, v)) = &f.field {
        let value = match c.kind() {
            ColumnKind::Text => Some(lit(v.clone())),
            ColumnKind::Integer => v.trim().parse::<i64>().ok().map(lit),
            ColumnKind::Float => v.trim().parse::<f64>().ok().map(lit),
        };
        parts.push(match value {
            Some(value) => col(c.column()).eq(value),
            None => lit(false),
        });
    }
    parts.into_iter().reduce(Expr::and)
}

fn u64_at(b: &RecordBatch, c: usize, row: usize) -> u64 {
    let a = b.column(c);
    match a.data_type() {
        DataType::Int64 => a.as_primitive::<Int64Type>().value(row) as u64,
        DataType::UInt64 => a.as_primitive::<UInt64Type>().value(row),
        _ => 0,
    }
}

fn f64_at(b: &RecordBatch, c: usize, row: usize) -> f64 {
    let a = b.column(c);
    if a.is_null(row) {
        return f64::NAN;
    }
    match a.data_type() {
        DataType::Float64 => a.as_primitive::<Float64Type>().value(row),
        DataType::Int64 => a.as_primitive::<Int64Type>().value(row) as f64,
        _ => f64::NAN,
    }
}

fn text_at(a: &dyn Array, row: usize) -> String {
    match ScalarValue::try_from_array(a, row) {
        Ok(v) if !v.is_null() => v.to_string(),
        _ => String::new(),
    }
}

impl SpikeStore for FusionStore {
    fn page(&self, filter: &Filter, offset: u64, limit: usize) -> Result<PageOut> {
        if filter.is_empty() {
            let df = self
                .df()?
                .filter(col("id").gt_eq(lit(offset)).and(col("id").lt(lit(offset + limit as u64))))?
                .sort(vec![col("id").sort(true, false)])?;
            let mut p = self.page_rows(df)?;
            p.total = self.total;
            return Ok(p);
        }
        let df = self.filtered(filter)?;
        let total = self.rt.block_on(df.clone().count())? as u64;
        let df = df.sort(vec![col("id").sort(true, false)])?.limit(offset as usize, Some(limit))?;
        let mut p = self.page_rows(df)?;
        p.total = total;
        Ok(p)
    }

    fn facet(&self, column: Column, filter: &Filter, limit: usize) -> Result<FacetOut> {
        let c = column.column();
        let df = self.filtered(filter)?;
        let b = self.collect(df.clone().aggregate(vec![], vec![count(col(c)), count_distinct(col(c))])?)?;
        let (total, distinct) = b.first().map_or((0, 0), |b| (u64_at(b, 0, 0), u64_at(b, 1, 0)));
        let top = df
            .filter(col(c).is_not_null())?
            .aggregate(vec![col(c)], vec![count(lit(1)).alias("n")])?
            .sort(vec![col("n").sort(false, false), col(c).sort(true, false)])?
            .limit(0, Some(limit))?;
        let mut values = Vec::new();
        for b in self.collect(top)? {
            for row in 0..b.num_rows() {
                values.push((text_at(b.column(0), row), u64_at(&b, 1, row)));
            }
        }
        Ok(FacetOut {
            total,
            distinct,
            top: values,
        })
    }

    fn timeline(&self, filter: &Filter, buckets: usize) -> Result<TimelineOut> {
        let (from, to) = (self.from, self.to);
        let span = (to - from).max(1);
        let df = self.filtered(filter)?;
        let bucket = ((col("ts") - lit(from)) * lit(buckets as i64) / lit(span)).alias("b");
        let hist = df
            .clone()
            .filter(col("ts").is_not_null())?
            .aggregate(vec![bucket, col("level")], vec![count(lit(1))])?;
        let mut histogram = vec![[0u64; 6]; buckets];
        for b in self.collect(hist)? {
            let lv = b.column(1).as_primitive::<datafusion::arrow::datatypes::UInt8Type>();
            for row in 0..b.num_rows() {
                let i = b.column(0).as_primitive::<Int64Type>().value(row);
                histogram[(i.max(0) as usize).min(buckets - 1)][lv.value(row) as usize % 6] += u64_at(&b, 2, row);
            }
        }
        let key = when(col("machine_name").is_null().and(col("process_id").is_null()), col("source")).end()?;
        let lanes = self
            .df()?
            .aggregate(vec![col("machine_name"), col("process_id"), key.alias("src")], vec![])?;
        let lanes = self.rt.block_on(lanes.count())?;
        Ok(TimelineOut {
            from,
            to,
            histogram,
            lanes,
        })
    }

    fn field_stats(&self, column: Column, filter: &Filter) -> Result<StatsOut> {
        let c = column.column();
        let df = self.filtered(filter)?;
        let x = || cast(col(c), DataType::Float64);
        let pct = |p: f64| approx_percentile_cont(x().sort(true, false), lit(p), None);
        let agg = df.clone().aggregate(
            vec![],
            vec![
                count(col(c)),
                count(lit(1)),
                count_distinct(col(c)),
                min(x()),
                max(x()),
                avg(x()),
                pct(0.5),
                pct(0.95),
                pct(0.99),
            ],
        )?;
        let b = self.collect(agg)?;
        let b = b.first().context("no statistics row")?;
        let n = u64_at(b, 0, 0);
        let mut s = StatsOut {
            count: n,
            missing: u64_at(b, 1, 0) - n,
            distinct: u64_at(b, 2, 0),
            min: f64_at(b, 3, 0),
            max: f64_at(b, 4, 0),
            mean: f64_at(b, 5, 0),
            p50: f64_at(b, 6, 0),
            p95: f64_at(b, 7, 0),
            p99: f64_at(b, 8, 0),
            histogram: Vec::new(),
        };
        if n > 0 && column.kind() != ColumnKind::Text {
            let width = ((s.max - s.min) / HISTOGRAM_BINS as f64).max(f64::MIN_POSITIVE);
            let bin = cast((x() - lit(s.min)) / lit(width), DataType::Int64).alias("b");
            let h = df.clone().filter(col(c).is_not_null())?.aggregate(vec![bin], vec![count(lit(1))])?;
            let mut hist = vec![0u64; HISTOGRAM_BINS];
            for b in self.collect(h)? {
                for row in 0..b.num_rows() {
                    let i = b.column(0).as_primitive::<Int64Type>().value(row);
                    hist[(i.max(0) as usize).min(HISTOGRAM_BINS - 1)] += u64_at(&b, 1, row);
                }
            }
            s.histogram = hist;
        }
        let _top = self.facet(column, filter, 20)?;
        Ok(s)
    }

    fn search(&self, text: &str, limit: usize) -> Result<SearchOut> {
        let (hits, ids) = self.fts.search(text, limit)?;
        if ids.is_empty() {
            return Ok(SearchOut { hits, ids, fetched: 0 });
        }
        let list = ids.iter().map(|i| lit(*i)).collect();
        let df = self
            .df()?
            .filter(col("id").in_list(list, false))?
            .sort(vec![col("id").sort(true, false)])?;
        let p = self.page_rows(df)?;
        Ok(SearchOut {
            hits,
            fetched: p.ids.len(),
            ids,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::model::Level;

    #[test]
    fn filters_compile_to_predicates() {
        assert!(filter_expr(&Filter::default()).is_none());
        let f = Filter {
            levels: vec![Level::Error],
            text: Some("a%b".into()),
            field: Some((Column::StatusCode, "x".into())),
        };
        let e = filter_expr(&f).unwrap().to_string();
        assert!(e.contains("level IN"), "{e}");
        assert!(e.contains("ILIKE"), "{e}");
        assert!(e.contains("a\\%b"), "{e}");
        assert!(e.ends_with("AND Boolean(false)"), "{e}");
    }
}
