//! The wire types of Settings ▸ Storage (mirrored in `app/src/app/settings/storage/storage.types.ts`).

use serde::Serialize;

/// One kind of thing IMT keeps on disk. The UI only ever names these; paths never come from the webview.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum CategoryId {
    /// `.imtz` bundles extracted to be opened (the system temp folder).
    BundleExtracts,
    /// The webview's HTTP, code and GPU caches.
    WebviewCache,
    /// Inference runtime builds of releases IMT no longer pins.
    AiRuntimesOutdated,
    /// Inference runtime builds of the pinned release.
    AiRuntimes,
    AiModels,
    /// IMT's own rolling logs (never today's file).
    Logs,
    ErrorSnapshots,
    /// Autosaved sessions older than this run.
    Sessions,
    /// Persistent event stores (ADR 0010); listed when the folder exists.
    Stores,
}

impl CategoryId {
    pub const ALL: [CategoryId; 9] = [
        CategoryId::BundleExtracts,
        CategoryId::WebviewCache,
        CategoryId::AiRuntimesOutdated,
        CategoryId::AiRuntimes,
        CategoryId::AiModels,
        CategoryId::Logs,
        CategoryId::ErrorSnapshots,
        CategoryId::Sessions,
        CategoryId::Stores,
    ];

    pub fn as_str(self) -> &'static str {
        match self {
            CategoryId::BundleExtracts => "bundle-extracts",
            CategoryId::WebviewCache => "webview-cache",
            CategoryId::AiRuntimesOutdated => "ai-runtimes-outdated",
            CategoryId::AiRuntimes => "ai-runtimes",
            CategoryId::AiModels => "ai-models",
            CategoryId::Logs => "logs",
            CategoryId::ErrorSnapshots => "error-snapshots",
            CategoryId::Sessions => "sessions",
            CategoryId::Stores => "stores",
        }
    }

    /// Parses an id from the UI.
    pub fn parse(s: &str) -> Option<CategoryId> {
        CategoryId::ALL.into_iter().find(|c| c.as_str() == s)
    }

    pub fn kind(self) -> Kind {
        match self {
            CategoryId::BundleExtracts | CategoryId::WebviewCache | CategoryId::AiRuntimesOutdated => Kind::Cache,
            _ => Kind::Data,
        }
    }

    /// English label (the UI shows its own translation by id and falls back to this).
    pub fn label(self) -> &'static str {
        match self {
            CategoryId::BundleExtracts => "Opened bundles (temporary copies)",
            CategoryId::WebviewCache => "Web view cache",
            CategoryId::AiRuntimesOutdated => "Outdated AI runtimes",
            CategoryId::AiRuntimes => "AI runtimes",
            CategoryId::AiModels => "AI models",
            CategoryId::Logs => "IMT logs",
            CategoryId::ErrorSnapshots => "Error snapshots",
            CategoryId::Sessions => "Session backups",
            CategoryId::Stores => "Persistent stores",
        }
    }
}

/// Whether IMT can rebuild it (`cache`: cleared together by "Clear cache") or it is the user's (`data`:
/// each needs its own confirmation).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Cache,
    Data,
}

/// One row of the Storage table.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageCategory {
    pub id: CategoryId,
    pub label: String,
    pub kind: Kind,
    /// The folder, for display and "Show in folder"; `None` when IMT cannot resolve it.
    pub path: Option<String>,
    /// Whether the folder exists (nothing to show in the file manager otherwise).
    pub exists: bool,
    pub bytes: u64,
    pub files: u64,
    /// What clearing would remove now (protected items excluded).
    pub clearable_bytes: u64,
    /// The traversal stopped at its entry or time limit: the sizes are lower bounds ("≥").
    pub capped: bool,
    pub clearable: bool,
    /// Why it cannot be cleared, or what clearing keeps.
    pub note: Option<String>,
}

/// Reply of `storage_usage`.
#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageUsage {
    pub categories: Vec<StorageCategory>,
    pub total_bytes: u64,
    /// Any category's sizes are lower bounds.
    pub capped: bool,
}

/// Something clearing left in place, and why.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Skipped {
    pub path: String,
    pub reason: String,
}

/// What clearing one category did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CategoryClearResult {
    pub id: CategoryId,
    pub freed_bytes: u64,
    pub removed_files: u64,
    /// The first skipped items (at most [`MAX_SKIPPED_LISTED`]); `skipped_count` has them all.
    pub skipped: Vec<Skipped>,
    pub skipped_count: u64,
    /// The category could not be cleared at all.
    pub error: Option<String>,
}

/// How many skipped items one result lists.
pub const MAX_SKIPPED_LISTED: usize = 20;

impl CategoryClearResult {
    pub fn new(id: CategoryId) -> Self {
        CategoryClearResult {
            id,
            freed_bytes: 0,
            removed_files: 0,
            skipped: Vec::new(),
            skipped_count: 0,
            error: None,
        }
    }

    pub fn failed(id: CategoryId, error: impl Into<String>) -> Self {
        CategoryClearResult {
            error: Some(error.into()),
            ..CategoryClearResult::new(id)
        }
    }

    pub fn skip(&mut self, path: &std::path::Path, reason: impl Into<String>) {
        self.skipped_count += 1;
        if self.skipped.len() < MAX_SKIPPED_LISTED {
            self.skipped.push(Skipped {
                path: path.to_string_lossy().into_owned(),
                reason: reason.into(),
            });
        }
    }
}

/// Reply of `storage_clear`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageClearResult {
    pub freed_bytes: u64,
    pub categories: Vec<CategoryClearResult>,
    /// Stopped (background task stop) before everything was cleared.
    pub cancelled: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_round_trip_and_unknown_ones_are_refused() {
        for id in CategoryId::ALL {
            assert_eq!(CategoryId::parse(id.as_str()), Some(id));
            assert_eq!(serde_json::to_value(id).unwrap(), id.as_str());
            assert!(!id.label().is_empty());
        }
        for bad in ["", "LOGS", "../logs", "logs ", "cache"] {
            assert_eq!(CategoryId::parse(bad), None, "{bad}");
        }
        let cache: Vec<_> = CategoryId::ALL.into_iter().filter(|c| c.kind() == Kind::Cache).collect();
        assert_eq!(
            cache,
            [CategoryId::BundleExtracts, CategoryId::WebviewCache, CategoryId::AiRuntimesOutdated]
        );
        assert_eq!(serde_json::to_value(Kind::Data).unwrap(), "data");
    }

    #[test]
    fn skipped_items_are_counted_but_listed_only_up_to_the_cap() {
        let mut r = CategoryClearResult::new(CategoryId::Logs);
        for i in 0..(MAX_SKIPPED_LISTED + 5) {
            r.skip(std::path::Path::new(&format!("f{i}")), "in use");
        }
        assert_eq!(r.skipped.len(), MAX_SKIPPED_LISTED);
        assert_eq!(r.skipped_count, MAX_SKIPPED_LISTED as u64 + 5);
        let failed = CategoryClearResult::failed(CategoryId::Stores, "no");
        assert_eq!(failed.error.as_deref(), Some("no"));
        let json = serde_json::to_value(&failed).unwrap();
        assert_eq!(json["skippedCount"], 0);
        assert_eq!(json["freedBytes"], 0);
    }
}
