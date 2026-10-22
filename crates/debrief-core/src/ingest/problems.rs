//! Import problems, aggregated per kind (ADR 0014 §5). The types are part of the plugin contract
//! (`debrief_plugin_api::problems`): plugin importers report them too.

pub use debrief_plugin_api::problems::{Problem, ProblemKind, Problems, MAX_SAMPLES};
