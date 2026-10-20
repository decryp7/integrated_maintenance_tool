//! Event segments (ADR 0014 R2b notes): events are stored as zstd-compressed Parquet files of about
//! [`SEGMENT_ROWS`] rows under `<store>/segments/`, written by the parse threads in parallel and listed in
//! the `segments` table. Each segment owns a contiguous block of event ids (`first_id .. end_id`), so the
//! ids of a segment are dense and a lookup by id prunes to one file; unused ids at a block's end are never
//! handed out again.
//!
//! A segment is written to `<name>.tmp` and renamed when complete, so a crash leaves no half-written file
//! that a query could read; files that are not in the manifest are deleted when the store is opened.

use anyhow::{Context, Result};
use imt_core::batch::{EventBatch, EventSchema};
use parquet::arrow::ArrowWriter;
use parquet::basic::{Compression, ZstdLevel};
use parquet::file::properties::WriterProperties;
use std::collections::BTreeSet;
use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// Rows of a full segment (16 batches); also the size of the id block a segment reserves.
pub const SEGMENT_ROWS: u64 = 131_072;

/// Rows of a Parquet row group (the unit DuckDB prunes and reads in parallel).
pub const ROW_GROUP_ROWS: usize = 65_536;

/// A segment still being written is closed after this long, so a slow import becomes visible.
pub const SEGMENT_AGE: Duration = Duration::from_secs(2);

/// Folder of the segments in a store folder.
pub const SEGMENTS_DIR: &str = "segments";

/// Extension of a segment being written.
const TMP: &str = "tmp";

fn props() -> WriterProperties {
    WriterProperties::builder()
        .set_compression(Compression::ZSTD(ZstdLevel::default()))
        .set_max_row_group_row_count(Some(ROW_GROUP_ROWS))
        .build()
}

/// The file name of the segment starting at `first_id` (`generation` tells rewritten segments apart).
pub fn file_name(first_id: u64, generation: u64) -> String {
    format!("seg-{first_id:016x}-{generation:04x}.parquet")
}

/// A complete segment, as listed in the manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Segment {
    pub file: String,
    pub first_id: u64,
    /// One past the last id of the block.
    pub end_id: u64,
    pub rows: u64,
    /// Sources with rows in the segment.
    pub sources: BTreeSet<u32>,
}

/// A segment being written by one parse thread.
pub struct SegmentWriter {
    dir: PathBuf,
    file: String,
    writer: ArrowWriter<File>,
    first_id: u64,
    next_id: u64,
    end_id: u64,
    rows: u64,
    sources: BTreeSet<u32>,
    opened: Instant,
}

impl SegmentWriter {
    /// Starts a segment in `dir` for the ids `first_id .. end_id`.
    pub fn create(dir: &Path, schema: &EventSchema, first_id: u64, end_id: u64, generation: u64) -> Result<SegmentWriter> {
        let file = file_name(first_id, generation);
        let tmp = dir.join(&file).with_extension(TMP);
        let f = File::create(&tmp).with_context(|| format!("creating {}", tmp.display()))?;
        let writer = ArrowWriter::try_new(f, schema.arrow().clone(), Some(props())).context("starting a segment")?;
        Ok(SegmentWriter {
            dir: dir.to_path_buf(),
            file,
            writer,
            first_id,
            next_id: first_id,
            end_id,
            rows: 0,
            sources: BTreeSet::new(),
            opened: Instant::now(),
        })
    }

    /// Ids left in the block.
    pub fn remaining(&self) -> u64 {
        self.end_id - self.next_id
    }

    /// The first id the next batch gets.
    pub fn next_id(&self) -> u64 {
        self.next_id
    }

    pub fn rows(&self) -> u64 {
        self.rows
    }

    /// Whether the segment has been open long enough to be closed early.
    pub fn is_old(&self) -> bool {
        self.opened.elapsed() >= SEGMENT_AGE
    }

    pub fn contains_source(&self, source: u32) -> bool {
        self.sources.contains(&source)
    }

