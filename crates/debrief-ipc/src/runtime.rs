//! The runtime every connection of a process runs on: two I/O threads (frames are small and the work happens on
//! blocking threads, see [`crate::peer`]). Started on first use.

use std::future::Future;
use std::sync::OnceLock;

/// The shared runtime, or why it could not start.
pub fn shared() -> Result<&'static tokio::runtime::Runtime, String> {
    static RT: OnceLock<Result<tokio::runtime::Runtime, String>> = OnceLock::new();
    RT.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .thread_name("debrief-ipc")
            .enable_all()
            .build()
            .map_err(|e| format!("cannot start the IPC runtime: {e}"))
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// Runs `fut` on the shared runtime and waits for it on this thread. Works from any thread, including a
/// thread of another runtime (it never calls `block_on` there).
pub fn run<F>(fut: F) -> Result<F::Output, String>
where
    F: Future + Send + 'static,
    F::Output: Send + 'static,
{
    let rt = shared()?;
    let (tx, rx) = std::sync::mpsc::channel();
    rt.spawn(async move {
        let _ = tx.send(fut.await);
    });
    rx.recv().map_err(|_| "the IPC runtime stopped".to_string())
}

#[cfg(test)]
mod tests {
    #[test]
    fn futures_run_from_any_thread() {
        assert_eq!(super::run(async { 2 + 2 }).unwrap(), 4);
        // Also from inside another runtime.
        let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
        assert_eq!(rt.block_on(async { super::run(async { 5 }).unwrap() }), 5);
    }
}
