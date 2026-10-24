//! Paste as log: pasted or dropped text becomes a source of the window's project. The engine checks and names it
//! (`debrief_core::paste`); the text is written into the project folder's `pasted/` folder (so a saved project carries it)
//! and opened like any file, so the normal importer detection decides how it is read.

use crate::activity::{domain_event, OpId};
use crate::windows::CurrentWindow;
use crate::{CmdError, CmdResult, ErrorCode};
use debrief_core::paste::{prepare, PASTE_DIR};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Runtime};

/// Writes `text` into `dir/pasted` under a safe, unique name (taking `name` as the wish); returns the file.
fn store_text(dir: &Path, name: Option<&str>, text: &str) -> CmdResult<PathBuf> {
    let folder = dir.join(PASTE_DIR);
    let io = |what: &str, e: std::io::Error| {
        CmdError::new(ErrorCode::Io, format!("cannot {what}: {e}")).hint("Check the disk space and that Debrief's data folder is writable.")
    };
    std::fs::create_dir_all(&folder).map_err(|e| io("create the folder for pasted logs", e))?;
    let taken: HashSet<String> = std::fs::read_dir(&folder)
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().into_string().ok()).collect())
        .unwrap_or_default();
    let pasted = prepare(name, text, &taken).map_err(|e| CmdError::invalid(e.to_string()).field("text"))?;
    let file = folder.join(&pasted.file_name);
    let tmp = folder.join(format!("{}.part", pasted.file_name));
    std::fs::write(&tmp, &pasted.bytes)
        .and_then(|_| std::fs::rename(&tmp, &file))
        .map_err(|e| {
            let _ = std::fs::remove_file(&tmp);
            io("save the pasted text", e)
        })?;
    Ok(file)
}

/// Adds pasted (or dropped) text as a source named `name` (any name; made safe and unique) and opens it. The text
/// may be at most `debrief_core::paste::MAX_PASTE_BYTES`.
#[tauri::command]
pub(crate) async fn paste_log<R: Runtime>(
    app: AppHandle<R>,
    state: CurrentWindow,
    op: OpId,
    text: String,
    name: Option<String>,
) -> CmdResult<u64> {
    // What the engine refuses is refused here, before anything is queued.
    prepare(name.as_deref(), &text, &HashSet::new()).map_err(|e| CmdError::invalid(e.to_string()).field("text"))?;
    // Starting the store, writing the text and opening it are the import job's: the command returns its id at once.
    let spec = crate::jobs::JobSpec::new(crate::jobs::JobKind::Import, "Open the pasted text").window(state.label());
    let (controls, flag) = crate::engine::import::controls();
    let (handle, win) = (app.clone(), state.clone());
    crate::jobs::start_value(&app, spec, controls, move |ctx: &crate::jobs::JobCtx| {
        let dir = win.engine.store()?.store_dir();
        let file = store_text(&dir, name.as_deref(), &text)?;
        // Debrief's own file: opened without a grant check, like the files of an unpacked bundle.
        let summary = crate::workspace_commands::open_in_job(&handle, &win, ctx, &flag, vec![file.to_string_lossy().into_owned()], false)?;
        domain_event("source.pasted", op.as_deref(), &[]);
        Ok(crate::jobs::Done::new(summary).with_result(crate::jobs::JobResult::workspace()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{block, main_window, test_app, TempDir};

    #[test]
    fn text_is_stored_under_a_unique_safe_name() {
        let dir = TempDir::new("paste-store");
        let a = store_text(&dir.0, Some("../evil name"), "x 1\n").unwrap();
        let b = store_text(&dir.0, Some("../evil name"), "x 2").unwrap();
        assert_eq!(a.parent().unwrap(), dir.0.join(PASTE_DIR));
        assert_eq!(a.file_name().unwrap(), "evil name.log");
        assert_eq!(b.file_name().unwrap(), "evil name (2).log");
        assert_eq!(std::fs::read_to_string(&b).unwrap(), "x 2\n");
        // No temporary file is left behind.
        assert!(std::fs::read_dir(dir.0.join(PASTE_DIR))
            .unwrap()
            .flatten()
            .all(|e| !e.file_name().to_string_lossy().ends_with(".part")));
    }

    #[test]
    fn empty_and_oversized_text_is_refused_with_what_to_do() {
        let dir = TempDir::new("paste-refuse");
        let e = store_text(&dir.0, None, "  \n").unwrap_err();
        assert_eq!(e.code, ErrorCode::Invalid);
        let big = "a".repeat(debrief_core::paste::MAX_PASTE_BYTES + 1);
        assert!(store_text(&dir.0, None, &big).unwrap_err().message.contains("open the file"));
        assert!(!dir.0.join(PASTE_DIR).join("pasted-log.log").exists());
    }

    #[test]
    fn an_unwritable_project_folder_is_an_io_error_with_a_hint() {
        let dir = TempDir::new("paste-io");
        // A file where the folder should be.
        std::fs::write(dir.0.join(PASTE_DIR), "in the way").unwrap();
        let e = store_text(&dir.0, None, "a\n").unwrap_err();
        assert_eq!(e.code, ErrorCode::Io);
        assert!(e.hint.is_some());
    }

    #[test]
    fn pasted_text_opens_like_a_file() {
        let app = test_app(None);
        let summary = crate::test_support::finished(
            &app,
            crate::test_support::returns_fast(
                app.handle(),
                "paste_log",
                paste_log(
                    app.handle().clone(),
                    main_window(&app),
                    OpId::default(),
                    "2026-10-14 10:00:00 INFO first\n2026-10-14 10:00:05 ERROR second\n".into(),
                    Some("web01 errors".into()),
                ),
            ),
        )
        .unwrap();
        assert_eq!(summary["total"], 2);
        assert_eq!(summary["sources"].as_array().unwrap().len(), 1);
        // Listed under the project's `pasted` folder.
        assert_eq!(summary["sources"][0]["name"], "pasted/web01 errors.log");
        // Pasting the same name again is a second source.
        let again = crate::test_support::finished(
            &app,
            block(paste_log(
                app.handle().clone(),
                main_window(&app),
                OpId::default(),
                "2026-10-14 11:00:00 INFO third\n".into(),
                Some("web01 errors".into()),
            )),
        )
        .unwrap();
        assert_eq!(again["sources"].as_array().unwrap().len(), 2);
        assert!(again["sources"]
            .as_array()
            .unwrap()
            .iter()
            .any(|s| s["name"] == "pasted/web01 errors (2).log"));
    }
}
