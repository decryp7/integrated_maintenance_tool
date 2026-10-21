//! IMT profiling engine: the common profile model and every analysis over it.
//!
//! Importers (plugins) turn capture files into a [`Profile`] (CPU samples,
//! allocations, instrumented calls, timeline events) or a
//! [`heap::HeapSnapshot`] through [`ProfileBuilder`] / [`heap::HeapBuilder`].
//! Everything else is pure and bounded: call trees, hotspots, flame graph
//! layout, comparisons, thread timelines, findings, heap diffs and dominators.

pub mod api;
pub mod builder;
pub mod error;
pub mod frames;
pub mod heap;
pub mod import;
pub mod limits;
pub mod model;
pub mod ops;
pub mod store;
pub mod wire;

pub use builder::{FrameSpec, ProfileBuilder};
pub use error::{ProfileError, Result};
pub use import::{Capture, CaptureImporter, IMPORTERS_POINT};
pub use limits::Limits;
pub use model::*;
pub use store::{fuzz_views, CaptureCatalog, Captures, HeapQueries, ProfileQueries, ProfileStore};
