//! Clock-skew correction commands (per window): set a source's time offset
//! (manually or by aligning two events) and suggest offsets from shared
//! correlation ids. Payloads are documented in app/src/app/core/api.types.ts.

use crate::activity::{domain_event, OpId};
use crate::windows::{read, write, CurrentWindow};
use crate::CmdResult;
use imt_core::skew::{OffsetChange, SetOffsetRequest, SuggestRequest, SuggestResult, MAX_TRACKED};

/// Sets a source's clock offset and moves its records; returns the new ids of
/// the tracked records so the UI can keep its selection and bookmarks.
#[tauri::command]
pub(crate) async fn set_source_offset(state: CurrentWindow, op: OpId, request: SetOffsetRequest) -> CmdResult<OffsetChange> {
    if request.track.len() > MAX_TRACKED {
        return Err(crate::CmdError::from(format!(
            "too many tracked events ({}; at most {MAX_TRACKED})",
            request.track.len()
        )));
    }
    write(&state, move |ws| {
        let change = ws.set_source_offset(&request)?;
        log::info!(OffsetMs = change.offset_ms, PreviousMs = change.previous_ms; "Clock offset set");
        let (source, offset) = (change.source.to_string(), change.offset_ms.to_string());
        domain_event("source.offsetSet", op.as_deref(), &[("SourceId", &source), ("OffsetMs", &offset)]);
        Ok(change)
    })
    .await
}

/// Offsets that would align each source with a reference, from events sharing
/// a correlation id, with a confidence per source.
#[tauri::command]
pub(crate) async fn suggest_offsets(state: CurrentWindow, request: Option<SuggestRequest>) -> CmdResult<SuggestResult> {
    read(&state, move |ws| Ok(ws.suggest_offsets(&request.unwrap_or_default()))).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, test_app, TempDir};
    use imt_core::skew::OffsetSpec;
    use serde_json::json;

    fn clef(ms: i64, msg: &str, id: &str) -> String {
        let t = chrono::DateTime::from_timestamp_millis(1_791_972_000_000 + ms)
            .unwrap()
            .format("%Y-%m-%dT%H:%M:%S%.3fZ");
        format!("{{\"@t\":\"{t}\",\"@m\":\"{msg}\",\"TraceId\":\"{id}\"}}\n")
    }

    #[test]
    fn set_and_suggest_through_commands() {
        let _log = crate::activity::test_capture();
        let dir = TempDir::new("skew");
        let a: String = (0..12).map(|i| clef(i * 1000, "A", &format!("t{i}"))).collect();
        let b: String = (0..12).map(|i| clef(i * 1000 - 1_200, "B", &format!("t{i}"))).collect();
        std::fs::write(dir.0.join("a.log"), a).unwrap();
        std::fs::write(dir.0.join("b.log"), b).unwrap();
        let app = test_app(None);
        let st = || main_window(&app);
        st().store_write().open(std::slice::from_ref(&dir.0)).unwrap();
        let summary = st().store_read().summary();
        let b_id = summary.sources.iter().find(|s| s.name.ends_with("b.log")).unwrap().id;
        let a_id = summary.sources.iter().find(|s| s.name.ends_with("a.log")).unwrap().id;

        let req: Option<SuggestRequest> = serde_json::from_value(json!({ "reference": a_id })).unwrap();
        let s = block(suggest_offsets(st(), req)).unwrap();
        assert_eq!(s.suggestions[0].suggested_ms, 1_200);
        assert_eq!(block(suggest_offsets(st(), None)).unwrap().suggestions.len(), 1);

        let req: SetOffsetRequest =
            serde_json::from_value(json!({ "kind": "manual", "source": b_id, "offsetMs": 1_200, "track": [0] })).unwrap();
        assert_eq!(
            req.spec,
            OffsetSpec::Manual {
                source: b_id,
                offset_ms: 1_200
            }
        );
        let change = block(set_source_offset(st(), OpId(Some("op-skew".into())), req)).unwrap();
        let e = crate::tools::tests::domain_event("source.offsetSet", "op-skew");
        assert_eq!(
            (e.props["SourceId"].clone(), e.props["OffsetMs"].as_str()),
            (b_id.to_string(), "1200")
        );
        assert_eq!((change.offset_ms, change.tracked.len()), (1_200, 1));
        let wire = serde_json::to_value(&change).unwrap();
        assert_eq!(wire["offsetMs"], 1_200);
        assert!(wire.get("idsChangedFrom").is_some());

        let too_big: SetOffsetRequest =
            serde_json::from_value(json!({ "kind": "manual", "source": b_id, "offsetMs": 999_999_999_999i64 })).unwrap();
        assert!(block(set_source_offset(st(), OpId::default(), too_big))
            .unwrap_err()
            .contains("too large"));
        let mut many: SetOffsetRequest = serde_json::from_value(json!({ "kind": "align", "reference": 0, "target": 1 })).unwrap();
        many.track = vec![0; MAX_TRACKED + 1];
        assert!(block(set_source_offset(st(), OpId::default(), many))
            .unwrap_err()
            .contains("too many"));
        // The offset is saved with the project and restored when it is opened.
        let project = dir.0.join("skew.imtproj");
        crate::paths::allow_under(app.handle(), "main", &dir.0);
        let path = project.to_string_lossy().into_owned();
        block(crate::workspace_commands::save_project(
            app.handle().clone(),
            st(),
            crate::paths::WindowGrants::of(app.handle(), "main"),
            OpId::default(),
            path.clone(),
            "Skew".into(),
            json!({}),
            None,
            None,
        ))
        .unwrap();
        let saved: serde_json::Value = serde_json::from_str(&std::fs::read_to_string(&project).unwrap()).unwrap();
        assert_eq!(saved["clock"]["b.log"], 1_200);
        block(crate::workspace_commands::clear_workspace(st())).unwrap();
        let opened = block(crate::workspace_commands::open_project(
            app.handle().clone(),
            st(),
            crate::paths::WindowGrants::of(app.handle(), "main"),
            OpId::default(),
            path,
        ))
        .unwrap();
        let sources = opened["summary"]["sources"].as_array().unwrap();
        let b = sources.iter().find(|s| s["name"].as_str().unwrap().ends_with("b.log")).unwrap();
        assert_eq!(b["offsetMs"], 1_200);

        // Unknown kinds are refused when the payload is parsed.
        assert!(serde_json::from_value::<SetOffsetRequest>(json!({ "kind": "guess", "source": 1 })).is_err());
    }
}
