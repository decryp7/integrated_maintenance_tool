//! Resource bounds. Captures are untrusted and can be huge; every table that
//! grows with the input is capped here, and running out is a clear error
//! ([`crate::ProfileError::LimitExceeded`]) instead of an out-of-memory abort.

/// Caps applied while importing and building a capture.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Limits {
    /// Bytes of input an importer accepts (after decompression).
    pub max_input_bytes: u64,
    /// Distinct frames (functions) in one profile.
    pub max_frames: u32,
    /// Distinct stack prefixes (call-tree nodes) in one profile.
    pub max_stack_nodes: u32,
    /// Samples (weighted stack observations) in one profile.
    pub max_samples: u64,
    /// Threads in one profile.
    pub max_threads: u32,
    /// Timeline events (GC, JIT, exceptions, lock waits) in one profile.
    pub max_events: u32,
    /// Frames per stack; deeper stacks keep their outermost frames and are noted.
    pub max_stack_depth: u32,
    /// Characters kept of a function, module or file name.
    pub max_name_len: usize,
    /// Objects in one heap snapshot's graph.
    pub max_objects: u32,
    /// References in one heap snapshot's graph.
    pub max_edges: u64,
    /// Nesting depth of structured input (JSON, protobuf) importers accept.
    pub max_nesting: u32,
}

impl Default for Limits {
    fn default() -> Self {
        Limits {
            max_input_bytes: 1 << 30,
            max_frames: 1_000_000,
            max_stack_nodes: 4_000_000,
            max_samples: 20_000_000,
            max_threads: 65_536,
            max_events: 2_000_000,
            max_stack_depth: 4096,
            max_name_len: 1024,
            max_objects: 20_000_000,
            max_edges: 100_000_000,
            max_nesting: 128,
        }
    }
}

/// Paging and result caps for queries (results are paged, never sent whole).
pub mod query {
    /// Rows per page of any paged table.
    pub const MAX_PAGE: usize = 10_000;
    /// Rectangles in one flame graph layout.
    pub const MAX_RECTS: usize = 200_000;
    /// Buckets of a thread timeline.
    pub const MAX_BUCKETS: usize = 4_000;
    /// Timeline events returned at once.
    pub const MAX_EVENTS: usize = 20_000;
    /// Paths returned by one frame-paths request.
    pub const MAX_FRAME_PATHS: usize = 50;
    /// Nodes on an expanded hot path.
    pub const MAX_HOT_PATH: usize = 1_000;
    /// Patterns in a "just my code" list, and their length.
    pub const MAX_PATTERNS: usize = 64;
    pub const MAX_PATTERN_LEN: usize = 256;
    /// Length of a search text or regex.
    pub const MAX_SEARCH_LEN: usize = 512;
    /// Compiled size of a search regex.
    pub const MAX_REGEX_SIZE: usize = 1 << 20;
    /// Paths to GC roots returned at once, and their length.
    pub const MAX_PATHS: usize = 20;
    pub const MAX_PATH_LEN: usize = 256;
    /// Objects visited while searching paths to roots.
    pub const MAX_PATH_VISITS: usize = 2_000_000;
    /// Call trees kept per store for fast paging (per view).
    pub const TREE_CACHE: usize = 6;
}

/// Truncates `s` to at most `max` characters (on a char boundary).
pub fn clip(s: &str, max: usize) -> String {
    match s.char_indices().nth(max) {
        Some((i, _)) => s[..i].to_string(),
        None => s.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clip_respects_char_boundaries() {
        assert_eq!(clip("héllo", 2), "hé");
        assert_eq!(clip("abc", 10), "abc");
        assert_eq!(clip("", 0), "");
        assert!(Limits::default().max_frames > 50_000);
    }
}
