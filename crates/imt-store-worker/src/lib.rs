//! The store worker (ADR 0014 §1): one process per app instance hosts the DuckDB stores of all windows and
//! their full-text indexes. [`server`] is the worker side, [`client`] the app side ([`client::RemoteStore`]
//! implements the store ports), [`protocol`] what travels between them.

pub mod client;
pub mod protocol;
pub mod server;

pub use client::{RemoteStore, StoreWorker, WorkerConfig};
