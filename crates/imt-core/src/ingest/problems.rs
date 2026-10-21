//! Import problems, aggregated per kind (ADR 0014 §5). The types are part of the plugin contract
//! (`imt_plugin_api::problems`): plugin importers report them too.

pub use imt_plugin_api::problems::{Problem, ProblemKind, Problems, MAX_SAMPLES};
