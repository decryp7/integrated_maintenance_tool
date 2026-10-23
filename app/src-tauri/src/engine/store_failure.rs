//! Why the log store could not start, in words for the user. The raw chain ("the store worker failed 3 times ...
//! signal: 6 (SIGABRT)") goes into the error's technical detail; the message and the next step come from the cause,
//! classified from that chain and from what the worker wrote on its standard error (the dynamic linker's refusal
//! is only there).

/// The cause of a store that cannot start.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StoreFailure {
    /// The OS refused to load the bundled database library (macOS library validation, a blocked or damaged file).
    LibraryRefused,
    /// The database library or the worker is not where the worker looks for it.
    LibraryMissing,
    /// The worker crashed repeatedly and Debrief stopped restarting it (until it is reset).
    Quarantined,
    /// The worker stopped unexpectedly.
    Crashed,
    /// Anything else.
    Other,
}

impl StoreFailure {
    /// Classifies a failure from the error text and the worker's recent standard error.
    pub fn classify(error: &str, stderr: &str) -> StoreFailure {
        let all = format!("{stderr}\n{error}").to_lowercase();
        let has = |s: &str| all.contains(s);
        if has("different team ids")
            || has("not valid for use in process")
            || has("library validation")
            || (has("code signature") && has("library not loaded"))
            || has("code signature invalid")
        {
            StoreFailure::LibraryRefused
        } else if has("library not loaded")
            || has("image not found")
            || has("cannot open shared object")
            || has("the code execution cannot proceed")
            || (has("was not found") && has("duckdb"))
            || (has("no such file") && has("duckdb"))
        {
            StoreFailure::LibraryMissing
        } else if has("was stopped; reset it") || has("quarantined") {
            StoreFailure::Quarantined
        } else if has("signal:") || has("exited") || has("crash") || has("abort") {
            StoreFailure::Crashed
        } else {
            StoreFailure::Other
        }
    }

    /// What happened, in one plain sentence.
    pub fn message(self) -> &'static str {
        match self {
            StoreFailure::LibraryRefused => {
                "Debrief's log store couldn't start: the system refused to load a component that ships with Debrief."
            }
            StoreFailure::LibraryMissing => "Debrief's log store couldn't start: a component it needs is missing.",
            StoreFailure::Quarantined => "Debrief's log store was stopped after it failed several times in a row.",
            StoreFailure::Crashed => "Debrief's log store stopped unexpectedly.",
            StoreFailure::Other => "Debrief's log store couldn't start.",
        }
    }

    /// What to do next.
    pub fn hint(self) -> &'static str {
        match self {
            StoreFailure::LibraryRefused => {
                "Install the latest Debrief over this copy; if it persists, report the problem and include the details."
            }
            StoreFailure::LibraryMissing => "Reinstall Debrief: the store worker and its database library must sit next to the app.",
            StoreFailure::Quarantined => "Choose Try again to restart it. If it stops again, report the problem and include the details.",
            StoreFailure::Crashed => "Choose Try again. If it keeps stopping, report the problem and include the details.",
            StoreFailure::Other => "Choose Try again. If it keeps failing, report the problem and include the details.",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHAIN: &str = "store: the store worker stopped (the store worker failed 3 times in 5 minutes (last: signal: 6 (SIGABRT)) and was stopped; reset it to try again)";

    #[test]
    fn the_dynamic_linkers_library_validation_refusal_is_recognised_from_the_workers_stderr() {
        let stderr = "dyld[1]: Library not loaded: @rpath/libduckdb.dylib\n  Reason: tried: '/x/libduckdb.dylib' (code signature in <A> '/x/libduckdb.dylib' not valid for use in process: mapping process and mapped file (non-platform) have different Team IDs)";
        assert_eq!(StoreFailure::classify(CHAIN, stderr), StoreFailure::LibraryRefused);
    }

    #[test]
    fn a_missing_library_a_quarantine_and_a_crash_are_told_apart() {
        assert_eq!(
            StoreFailure::classify(CHAIN, "dyld: Library not loaded: @rpath/libduckdb.dylib\n Reason: image not found"),
            StoreFailure::LibraryMissing
        );
        assert_eq!(
            StoreFailure::classify(
                "error while loading shared libraries: libduckdb.so: cannot open shared object file",
                ""
            ),
            StoreFailure::LibraryMissing
        );
        assert_eq!(StoreFailure::classify(CHAIN, ""), StoreFailure::Quarantined);
        assert_eq!(
            StoreFailure::classify("the store worker exited (signal: 11)", ""),
            StoreFailure::Crashed
        );
        assert_eq!(StoreFailure::classify("something odd", ""), StoreFailure::Other);
    }

    #[test]
    fn every_cause_has_a_plain_message_and_a_next_step_without_internals() {
        use StoreFailure::*;
        for f in [LibraryRefused, LibraryMissing, Quarantined, Crashed, Other] {
            for text in [f.message(), f.hint()] {
                assert!(
                    !text.contains("SIGABRT") && !text.contains("signal") && !text.contains("try again;"),
                    "{text}"
                );
            }
        }
        assert!(!LibraryRefused.hint().contains("Reinstall"));
        assert!(LibraryMissing.hint().contains("Reinstall"));
    }
}
