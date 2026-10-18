//! Resource limits of a script step. Exceeding one at run time stops the script for that
//! record (the record stays as it was and the step counts an error).

use rhai::Engine;

/// Longest script text accepted in a profile (bytes).
pub const MAX_SCRIPT_LEN: usize = 64 * 1024;
/// Operations one record may cost (a loop iteration, a call, an expression...).
pub const MAX_OPERATIONS: u64 = 100_000;
/// Nested function calls.
pub const MAX_CALL_LEVELS: usize = 16;
/// Expression nesting at the top level and inside functions (checked when compiling).
pub const MAX_EXPR_DEPTH: usize = 32;
pub const MAX_FN_EXPR_DEPTH: usize = 16;
/// Longest string a script may build: the longest line an importer accepts.
pub const MAX_STRING_SIZE: usize = crate::importers::MAX_LINE_LEN;
/// Elements in one array, entries in one map.
pub const MAX_ARRAY_SIZE: usize = 10_000;
pub const MAX_MAP_SIZE: usize = 10_000;
/// Variables alive at once, and functions a script may define.
pub const MAX_VARIABLES: usize = 256;
pub const MAX_FUNCTIONS: usize = 64;

pub(crate) fn apply(engine: &mut Engine) {
    engine
        .set_max_operations(MAX_OPERATIONS)
        .set_max_call_levels(MAX_CALL_LEVELS)
        .set_max_expr_depths(MAX_EXPR_DEPTH, MAX_FN_EXPR_DEPTH)
        .set_max_string_size(MAX_STRING_SIZE)
        .set_max_array_size(MAX_ARRAY_SIZE)
        .set_max_map_size(MAX_MAP_SIZE)
        .set_max_variables(MAX_VARIABLES)
        .set_max_functions(MAX_FUNCTIONS);
}
