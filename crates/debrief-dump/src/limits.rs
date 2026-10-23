//! Bounds of everything a dump or UMDH log may hold. The plugin's readers stop at them (and note it); the engine
//! checks a decoded document against them again, since a plugin process is not trusted to respect them.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Limits {
    /// Threads read from a dump.
    pub max_threads: usize,
    /// Frames kept per thread.
    pub max_frames: usize,
    /// Stack bytes above the stack pointer a walk may read per thread.
    pub max_stack_bytes: u64,
    /// Modules read from a dump.
    pub max_modules: usize,
    /// Handle types listed (the rest are summed into "other").
    pub max_handle_types: usize,
    /// Allocation stacks read from a UMDH log.
    pub max_stacks: usize,
    /// Frames kept per allocation stack.
    pub max_umdh_frames: usize,
    /// Characters of any text taken from a file (paths, names, functions).
    pub max_text: usize,
    /// Notes kept.
    pub max_notes: usize,
    /// Size of a dump or UMDH file that is read whole; larger full dumps are read in parts.
    pub max_read_bytes: u64,
    /// Encoded document size accepted from the plugin.
    pub max_encoded_bytes: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_threads: 4096,
            max_frames: 256,
            max_stack_bytes: 256 << 10,
            max_modules: 4096,
            max_handle_types: 64,
            max_stacks: 200_000,
            max_umdh_frames: 64,
            max_text: 1024,
            max_notes: 64,
            max_read_bytes: 1 << 30,
            max_encoded_bytes: 512 << 20,
        }
    }
}

/// Cuts `s` to `max` characters (with an ellipsis) and drops control characters.
pub fn clip(s: &str, max: usize) -> String {
    let mut chars = s.chars().filter(|c| !c.is_control());
    let mut out: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() && max > 0 {
        out.pop();
        out.push('\u{2026}');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_cuts_and_cleans() {
        assert_eq!(clip("ab\u{0}c", 10), "abc");
        assert_eq!(clip("abcdef", 4), "abc\u{2026}");
        assert_eq!(clip("abcd", 4), "abcd");
        assert_eq!(clip("abc", 0), "");
        assert!(Limits::default().max_frames > 0);
    }
}
