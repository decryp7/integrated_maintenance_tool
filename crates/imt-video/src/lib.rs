//! Video sources (ADR 0033): the downloadable FFmpeg runtime, sandboxed ffmpeg/ffprobe runs, frame selection,
//! alignment to the logs, the video records kept in the project, and the AI descriptions of frames.
//!
//! Pure parts (`select`, `parse`, `align`, argument builders) are unit-tested; process work goes through `run`.

pub mod align;
pub mod analyze;
pub mod api;
pub mod describe;
pub mod error;
pub mod parse;
pub mod record;
pub mod run;
pub mod runtime;
pub mod select;

pub use error::{Result, VideoError};
