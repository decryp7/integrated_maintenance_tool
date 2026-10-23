//! Update checks for the inference runtimes (llama.cpp builds) and the AI models. Pure: what counts as an
//! update, when a check is due, how failures back off and what each policy does with what was found. The app
//! (`app/src-tauri/src/ai/updates.rs`) runs the checks, the jobs and the notifications.
//!
//! - [`policy`]: Automatic / Ask / Manual, the schedule, the backoff and the decision for one update;
//! - [`candidates`]: the updates of the installed builds and models, from the pinned catalogs only;
//! - [`record`]: what is kept between runs (last check, failures, skipped and announced versions).
//!
//! Nothing outside the pinned catalogs is ever offered: a runtime update is a newer pinned release of an
//! installed build (`runtimes/llama-runtimes.json`, SHA-256 per asset), a model update is a catalog entry
//! that replaces an installed file (`supersedes`), confirmed against the model host before it is offered.

pub mod candidates;
pub mod policy;
pub mod record;

pub use candidates::{confirm_model, model_candidates, runtime_candidates, UpdateCandidate};
pub use policy::{backoff_secs, check_due, decide, Decision, Situation, UpdateKind, UpdatePolicy};
pub use record::{KindRecord, UpdatesRecord};
