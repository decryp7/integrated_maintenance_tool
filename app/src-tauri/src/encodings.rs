//! Text encodings of source files: the choices the UI offers and "Encoding…", which reads one source again as the
//! chosen encoding (an import job of the calling window). Detection and decoding are the engine's
//! (`imt_core::ingest`); the shell only validates the request and starts the job.

use crate::jobs::{JobKind, JobSpec};
use crate::windows::CurrentWindow;
use crate::{CmdError, CmdResult};
use imt_core::batch::SourceId;
use imt_core::ingest::{encoding_options, EncodingChoice, EncodingOption};
use tauri::{AppHandle, Runtime};

/// The encodings the source menu and Settings offer (any other label the engine knows is accepted too).
#[tauri::command]
pub(crate) async fn list_encodings() -> CmdResult<Vec<EncodingOption>> {
    Ok(encoding_options())
}

/// "Encoding…": reads one source again as `encoding` (a label; `None` or `auto`: detected again).
#[tauri::command]
pub(crate) async fn set_source_encoding<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    source_id: u32,
    encoding: Option<String>,
) -> CmdResult<u64> {
    // Refused here with the message that says what to fix; the store checks it again.
    let choice = match encoding.as_deref() {
        Some(label) => EncodingChoice::parse(label).map_err(|m| CmdError::invalid(m).field("encoding"))?,
        None => EncodingChoice::Auto,
    };
    let title = match choice {
        EncodingChoice::Fixed(e) => format!("Read the file as {}", e.label()),
        EncodingChoice::Auto => "Read the file with the detected encoding".to_string(),
    };
    let spec = JobSpec::new(JobKind::Import, title);
    crate::engine::import::start(
        &app,
        &state,
        spec,
        Box::new(move |store| store.set_source_encoding(SourceId(source_id), encoding.as_deref())),
        |_| Ok(()),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, test_app};

    #[test]
    fn the_offered_encodings_include_the_legacy_ones() {
        let labels: Vec<String> = block(list_encodings()).unwrap().into_iter().map(|o| o.label).collect();
        for want in ["UTF-8", "UTF-16LE", "Shift_JIS", "GBK", "EUC-KR", "Big5", "windows-1252"] {
            assert!(labels.iter().any(|l| l == want), "{want}");
        }
    }

    #[test]
    fn an_unknown_label_is_refused_before_any_job_starts() {
        let app = test_app(None);
        let err = block(set_source_encoding(
            app.handle().clone(),
            main_window(&app),
            1,
            Some("klingon".into()),
        ))
        .unwrap_err();
        assert!(err.message.contains("klingon"), "{}", err.message);
    }
}
