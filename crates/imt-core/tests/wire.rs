//! Requests from the UI (camelCase JSON) fill in documented defaults for the
//! fields they leave out.

use imt_core::api::{FieldStatsRequest, ParsePreviewRequest, RequestsRequest};
use serde_json::json;

#[test]
fn omitted_request_fields_take_their_defaults() {
    let stats: FieldStatsRequest = serde_json::from_value(json!({"field": "ElapsedMs"})).unwrap();
    assert_eq!((stats.field.as_str(), stats.buckets), ("ElapsedMs", 20));

    let requests: RequestsRequest = serde_json::from_value(json!({})).unwrap();
    assert_eq!(requests.id_field, "RequestId|CorrelationId|TraceId");

    let preview: ParsePreviewRequest = serde_json::from_value(json!({"toml": "", "sample": "x"})).unwrap();
    assert_eq!((preview.file_name.as_str(), preview.limit), ("", 50));

    // Given values win.
    let preview: ParsePreviewRequest = serde_json::from_value(json!({"toml": "", "sample": "x", "fileName": "a.log", "limit": 3})).unwrap();
    assert_eq!((preview.file_name.as_str(), preview.limit), ("a.log", 3));
}
