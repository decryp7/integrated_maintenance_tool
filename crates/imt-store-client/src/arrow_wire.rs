//! Event batches between the store worker and IMT (ADR 0017 §8): each streamed item of a scan is one Arrow IPC
//! stream (schema and one record batch). A batch whose encoding would not fit a frame is split in halves until
//! it does; the receiving side checks every batch against the event schema it describes.

use arrow_array::RecordBatch;
use arrow_ipc::reader::StreamReader;
use arrow_ipc::writer::StreamWriter;
use imt_core::batch::{EventBatch, EventSchema};
use std::sync::Arc;

/// Why a batch could not be sent or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WireBatchError(pub String);

impl std::fmt::Display for WireBatchError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}

fn encode_one(rb: &RecordBatch) -> Result<Vec<u8>, WireBatchError> {
    let mut out = Vec::new();
    let mut w = StreamWriter::try_new(&mut out, rb.schema_ref()).map_err(|e| WireBatchError(format!("cannot encode a batch: {e}")))?;
    w.write(rb).map_err(|e| WireBatchError(format!("cannot encode a batch: {e}")))?;
    w.finish().map_err(|e| WireBatchError(format!("cannot encode a batch: {e}")))?;
    drop(w);
    Ok(out)
}

/// Encodes a batch as one or more items of at most `max_item` bytes each (a batch of one row that is still too
/// large is an error: the frame limit is far above the largest event).
pub fn encode(batch: &EventBatch, max_item: usize) -> Result<Vec<Vec<u8>>, WireBatchError> {
    let mut items = Vec::new();
    let mut todo = vec![batch.record_batch().clone()];
    while let Some(rb) = todo.pop() {
        let bytes = encode_one(&rb)?;
        if bytes.len() <= max_item {
            items.push(bytes);
            continue;
        }
        let rows = rb.num_rows();
        if rows <= 1 {
            return Err(WireBatchError(format!(
                "an event of {} bytes does not fit a message of {max_item} bytes",
                bytes.len()
            )));
        }
        // Second half first on the stack, so the halves come out in order.
        let half = rows / 2;
        todo.push(rb.slice(half, rows - half));
        todo.push(rb.slice(0, half));
    }
    Ok(items)
}

/// Decodes one item into an event batch, checking it against the event schema its Arrow schema describes.
pub fn decode(item: &[u8]) -> Result<EventBatch, WireBatchError> {
    let mut reader = StreamReader::try_new(item, None).map_err(|e| WireBatchError(format!("invalid batch from the store worker: {e}")))?;
    let rb = match reader.next() {
        Some(Ok(rb)) => rb,
        Some(Err(e)) => return Err(WireBatchError(format!("invalid batch from the store worker: {e}"))),
        None => return Err(WireBatchError("an empty batch message from the store worker".into())),
    };
    if reader.next().is_some() {
        return Err(WireBatchError(
            "a batch message from the store worker holds more than one batch".into(),
        ));
    }
    let schema =
        EventSchema::from_arrow(rb.schema_ref()).map_err(|e| WireBatchError(format!("invalid batch from the store worker: {e}")))?;
    EventBatch::try_new(Arc::new(schema), rb).map_err(|e| WireBatchError(format!("invalid batch from the store worker: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use imt_core::batch::{ColumnType, EventBatchBuilder, EventId, EventTime, FieldValue, LevelCode, NewEvent, SourceId};

    fn batch(rows: usize, message: &str) -> EventBatch {
        let schema = Arc::new(EventSchema::new([("Status", ColumnType::Int64)]).unwrap());
        let mut b = EventBatchBuilder::new(schema);
        for i in 0..rows {
            b.push(
                &NewEvent {
                    time: EventTime::parsed(Some(1_000 + i as i64)),
                    level: LevelCode::new(2).unwrap(),
                    source: SourceId(1),
                    line: i as u32 + 1,
                    byte_offset: 0,
                    message,
                    template: None,
                    exception: None,
                    markers: 0,
                    raw: "",
                },
                [("Status", FieldValue::Int(i as i64)), ("Host", FieldValue::Text("web"))],
            )
            .unwrap();
        }
        b.finish().assign_ids(EventId(10)).unwrap()
    }

    #[test]
    fn batches_round_trip_and_split_to_fit() {
        let b = batch(100, "hello");
        let items = encode(&b, 1 << 20).unwrap();
        assert_eq!(items.len(), 1);
        let back = decode(&items[0]).unwrap();
        assert_eq!(back.num_rows(), 100);
        assert_eq!(back.row(5).id(), EventId(15));
        assert_eq!(back.row(5).message(), "hello");
        assert!(back.schema().same_columns(b.schema()));

        let big = batch(64, &"x".repeat(4096));
        let small = encode(&big, 64 << 10).unwrap();
        assert!(small.len() > 1);
        let ids: Vec<u64> = small
            .iter()
            .flat_map(|i| decode(i).unwrap().rows().map(|r| r.id().0).collect::<Vec<_>>())
            .collect();
        assert_eq!(ids, (10..74).collect::<Vec<_>>(), "split in order, nothing lost");
        assert!(encode(&batch(1, &"x".repeat(4096)), 100).is_err());
    }

    #[test]
    fn garbage_is_refused() {
        assert!(decode(b"").is_err());
        assert!(decode(b"not arrow").is_err());
        let two = {
            let b = batch(2, "m");
            let mut out = Vec::new();
            let mut w = StreamWriter::try_new(&mut out, b.record_batch().schema_ref()).unwrap();
            w.write(b.record_batch()).unwrap();
            w.write(b.record_batch()).unwrap();
            w.finish().unwrap();
            drop(w);
            out
        };
        assert!(decode(&two).unwrap_err().0.contains("more than one"));
    }
}
