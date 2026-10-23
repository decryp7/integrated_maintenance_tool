//! A video's frames as a source of the window's workspace (ADR 0033 §4): one event per kept frame, so the timeline,
//! search, filters, Moment and the SQL console see the video like any log. Whenever a record changes (descriptions,
//! notes, alignment, frames) its frames file is rewritten and the source is replaced.

use super::folder_of;
use crate::error::{CmdError, CmdResult};
use crate::jobs::JobCtx;
use crate::windows::WindowState;
use debrief_core::batch::SourceId;
use debrief_core::ports::Store;
use debrief_video::record::VideoRecord;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

fn same_file(a: &str, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(x), Ok(y)) => x == y,
        _ => Path::new(a) == b,
    }
}

/// The workspace source of a frames file, when it is open.
pub(crate) fn source_id(store: &dyn Store, file: &Path) -> Option<u32> {
    store
        .summary()
        .ok()?
        .sources
        .iter()
        .find(|s| same_file(&s.path, file))
        .map(|s| s.id)
}

/// The frames file of `record` in its project folder.
pub(crate) fn frames_file(win: &WindowState, record: &VideoRecord) -> CmdResult<PathBuf> {
    Ok(folder_of(win)?
        .dir(&record.key)
        .map_err(super::cmd)?
        .join(record.source_file_name()))
}

/// Replaces the workspace source of `record`'s frames file with a fresh read of it. Blocking (it waits for the
/// import); progress and Cancel go to `ctx`. Returns the new source's id.
pub(crate) fn replace(win: &WindowState, ctx: &JobCtx, cancel: &AtomicBool, record: &VideoRecord) -> CmdResult<Option<u32>> {
    let file = frames_file(win, record)?;
    let store = win.engine.store()?;
    if let Some(id) = source_id(store.as_ref(), &file) {
        store.remove_source(SourceId(id))?;
    }
    let session = store.open(std::slice::from_ref(&file))?;
    crate::engine::import::follow(win, ctx, cancel, session).map_err(CmdError::from)?;
    Ok(source_id(store.as_ref(), &file))
}

/// [`replace`] outside a job (a quick edit of one video).
pub(crate) fn replace_now(win: &WindowState, record: &VideoRecord) -> CmdResult<Option<u32>> {
    replace(win, &JobCtx::detached(), &AtomicBool::new(false), record)
}

/// Drops the workspace source of a frames file (the video is being removed).
pub(crate) fn forget(win: &WindowState, record: &VideoRecord) -> CmdResult<()> {
    let file = frames_file(win, record)?;
    let store = win.engine.store()?;
    if let Some(id) = source_id(store.as_ref(), &file) {
        store.remove_source(SourceId(id))?;
    }
    Ok(())
}
