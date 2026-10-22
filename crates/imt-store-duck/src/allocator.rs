//! DuckDB's own allocator. Its Linux builds bring jemalloc (IMT's Rust code uses mimalloc), which by default
//! has as many arenas as the machine has CPUs. Each DuckDB thread allocates from its own arena and keeps what it
//! freed there for the decay time (1 s): pages one thread freed cannot serve another, so after every statement
//! each thread's arena holds about its own working set again. On the 1M-event corpus with 4 DuckDB threads this
//! kept 70-90 MB resident (jemalloc's `stats.resident`) for 10-15 MB in use, more on machines with more CPUs; two
//! arenas let the threads reuse each other's pages (25 MB less growth during anomaly detection, as fast).
//!
//! jemalloc reads its options once, when DuckDB first allocates, from DuckDB's own defaults and then from the
//! environment variable [`CONF_VAR`]; [`configure`] sets it, so it must run before the first database opens.
//! Where DuckDB uses the system allocator (Windows, macOS) the variable is ignored.

/// Arenas of DuckDB's allocator by default (the worker's tuning may name others).
pub const ARENAS: usize = 2;

/// The most arenas a tuning may name.
pub const MAX_ARENAS: usize = 256;

/// The environment variable DuckDB's jemalloc reads its options from (after DuckDB's own).
pub const CONF_VAR: &str = "DUCKDB_JE_MALLOC_CONF";

/// The options for `arenas` arenas (bounded to 1..=[`MAX_ARENAS`]), or `None` when `existing` options are
/// already set (a developer's experiment, kept as it is).
pub fn conf(arenas: usize, existing: Option<&str>) -> Option<String> {
    if existing.is_some_and(|e| !e.trim().is_empty()) {
        return None;
    }
    Some(format!("narenas:{}", arenas.clamp(1, MAX_ARENAS)))
}

/// Gives DuckDB's allocator `arenas` arenas. Call it first thing in `main`, while the process has one thread
/// and before any database opens (later it has no effect: jemalloc has read its options).
pub fn configure(arenas: usize) {
    if let Some(value) = conf(arenas, std::env::var(CONF_VAR).ok().as_deref()) {
        std::env::set_var(CONF_VAR, value);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_name_the_arenas_within_bounds_and_keep_existing_ones() {
        assert_eq!(conf(ARENAS, None).as_deref(), Some("narenas:2"));
        assert_eq!(conf(0, None).as_deref(), Some("narenas:1"));
        assert_eq!(conf(100_000, None).as_deref(), Some("narenas:256"));
        assert_eq!(conf(2, Some("")).as_deref(), Some("narenas:2"));
        assert_eq!(conf(2, Some("narenas:8,dirty_decay_ms:0")), None);
    }
}
