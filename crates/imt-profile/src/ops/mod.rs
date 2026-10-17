//! Analyses over a profile. All are pure functions of the profile, an
//! aggregated [`tree::CallTree`] and a request; none mutate anything.

pub mod butterfly;
pub mod common;
pub mod diff;
pub mod findings;
pub mod flame;
pub mod hotspots;
pub mod nav;
pub mod timeline;
pub mod tree;
