//! The storage seam between the UI shell and the engine that holds the records.
//! [`Workspace`] is the in-memory implementation; another backend (for example
//! DuckDB) can be swapped in by implementing [`LogStore`].

use crate::api::{
    ChartData, ChartRequest, ExtractPreview, ExtractPreviewRequest, Facet, FacetRequest, Page, PageRequest, Timeline, TimelineRequest,
    WorkspaceSummary,
};
use crate::profile::Profile;
use crate::workspace::Workspace;
use anyhow::Result;
use std::path::PathBuf;

/// Everything the UI shell needs from a log store.
pub trait LogStore: Send + Sync {
    /// Opens files and folders (recursively), skipping files already loaded.
    fn open(&mut self, paths: &[PathBuf]) -> Result<()>;

    /// Re-imports every opened root with the current profile.
    fn reload(&mut self) -> Result<()>;

    /// Forgets all opened files and records.
    fn clear(&mut self);

    /// Drops one source (by id) and its records; folder scans will not bring it back.
    fn remove_source(&mut self, id: u32);

    /// Replaces the profile and re-imports everything.
    fn set_profile(&mut self, profile: Profile) -> Result<()>;

    /// The active profile.
    fn profile(&self) -> &Profile;

    /// Sources, fields, totals and chart definitions of what is loaded.
    fn summary(&self) -> WorkspaceSummary;

    /// One page of the records matching a query.
    fn page(&self, req: &PageRequest) -> Result<Page<'_>>;

    /// Instance lanes and histogram for a query.
    fn timeline(&self, req: &TimelineRequest) -> Result<Timeline>;

    /// Value counts of one field over the records matching a query.
    fn facet(&self, req: &FacetRequest) -> Result<Facet>;

    /// Time-bucketed series for a chart definition.
    fn chart(&self, req: &ChartRequest) -> Result<ChartData>;

    /// Runs one extractor of an (unsaved) profile against the current records.
    fn preview_extractor(&self, req: &ExtractPreviewRequest) -> ExtractPreview;
}

impl LogStore for Workspace {
    fn open(&mut self, paths: &[PathBuf]) -> Result<()> {
        Workspace::open(self, paths)
    }

    fn reload(&mut self) -> Result<()> {
        Workspace::reload(self)
    }

    fn clear(&mut self) {
        Workspace::clear(self)
    }

    fn remove_source(&mut self, id: u32) {
        Workspace::remove_source(self, id)
    }

    fn set_profile(&mut self, profile: Profile) -> Result<()> {
        Workspace::set_profile(self, profile)
    }

    fn profile(&self) -> &Profile {
        Workspace::profile(self)
    }

    fn summary(&self) -> WorkspaceSummary {
        Workspace::summary(self)
    }

    fn page(&self, req: &PageRequest) -> Result<Page<'_>> {
        Workspace::page(self, req)
    }

    fn timeline(&self, req: &TimelineRequest) -> Result<Timeline> {
        Workspace::timeline(self, req)
    }

    fn facet(&self, req: &FacetRequest) -> Result<Facet> {
        Workspace::facet(self, req)
    }

    fn chart(&self, req: &ChartRequest) -> Result<ChartData> {
        Workspace::chart(self, req)
    }

    fn preview_extractor(&self, req: &ExtractPreviewRequest) -> ExtractPreview {
        Workspace::preview_extractor(self, req)
    }
}
