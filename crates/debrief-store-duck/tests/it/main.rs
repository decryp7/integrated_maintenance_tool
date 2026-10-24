//! The integration tests of this crate, one binary (linking one test binary per file cost more than running them).
//! Each module is a former `tests/<name>.rs`; run one with `cargo test --test it <name>::`.
//! The tests that measure the process (its resident memory, with mimalloc as the global allocator like the store worker)
//! stay binaries of their own: `memory`, `bench`, `bench_reads` and `analysis_memory`.

// Shared with the measuring tests, which keep their own binaries.
#[path = "../common/mod.rs"]
mod common;

mod analysis;
mod annotators;
mod catalogue;
mod differential;
mod encodings;
mod import_phases;
mod large_folder;
mod layout_bakeoff;
mod live;
mod long_ops;
mod outliers;
mod projects;
mod qlang;
mod redacted_copy;
mod runtime_lib;
mod search;
mod segments;
mod skew;
mod span_rules;
mod store_pane;
mod streaming_import;
mod tail;
mod time_window;
