//! Inference runtimes: the official prebuilt llama.cpp `llama-server` builds the worker runs (ADR 0009).
//! They are not bundled with IMT; the user downloads the ones they want, like models:
//!
//! - [`catalog`]: the pinned release and the assets per platform and build (`llama-runtimes.json`);
//! - [`github`]: fetching those assets from GitHub releases (a [`crate::models::ModelSource`]);
//! - [`unpack`]: safe extraction; [`macos`]: quarantine and signing after verification;
//! - [`store`]: `<data>/ai/runtimes/`, install (download, verify, unpack) and delete;
//! - [`builds`]: build ids, labels and the automatic order, shared with the worker;
//! - [`report`]: which installed builds work here (`imt-ai-worker --list-builds`);
//! - [`overview`]: what the runtime manager shows.

pub mod builds;
pub mod catalog;
pub mod github;
pub mod macos;
pub mod overview;
pub mod report;
pub mod store;
pub mod unpack;

pub use builds::BuildChoice;
pub use catalog::{current_platform, suggest, RuntimeAsset, RuntimeBuild, RuntimeCatalog};
pub use github::{GithubReleases, ReleaseConfig};
pub use overview::{overview, OverviewInput, RuntimeEntry, RuntimeOverview};
pub use report::{list_builds, BuildInfo, BuildsReport, Device};
pub use store::{InstalledRuntime, RuntimeStage, RuntimeStore};
