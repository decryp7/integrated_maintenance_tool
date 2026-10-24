//! Service health (ADR 0055, Paddock ADR 0010): which of Debrief's services run, and why one does not.
//!
//! - [`model`] and [`registry`]: the general part (states, cause, repair action, snapshot, one announcement per real
//!   change). No Debrief types and no Tauri; they move into the shared `paddock-health` crate unchanged.
//! - [`hub`]: Debrief's five services (the store, the AI worker, plugins, the research browser, downloads), what reports
//!   into each and what repairs it.
//! - [`commands`]: `health_snapshot`, `health_repair`, the `debrief://service-health` event and the status probes.
//!
//! The supervisors of the services report here; nothing is derived from error text. Repair is served by the shell and
//! never goes through the service it repairs (Paddock rule 19).

pub mod commands;
pub mod hub;
pub mod model;
pub mod registry;

pub use hub::current;
