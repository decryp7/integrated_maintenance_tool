//! The store worker (ADR 0014 §1): one process per app instance hosts the DuckDB stores of all windows and
//! their full-text indexes. [`server`] is the worker side; IMT's side (the protocol, the supervisor and
//! `RemoteStore`) is `imt-store-client`, which links no database.

pub mod server;

pub use imt_store_client::{protocol, MemoryMode, RemoteStore, StoreWorker, WorkerConfig};
