//! Getting model files onto the disk: the download port and its Hugging Face adapter, the
//! resumable verified downloader, free-space checks, the `models/` folder and checking files the user
//! downloaded themselves.

pub mod disk;
pub mod download;
pub mod hf;
pub mod source;
pub mod store;
pub mod verify;

pub use disk::{DiskSpace, SystemDisk};
pub use download::{DownloadOptions, Downloader};
pub use hf::{HfConfig, HuggingFace};
pub use source::{ModelSource, Phase, Progress, ProgressSnapshot, RemoteFile};
pub use store::{AddMode, InstalledModel, ModelStore};
