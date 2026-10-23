//! Human-like name matching over the entity catalogue: the logs write one program as `CompanyA.Graphic.Builder.exe`,
//! `graphicbuilder.exe` and `graphic-builder`; the user asks for "graphic builder", "grafic bilder" or "image editor".
//!
//! - [`segment`], [`normalise`], [`group`]: deterministic word segmentation, name normalisation and grouping of
//!   spellings into one entity with variants (layer 1).
//! - [`alias_job`]: optional AI aliases from a local model, cached in the [`book::NameBook`] (layer 2).
//! - [`context`]: ranking by what the user is looking at, and name co-occurrence.
//! - [`book`]: the entity table, AI aliases and the user's own corrections (which outrank the AI's).
//! - [`resolve`]: matching with the way and the reason for every match, and clarifying questions on true ties.
//!
//! Everything is pure and bounded; the model sits behind [`alias_job::AliasModel`]. See `docs/adr/0050-human-name-matching.md`.

pub mod alias_job;
pub mod book;
pub mod build;
pub mod context;
pub mod group;
pub mod normalise;
pub mod resolve;
pub mod segment;

pub use book::NameBook;
pub use build::Names;
pub use context::{Cooccurrence, SearchContext};
pub use resolve::{How, NameSettings, Resolution, Resolver};
pub use segment::Vocabulary;
