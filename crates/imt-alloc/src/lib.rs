//! The global allocator of IMT's binaries that parse and analyse (the app, the store worker, the plugins
//! process): mimalloc v3 (user decision 2026-10-21; measurements in ADR 0014's performance notes).
//!
//! Each binary declares it and tunes it first thing in `main`:
//!
//! ```ignore
//! #[global_allocator]
//! static GLOBAL: imt_alloc::MiMalloc = imt_alloc::MiMalloc;
//!
//! fn main() {
//!     imt_alloc::tune();
//!     // ...
//! }
//! ```
//!
//! v3 keeps freed memory for a second (`purge_delay` = 1000 ms) before giving it back to the OS, which during an
//! import added 150-200 MB to the worker's peak (643-690 MB, over ADR 0014's 600 MB budget). [`tune`] sets v2's
//! timing instead (10 ms, arenas 10x that): measured as fast as v2 and back under the budget; purging at once
//! (0 ms) halved the peak but cost 20-40% of the import speed. The options are set in code because the
//! workers start with a cleared environment (no `MIMALLOC_*` variables).

pub use mimalloc::MiMalloc;

use std::os::raw::{c_int, c_long};

/// mimalloc v3's option ids (`mi_option_e` in `mimalloc.h`; the Rust bindings list v2's). The test pins them
/// by their defaults.
const PURGE_DELAY: c_int = 15;
const ARENA_PURGE_MULT: c_int = 24;

/// Milliseconds before freed memory goes back to the OS.
const PURGE_DELAY_MS: c_long = 10;
/// Arenas wait this many times as long.
const ARENA_PURGE_FACTOR: c_long = 10;

/// Sets the allocator's purge timing (see the module docs); call it first thing in `main`.
pub fn tune() {
    // SAFETY: plain option setters of the linked mimalloc; any value is valid for these options.
    unsafe {
        libmimalloc_sys::mi_option_set(PURGE_DELAY, PURGE_DELAY_MS);
        libmimalloc_sys::mi_option_set(ARENA_PURGE_MULT, ARENA_PURGE_FACTOR);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn get(option: c_int) -> c_long {
        // SAFETY: a plain option getter.
        unsafe { libmimalloc_sys::mi_option_get(option) }
    }

    #[test]
    fn option_ids_are_v3s_and_the_tuning_applies() {
        // v3's defaults around the ids used: purge_delay 1000 ms, destroy_on_exit 0, arena_reserve 1 GiB (in
        // KiB), arena_purge_mult 1. A different mimalloc version (or v2) shifts these and fails here.
        assert_eq!(get(PURGE_DELAY), 1000, "purge_delay");
        assert_eq!(get(22), 0, "destroy_on_exit");
        assert_eq!(get(23), 1024 * 1024, "arena_reserve");
        assert_eq!(get(ARENA_PURGE_MULT), 1, "arena_purge_mult");
        tune();
        assert_eq!((get(PURGE_DELAY), get(ARENA_PURGE_MULT)), (10, 10));
    }
}
