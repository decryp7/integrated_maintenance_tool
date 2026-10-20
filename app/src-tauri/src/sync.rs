//! The shell's small concurrency helpers, in one place (they were copied into a dozen modules):
//! poison-recovering lock guards ([`lock`], [`read`], [`write`]) and [`blocking`], which runs work off the
//! async runtime and turns a crash of it into an error instead of taking the command down.
//!
//! A lock is never `unwrap()`ed: a panic while it was held leaves the data as it was, and the shell goes on
//! with it (AGENTS.md, Robustness).

use crate::error::{CmdError, CmdResult};
use crate::ErrorCode;
use std::sync::{Mutex, MutexGuard, RwLock, RwLockReadGuard, RwLockWriteGuard};

/// Locks `m`, recovering from poisoning.
pub fn lock<T: ?Sized>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

/// Read-locks `l`, recovering from poisoning.
pub fn read<T: ?Sized>(l: &RwLock<T>) -> RwLockReadGuard<'_, T> {
    l.read().unwrap_or_else(|p| p.into_inner())
}

/// Write-locks `l`, recovering from poisoning.
pub fn write<T: ?Sized>(l: &RwLock<T>) -> RwLockWriteGuard<'_, T> {
    l.write().unwrap_or_else(|p| p.into_inner())
}

/// Runs `f` on the blocking pool; a panic in it becomes an `internal` error naming `what`.
pub async fn blocking<T: Send + 'static, E: From<CmdError> + Send + 'static>(
    what: &str,
    f: impl FnOnce() -> Result<T, E> + Send + 'static,
) -> Result<T, E> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| E::from(CmdError::new(ErrorCode::Internal, format!("{what} stopped unexpectedly ({e})"))))?
}

/// [`blocking`] for work that fails with `anyhow` (its whole chain becomes the message).
pub async fn blocking_anyhow<T: Send + 'static>(what: &str, f: impl FnOnce() -> anyhow::Result<T> + Send + 'static) -> CmdResult<T> {
    blocking(what, move || f().map_err(|e| CmdError::failed(format!("{e:#}")))).await
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locks_recover_from_poisoning_and_crashes_become_errors() {
        let m = std::sync::Arc::new(Mutex::new(1));
        let l = std::sync::Arc::new(RwLock::new(2));
        let (m2, l2) = (m.clone(), l.clone());
        let _ = std::thread::spawn(move || {
            let _a = m2.lock().unwrap();
            let _b = l2.write().unwrap();
            panic!("poison both");
        })
        .join();
        *lock(&m) += 1;
        *write(&l) += 1;
        assert_eq!((*lock(&m), *read(&l)), (2, 3));
        let ok: CmdResult<u8> = crate::test_support::block(blocking("work", || Ok(7)));
        assert_eq!(ok.unwrap(), 7);
        let crashed: CmdResult<u8> = crate::test_support::block(blocking("the import", || panic!("boom")));
        let e = crashed.unwrap_err();
        assert_eq!(e.code, ErrorCode::Internal);
        assert!(e.contains("the import stopped unexpectedly"), "{e}");
        let failed = crate::test_support::block(blocking_anyhow("x", || -> anyhow::Result<u8> {
            Err(anyhow::anyhow!("inner").context("outer"))
        }));
        assert!(failed.unwrap_err().contains("outer: inner"));
    }
}
