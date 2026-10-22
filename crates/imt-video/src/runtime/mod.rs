//! The FFmpeg runtime (ADR 0033 §1): pinned LGPL builds downloaded on demand like the inference runtime (ADR 0009),
//! never linked or bundled.
//!
//! - [`catalog`]: the pinned release and its archive per platform (`ffmpeg-runtimes.json`);
//! - [`unpack`]: safe extraction of `ffmpeg`, `ffprobe` and their libraries;
//! - [`testclip`] / [`selftest`]: the generated clip every runtime must analyse correctly before it is used;
//! - [`store`]: `<data>/video/ffmpeg/`, install, update with rollback, delete and the user's own ffmpeg.

pub mod catalog;
pub mod detect;
pub mod selftest;
pub mod store;
pub mod testclip;
pub mod unpack;

pub use catalog::{current_platform, FfmpegAsset, FfmpegCatalog};
pub use detect::detect;
pub use selftest::{self_test, SelfTestReport};
pub use store::{custom_tools, FfmpegStore, InstallStage, InstalledRelease, RuntimeMode, RuntimeStatus};
