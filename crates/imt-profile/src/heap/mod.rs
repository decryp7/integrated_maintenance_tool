//! Memory: heap snapshots, their comparison and (with an object graph)
//! dominators, retained sizes and paths to GC roots.

pub mod dominators;
pub mod findings;
pub mod model;
pub mod ops;

pub use dominators::HeapAnalysis;
pub use model::{HeapBuilder, HeapSnapshot, ObjectGraph, TypeStat};
