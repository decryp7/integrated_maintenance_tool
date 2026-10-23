//! A bounded producer-consumer pipeline for work that is a chain of stages (read, transform, commit).
//!
//! `read` (the caller's thread, usually IO) cuts the input into chunks and feeds a small bounded queue; a `work`
//! stage maps every item of a chunk on a bounded rayon pool (CPU); a single `commit` stage consumes the results
//! on its own thread. Every queue is bounded, so a slow stage back-pressures the ones before it and memory stays
//! at a few chunks. Chunks and the items inside them keep their order through every stage, so the commit stage
//! sees exactly the sequence a plain `for item in input { commit(item, work(&item)) }` would: the output is
//! deterministic and identical to the sequential path. Cancel stops the reader; the queued chunks drain.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::sync_channel;

/// Chunks queued between two stages.
pub const QUEUE_DEPTH: usize = 2;

/// Items per chunk when the caller has no better idea.
pub const DEFAULT_CHUNK: usize = 2048;

/// How a pipeline runs.
pub struct Stages<'a> {
    /// The pool of the CPU stage.
    pub pool: &'a rayon::ThreadPool,
    /// Items per chunk (at least 1).
    pub chunk: usize,
    /// Set to stop reading; whatever was read is still committed.
    pub cancel: Option<&'a AtomicBool>,
}

impl Stages<'_> {
    /// Runs `input` through `work` (parallel, in order) into `commit` (one thread, in order). `commit` also gets
    /// the number of items committed so far, for progress. Returns false when it stopped early because of cancel.
    pub fn run<I, T>(
        &self,
        input: impl Iterator<Item = I>,
        work: impl Fn(&I) -> T + Sync,
        mut commit: impl FnMut(I, T, usize) + Send,
    ) -> bool
    where
        I: Send + Sync,
        T: Send,
    {
        use rayon::prelude::*;
        let chunk = self.chunk.max(1);
        let cancelled = || self.cancel.is_some_and(|c| c.load(Ordering::Relaxed));
        let mut stopped = false;
        std::thread::scope(|s| {
            let (read_tx, read_rx) = sync_channel::<Vec<I>>(QUEUE_DEPTH);
            let (done_tx, done_rx) = sync_channel::<(Vec<I>, Vec<T>)>(QUEUE_DEPTH);
            let work = &work;
            let pool = self.pool;
            s.spawn(move || {
                for items in read_rx {
                    let results: Vec<T> = pool.install(|| items.par_iter().map(work).collect());
                    if done_tx.send((items, results)).is_err() {
                        break;
                    }
                }
            });
            s.spawn(move || {
                let mut n = 0usize;
                for (items, results) in done_rx {
                    for (item, result) in items.into_iter().zip(results) {
                        n += 1;
                        commit(item, result, n);
                    }
                }
            });
            let mut input = input;
            loop {
                if cancelled() {
                    stopped = true;
                    break;
                }
                let items: Vec<I> = input.by_ref().take(chunk).collect();
                if items.is_empty() {
                    break;
                }
                if read_tx.send(items).is_err() {
                    break;
                }
            }
            drop(read_tx);
        });
        !stopped
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;

    fn pool(n: usize) -> rayon::ThreadPool {
        rayon::ThreadPoolBuilder::new().num_threads(n).build().unwrap()
    }

    #[test]
    fn the_commit_sees_the_sequential_order_whatever_the_work_takes() {
        let p = pool(4);
        let stages = Stages {
            pool: &p,
            chunk: 7,
            cancel: None,
        };
        let mut seen = Vec::new();
        let done = stages.run(
            0..1000u32,
            |i| {
                if i % 13 == 0 {
                    std::thread::sleep(std::time::Duration::from_micros(200));
                }
                u64::from(*i) * 3
            },
            |i, t, n| seen.push((i, t, n)),
        );
        assert!(done);
        let want: Vec<_> = (0..1000u32).map(|i| (i, u64::from(i) * 3, i as usize + 1)).collect();
        assert_eq!(seen, want);
    }

    #[test]
    fn an_empty_input_commits_nothing() {
        let p = pool(2);
        let stages = Stages {
            pool: &p,
            chunk: 0,
            cancel: None,
        };
        assert!(stages.run(std::iter::empty::<u8>(), |_| (), |_, (), _| panic!("nothing to commit")));
    }

    #[test]
    fn cancel_stops_reading_and_the_queued_chunks_still_commit_in_order() {
        let p = pool(2);
        let cancel = AtomicBool::new(false);
        let stages = Stages {
            pool: &p,
            chunk: 10,
            cancel: Some(&cancel),
        };
        let read = AtomicUsize::new(0);
        let input = (0..100_000u32).inspect(|_| {
            if read.fetch_add(1, Ordering::Relaxed) == 50 {
                cancel.store(true, Ordering::Relaxed);
            }
        });
        let mut seen = Vec::new();
        let done = stages.run(input, |i| *i, |i, _, _| seen.push(i));
        assert!(!done);
        assert!(
            seen.len() >= 50 && seen.len() < 1000,
            "bounded read-ahead after cancel: {}",
            seen.len()
        );
        assert!(seen.windows(2).all(|w| w[1] == w[0] + 1), "in order, no gaps");
    }
}
