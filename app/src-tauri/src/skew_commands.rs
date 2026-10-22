//! Clock-skew correction commands (per window): set a source's time offset (manually or by aligning two
//! events) and suggest offsets from shared correlation ids. The store keeps the offsets with its events (and in the
//! project) and moves a file's events when its offset changes; event ids stay.

use crate::activity::OpId;
use crate::windows::CurrentWindow;
use crate::CmdResult;
use imt_core::skew::{OffsetChange, SetOffsetRequest, SuggestRequest, SuggestResult};

/// Sets a source's clock offset and moves its events.
#[tauri::command]
pub(crate) async fn set_source_offset(state: CurrentWindow, _op: OpId, request: SetOffsetRequest) -> CmdResult<OffsetChange> {
    let win = state.arc().clone();
    let change = crate::engine::with_store(&state, move |s| {
        let change = s.set_source_offset(&request)?;
        crate::engine::annotate::run(&win, s, &|_| {});
        Ok(change)
    })
    .await?;
    log::info!(Source = u64::from(change.source), OffsetMs = change.offset_ms; "Clock offset set");
    Ok(change)
}

/// Offsets that would align each source with a reference, from events sharing a correlation id.
#[tauri::command]
pub(crate) async fn suggest_offsets(state: CurrentWindow, request: Option<SuggestRequest>) -> CmdResult<SuggestResult> {
    let request = request.unwrap_or_default();
    crate::engine::with_store(&state, move |s| s.suggest_offsets(&request)).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, test_app, TempDir};
    use crate::ErrorCode;

    #[test]
    fn offsets_are_set_suggested_and_refused_through_the_store() {
        let dir = TempDir::new("skew-cmd");
        for host in ["web01", "web02"] {
            std::fs::create_dir_all(dir.0.join(host)).unwrap();
            let skew: i64 = if host == "web02" { -2_500 } else { 0 };
            let text: String = (0..6i64)
                .map(|n| {
                    let ms = 1_791_972_000_000 + n * 7_000 + skew;
                    let t = chrono::DateTime::from_timestamp_millis(ms)
                        .unwrap()
                        .format("%Y-%m-%dT%H:%M:%S%.3fZ")
                        .to_string();
                    format!("{{\"@t\":\"{t}\",\"@m\":\"{host} {n}\",\"CorrelationId\":\"c{n}\"}}\n")
                })
                .collect();
            std::fs::write(dir.0.join(host).join("app.clef"), text).unwrap();
        }
        let app = test_app(None);
        crate::paths::allow_under(app.handle(), "main", &dir.0);
        block(crate::workspace_commands::open_paths(
            app.handle().clone(),
            main_window(&app),
            vec![dir.0.to_string_lossy().into_owned()],
        ))
        .unwrap();
        let st = || main_window(&app);
        let suggested = block(suggest_offsets(st(), None)).unwrap();
        assert_eq!(suggested.suggestions.len(), 1);
        assert_eq!(suggested.suggestions[0].suggested_ms.abs(), 2_500);
        let target = suggested.suggestions[0].source;
        let req: SetOffsetRequest = serde_json::from_value(
            serde_json::json!({"kind": "manual", "source": target, "offsetMs": suggested.suggestions[0].suggested_ms}),
        )
        .unwrap();
        let change = block(set_source_offset(st(), OpId::default(), req)).unwrap();
        assert_eq!((change.source, change.ids_changed_from), (target, None));
        // The corrected source now agrees with the reference: nothing left to suggest for it.
        let after = block(suggest_offsets(st(), None)).unwrap();
        assert_eq!(after.suggestions[0].current_ms, suggested.suggestions[0].suggested_ms);
        // Refused: too large.
        let big: SetOffsetRequest =
            serde_json::from_value(serde_json::json!({"kind": "manual", "source": target, "offsetMs": 9_999_999_999i64})).unwrap();
        let e = block(set_source_offset(st(), OpId::default(), big)).unwrap_err();
        assert_eq!(e.code, ErrorCode::Invalid);
        assert!(e.message.contains("too large"), "{}", e.message);
    }
}
