//! The IMT inference worker as a library (the binary is `src/main.rs`): a [`imt_ai::worker::WorkerBackend`]
//! that runs the official prebuilt llama.cpp `llama-server` as a supervised child (ADR 0009).
//!
//! - [`variants`], [`locate`]: which builds are bundled (`llama/<id>/`) or configured, and their order;
//! - [`devices`]: what `llama-server --list-devices` reports;
//! - [`child`]: starting the server (empty environment, random key, lifetime tied to the worker);
//! - [`http`], [`completion`]: the local HTTP client and the `/completion` wire format;
//! - [`server`]: the backend; [`listing`]: the `--list-builds` report for Settings → AI.

pub mod child;
pub mod completion;
pub mod devices;
pub mod http;
pub mod listing;
pub mod locate;
pub mod server;
pub mod variants;
