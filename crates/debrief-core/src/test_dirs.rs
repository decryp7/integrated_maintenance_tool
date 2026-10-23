//! Scratch folders for unit tests that clean up even when the test panics.
//!
//! A folder made here lives until the test's thread ends (thread-local drop runs on unwind too), so a helper can
//! return a plain path while the test body keeps using it. CI fails on `debrief-*` folders left in the temp dir
//! (`scripts/check-temp-leaks.mjs`).

use std::cell::RefCell;
use std::path::PathBuf;

thread_local! {
    static KEPT: RefCell<Vec<tempfile::TempDir>> = const { RefCell::new(Vec::new()) };
}

/// A fresh empty folder named `debrief-<prefix>-...` in the OS temp dir, removed when the calling thread ends.
pub(crate) fn scratch_dir(prefix: &str) -> PathBuf {
    let dir = tempfile::Builder::new()
        .prefix(&format!("debrief-{prefix}-"))
        .tempdir()
        .expect("create a scratch folder");
    let path = dir.path().to_path_buf();
    KEPT.with(|k| k.borrow_mut().push(dir));
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_folder_exists_while_the_thread_runs_and_is_removed_after() {
        let path = std::thread::spawn(|| {
            let p = scratch_dir("probe");
            assert!(p.is_dir());
            assert!(p.file_name().unwrap().to_string_lossy().starts_with("debrief-probe-"));
            p
        })
        .join()
        .unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn the_folder_is_removed_after_a_panic() {
        let path = std::thread::spawn(|| {
            let p = scratch_dir("panic");
            std::fs::write(p.join("f"), b"x").unwrap();
            std::panic::resume_unwind(Box::new(p));
        })
        .join()
        .unwrap_err()
        .downcast::<PathBuf>()
        .unwrap();
        assert!(!path.exists());
    }
}