    /// Appends a batch whose ids start at [`SegmentWriter::next_id`]; returns the sources that are new in
    /// this segment.
    pub fn write(&mut self, batch: &EventBatch) -> Result<Vec<u32>> {
        let n = batch.num_rows() as u64;
        anyhow::ensure!(n <= self.remaining(), "a batch of {n} rows does not fit the segment's id block");
        self.writer.write(batch.record_batch()).context("writing a segment")?;
        self.next_id += n;
        self.rows += n;
        let mut new = Vec::new();
        let mut last = None;
        for r in batch.rows() {
            let s = r.source().0;
            if last != Some(s) {
                last = Some(s);
                if self.sources.insert(s) {
                    new.push(s);
                }
            }
        }
        Ok(new)
    }

    /// Finishes the file and moves it into place.
    pub fn close(self) -> Result<Segment> {
        let tmp = self.dir.join(&self.file).with_extension(TMP);
        let f = self.writer.into_inner().context("finishing a segment")?;
        f.sync_all().ok();
        drop(f);
        std::fs::rename(&tmp, self.dir.join(&self.file)).with_context(|| format!("moving {} into place", tmp.display()))?;
        Ok(Segment {
            file: self.file,
            first_id: self.first_id,
            end_id: self.end_id,
            rows: self.rows,
            sources: self.sources,
        })
    }

    /// Drops an unfinished segment (an import that failed).
    pub fn discard(self) {
        let tmp = self.dir.join(&self.file).with_extension(TMP);
        drop(self.writer);
        let _ = std::fs::remove_file(tmp);
    }
}

/// Deletes what a crash may have left in `dir`: unfinished segments and files the manifest does not list.
pub fn sweep(dir: &Path, listed: &BTreeSet<String>) -> Vec<PathBuf> {
    let mut left = Vec::new();
    let Ok(entries) = std::fs::read_dir(dir) else { return left };
    for e in entries.flatten() {
        let name = e.file_name().to_string_lossy().into_owned();
        if listed.contains(&name) {
            continue;
        }
        if std::fs::remove_file(e.path()).is_err() {
            left.push(e.path());
        }
    }
    left
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::batch::{EventBatchBuilder, EventId, EventTime, LevelCode, NewEvent, SourceId};
    use std::sync::Arc;

    pub(crate) fn batch(schema: &Arc<EventSchema>, source: u32, n: usize, first: u64) -> EventBatch {
        let mut b = EventBatchBuilder::new(schema.clone());
        for i in 0..n {
            let ev = NewEvent {
                time: EventTime::Parsed(i as i64),
                level: LevelCode::new(2).unwrap(),
                source: SourceId(source),
                line: i as u32 + 1,
                byte_offset: 0,
                message: "m",
                template: None,
                exception: None,
                markers: 0,
                raw: "raw m",
            };
            b.push(&ev, [("K", imt_core::batch::FieldValue::Text("v"))]).unwrap();
        }
        b.finish().assign_ids(EventId(first)).unwrap()
    }

    #[test]
    fn segments_are_written_whole_and_swept() {
        let dir = std::env::temp_dir().join(format!("imt-seg-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let schema = Arc::new(EventSchema::plain());
        let mut w = SegmentWriter::create(&dir, &schema, 100, 110, 0).unwrap();
        assert_eq!(w.write(&batch(&schema, 3, 4, 100)).unwrap(), [3]);
        assert_eq!(w.write(&batch(&schema, 3, 2, 104)).unwrap(), Vec::<u32>::new());
        assert_eq!((w.next_id(), w.remaining(), w.rows()), (106, 4, 6));
        assert!(w.contains_source(3) && !w.is_old());
        assert!(w.write(&batch(&schema, 4, 5, 106)).is_err(), "does not fit the block");
        assert!(dir.join(file_name(100, 0)).with_extension(TMP).is_file(), "written as .tmp");
        let seg = w.close().unwrap();
        assert_eq!((seg.first_id, seg.end_id, seg.rows), (100, 110, 6));
        assert!(dir.join(&seg.file).is_file());
        // A discarded segment leaves nothing; files not listed are swept.
        SegmentWriter::create(&dir, &schema, 200, 210, 0).unwrap().discard();
        std::fs::write(dir.join("stray.tmp"), b"x").unwrap();
        assert!(sweep(&dir, &[seg.file.clone()].into()).is_empty());
        let left: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, [std::ffi::OsString::from(&seg.file)]);
        std::fs::remove_dir_all(dir).ok();
    }
}
