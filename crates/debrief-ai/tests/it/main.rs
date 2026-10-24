//! The integration tests of this crate, one binary (linking one test binary per file cost more than running them).
//! Each module is a former `tests/<name>.rs`; run one with `cargo test --test it <name>::`.

mod common;
#[path = "support/duck_store.rs"]
mod duck_store;

mod assistant_usecases;
mod browser;
mod chat;
mod find_names;
mod frames;
mod hf_download;
mod hf_live;
mod incident_ai;
mod names_local;
mod net_tls;
mod parser_generation;
mod properties_knowledge;
mod query_transparency;
mod real_worker;
mod runtimes;
mod segmented_download;
mod setup;
mod sql_tool;
mod tools;
mod web;
mod worker;
mod worker_memory;
