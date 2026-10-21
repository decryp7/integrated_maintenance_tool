//! Clock-skew correction commands (per window): set a source's time offset (manually or by aligning two
//! events) and suggest offsets from shared correlation ids. The store keeps offsets in the project
//! (`project.toml`) but does not apply them to its events yet (ADR 0014 6c follow-up): both commands answer
//! `unsupported` with a message until it does.

use crate::activity::OpId;
use crate::windows::CurrentWindow;
use crate::{CmdError, CmdResult, ErrorCode};
use imt_core::skew::{OffsetChange, SetOffsetRequest, SuggestRequest, SuggestResult, MAX_TRACKED};

fn not_yet() -> CmdError {
    CmdError::new(
        ErrorCode::Unsupported,
        "clock-skew correction is not available with the new log store yet",
    )
    .hint("Shift a source's times with a shift_time step in the profile's [[pipeline]] meanwhile.")
}

/// Sets a source's clock offset and moves its events.
#[tauri::command]
pub(crate) async fn set_source_offset(state: CurrentWindow, op: OpId, request: SetOffsetRequest) -> CmdResult<OffsetChange> {
    if request.track.len() > MAX_TRACKED {
        return Err(crate::CmdError::from(format!(
            "too many tracked events ({}; at most {MAX_TRACKED})",
            request.track.len()
        )));
    }
    let _ = (state, op);
    Err(not_yet())
}

/// Offsets that would align each source with a reference, from events sharing a correlation id.
#[tauri::command]
pub(crate) async fn suggest_offsets(state: CurrentWindow, request: Option<SuggestRequest>) -> CmdResult<SuggestResult> {
    let _ = (state, request);
    Err(not_yet())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, test_app};

    #[test]
    fn skew_correction_says_it_is_not_available_yet() {
        let app = test_app(None);
        let e = block(suggest_offsets(main_window(&app), None)).unwrap_err();
        assert_eq!(e.code, ErrorCode::Unsupported);
        assert!(e.hint.is_some());
        let req: SetOffsetRequest = serde_json::from_value(serde_json::json!({"kind": "manual", "source": 0, "offsetMs": 5})).unwrap();
        assert_eq!(
            block(set_source_offset(main_window(&app), OpId::default(), req)).unwrap_err().code,
            ErrorCode::Unsupported
        );
    }
}
