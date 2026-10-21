//! Images for the one local model (ADR 0025): the same model reads text and images when its multimodal
//! projector is loaded with it.
//!
//! - [`image`]: untrusted images decoded in Rust within limits, downscaled and re-encoded without metadata;
//! - [`exif`]: the capture time, the one fact kept from the metadata;
//! - [`facts`]: what the model reads off a screenshot, as strict, bounded data;
//! - [`assist`]: explaining an image and "find related logs" (the model reads, the engine searches,
//!   `imt_core::related`, the model explains the matches);
//! - [`intent`]: whether a chat question with an image asks for its related logs.

pub mod assist;
pub mod exif;
pub mod facts;
pub mod image;
pub mod intent;

pub use assist::{RelatedOutcome, RelatedStage};
pub use facts::{ScreenFacts, ScreenKind};
pub use image::{sanitize, ImageError, ImageLimits, SanitizedImage, SourceFormat};
