//! DuckDB's own allocator in the store worker. Its Linux builds bring jemalloc (IMT's Rust code uses mimalloc),
//! which by default has as many arenas as the machine has CPUs. Each DuckDB thread allocates from its own arena and
//! keeps what it freed there for the decay time (1 s): pages one thread freed cannot serve another, so after every
//! statement each thread's arena holds about its own working set again. On the 1M-event corpus with 4 DuckDB
//! threads this kept 70-90 MB resident (jemalloc's `stats.resident`) for 10-15 MB in use, more on machines with
//! more CPUs; two arenas let the threads reuse each other's pages.
//!
//! jemalloc reads its options once, when the library first allocates — possibly while libduckdb loads, before the
//! worker's `main` runs — from the environment variable [`CONF_VAR`]. So the variable is part of the worker's
//! environment at spawn ([`worker_env`]), never set from inside the worker. Where DuckDB uses the system allocator
//! (Windows, macOS) there is nothing to set.

/// Arenas of DuckDB's allocator by default (the worker's tuning may name others).
pub const ARENAS: usize = 2;

/// The most arenas a tuning may name.
pub const MAX_ARENAS: usize = 256;

/// The environment variable DuckDB's jemalloc reads its options from (after DuckDB's own).
pub const CONF_VAR: &str = "DUCKDB_JE_MALLOC_CONF";

/// Decay of freed pages: jemalloc's defaults (10 s) kept an analysis's working set (60-70 MB) resident while the next
/// analysis started, so a run of analyses peaked 20 MB higher (Linux, ADR 0014 memory notes); 200 ms, and no muzzy stage.
const DECAY: &str = "dirty_decay_ms:200,muzzy_decay_ms:0";

/// The options for `arenas` arenas (bounded to 1..=[`MAX_ARENAS`]), or the `existing` options when they are set
/// (a developer's experiment, passed on as it is).
pub fn conf(arenas: usize, existing: Option<&str>) -> String {
    match existing.map(str::trim) {
        Some(e) if !e.is_empty() => e.to_string(),
        _ => format!("narenas:{},{DECAY}", arenas.clamp(1, MAX_ARENAS)),
    }
}

/// The variable the worker starts with for `arenas` (default [`ARENAS`]) on Linux; `None` elsewhere.
pub fn worker_env(arenas: Option<usize>) -> Option<(String, String)> {
    cfg!(target_os = "linux").then(|| {
        let existing = std::env::var(CONF_VAR).ok();
        (CONF_VAR.to_string(), conf(arenas.unwrap_or(ARENAS), existing.as_deref()))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_options_name_the_arenas_within_bounds_and_keep_existing_ones() {
        assert_eq!(conf(ARENAS, None), "narenas:2,dirty_decay_ms:200,muzzy_decay_ms:0");
        assert!(conf(0, None).starts_with("narenas:1,"));
        assert!(conf(100_000, None).starts_with("narenas:256,"));
        assert!(conf(2, Some("")).starts_with("narenas:2,"));
        assert!(conf(2, Some("  ")).starts_with("narenas:2,"));
        assert_eq!(conf(2, Some("narenas:8,dirty_decay_ms:0")), "narenas:8,dirty_decay_ms:0");
    }

    #[test]
    fn the_worker_gets_the_variable_on_linux_only() {
        let env = worker_env(Some(3));
        if cfg!(target_os = "linux") {
            let (k, v) = env.expect("set on Linux");
            assert_eq!(k, CONF_VAR);
            assert!(
                v.starts_with("narenas:3,") || std::env::var(CONF_VAR).is_ok_and(|e| !e.trim().is_empty()),
                "{v}"
            );
        } else {
            assert_eq!(env, None);
        }
    }
}
