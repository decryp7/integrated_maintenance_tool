//! IMT's side of the store worker (ADR 0014 §1): one worker process per app instance hosts the DuckDB stores of
//! all windows and their full-text indexes. [`client`] starts and supervises it and implements the store ports
//! over it ([`RemoteStore`]); [`protocol`] is what travels between them; [`arrow_wire`] carries event batches.
//!
//! This crate links no database: the app depends on it, the worker binary (`imt-store-worker`) on the store.

pub mod allocator;
pub mod arrow_wire;
pub mod client;
pub mod protocol;

pub use client::{RemoteStore, StoreWorker, WorkerConfig, WorkerLaunch};
pub use protocol::{MemoryMode, StoreTuning};
