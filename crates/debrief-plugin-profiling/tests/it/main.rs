//! The integration tests of this crate, one binary (linking one test binary per file cost more than running them).
//! Each module is a former `tests/<name>.rs`; run one with `cargo test --test it <name>::`.

mod gen_json_samples;
mod gen_nettrace;
mod gen_pprof;
mod properties_chrome;
mod properties_folded;
mod properties_heapsnapshot;
mod properties_nettrace;
mod properties_perf;
mod properties_pprof;
mod properties_speedscope;
mod samples;
