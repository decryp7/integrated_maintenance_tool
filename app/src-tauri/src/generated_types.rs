//! The TypeScript bindings of the shell's wire types (ADR 0016), generated with ts-rs into
//! `app/src/app/core/generated/` and re-exported by `api.types.ts`. The test fails when a file is stale;
//! `DEBRIEF_WRITE_TYPES=1 cargo test -p debrief-app --lib generated_types` rewrites them.
//!
//! Types of `debrief-core` are still mirrored by hand in `api.types.ts` (they will get the same derive when the
//! engine's owned reply types of ADR 0014 land).

use std::path::PathBuf;
use ts_rs::TS;

/// Where the files go.
fn dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../src/app/core/generated")
}

/// One generated file: its name and its text.
fn files() -> Vec<(&'static str, String)> {
    let header = "// Generated from the Rust wire types by `cargo test -p debrief-app --lib generated_types` (ts-rs; ADR 0016).\n// Do not edit: change the Rust type and run the test with DEBRIEF_WRITE_TYPES=1.\n\n";
    let decl = |d: String| format!("export {d}\n");
    vec![
        (
            "errors.ts",
            format!(
                "{header}{}\n{}\n{}",
                decl(crate::error::ErrorCode::decl()),
                decl(crate::error::ServiceCause::decl()),
                decl(crate::error::CmdError::decl())
            ),
        ),
        (
            "recent.ts",
            format!(
                "{header}{}\n{}\n{}\n{}",
                decl(crate::recent::RecentFile::decl()),
                decl(crate::recent::RecentProject::decl()),
                decl(crate::recent::RecentLists::decl()),
                decl(crate::recent::RecentKind::decl())
            ),
        ),
        ("dumps.ts", {
            use debrief_dump::api::*;
            let decls = [
                DocKind::decl(),
                DumpSniffKind::decl(),
                DumpSniffResult::decl(),
                Severity::decl(),
                DumpHeadline::decl(),
                DumpDocSummary::decl(),
                ExceptionCard::decl(),
                HandleTypeDto::decl(),
                HandlesDto::decl(),
                MemoryDto::decl(),
                SymbolStatusDto::decl(),
                SymbolProblems::decl(),
                DumpOverview::decl(),
                ThreadState::decl(),
                ThreadRow::decl(),
                ThreadsRequest::decl(),
                ThreadsPage::decl(),
                FrameTrust::decl(),
                FrameRow::decl(),
                StackRequest::decl(),
                StackDto::decl(),
                ModuleRow::decl(),
                ModulesRequest::decl(),
                ModulesPage::decl(),
                FindingKind::decl(),
                FindingEvidence::decl(),
                DumpFinding::decl(),
                DumpFindingsRequest::decl(),
                UmdhGroup::decl(),
                UmdhDiffRequest::decl(),
                UmdhRow::decl(),
                UmdhDiffPage::decl(),
                UmdhStackRequest::decl(),
                UmdhStackDto::decl(),
                UmdhTreeRequest::decl(),
                UmdhTreeNode::decl(),
                DumpMarker::decl(),
            ];
            format!("{header}{}", decls.into_iter().map(decl).collect::<Vec<_>>().join("\n"))
        }),
        (
            "paths.ts",
            format!(
                "{header}{}\n{}\n{}",
                decl(crate::paths::PickKind::decl()),
                decl(crate::paths::PickFilter::decl()),
                decl(crate::paths::PickRequest::decl())
            ),
        ),
        ("plugins.ts", crate::plugin_packages::dto::generated(header)),
        ("parsers.ts", crate::parsers::dto::generated(header)),
        ("store.ts", {
            use debrief_core::ports::*;
            let decls = [
                StoreActivity::decl(),
                LastImport::decl(),
                StoreOverview::decl(),
                SegmentsRequest::decl(),
                SegmentRow::decl(),
                SegmentsPage::decl(),
                FieldStorage::decl(),
                FieldType::decl(),
                StoreField::decl(),
                StoreFields::decl(),
                StoreIndex::decl(),
                RunningJob::decl(),
                WorkerMemory::decl(),
                StoreMemory::decl(),
                MaintenanceAction::decl(),
                MaintenanceReport::decl(),
                SqlColumn::decl(),
                SqlResult::decl(),
                SqlPageRequest::decl(),
                SqlPage::decl(),
                SqlIdsRequest::decl(),
                SqlIds::decl(),
                SqlViewColumn::decl(),
                SqlView::decl(),
                SqlSchema::decl(),
            ];
            format!("{header}{}", decls.into_iter().map(decl).collect::<Vec<_>>().join("\n"))
        }),
        ("video.ts", {
            use debrief_video::{align::*, api::*, record::*, runtime::*, select::*};
            let decls = [
                UserAlignment::decl(),
                ClockReading::decl(),
                AlignEvidence::decl(),
                Alignment::decl(),
                SelectMode::decl(),
                SelectOptions::decl(),
                SelectNote::decl(),
                FrameRecord::decl(),
                VideoRecord::decl(),
                InstallStage::decl(),
                RuntimeMode::decl(),
                InstalledRelease::decl(),
                RuntimeStatus::decl(),
                SelfTestReport::decl(),
                MediaState::decl(),
                VideoView::decl(),
                VideoList::decl(),
                SkippedVideo::decl(),
                DetectedFfmpeg::decl(),
                VideoRuntimeView::decl(),
                OpenVideoRequest::decl(),
                AlignRequest::decl(),
                FrameRef::decl(),
                NoteRequest::decl(),
                FramesRequest::decl(),
                ExplainRangeRequest::decl(),
                AddFrameRequest::decl(),
                IncludeRequest::decl(),
                FrameImage::decl(),
                FrameExplanation::decl(),
            ];
            format!("{header}{}", decls.into_iter().map(decl).collect::<Vec<_>>().join("\n"))
        }),
        ("notes.ts", {
            use debrief_notes::*;
            let decls = [
                Region::decl(),
                ImageRef::decl(),
                Anchor::decl(),
                NoteSeverity::decl(),
                Note::decl(),
                NoteDraft::decl(),
                NoteChange::decl(),
                NoteRecord::decl(),
                crate::notes::commands::NoteItem::decl(),
                crate::notes::commands::NotesList::decl(),
                crate::notes::commands::NotesRestore::decl(),
                crate::notes::images::NoteImage::decl(),
            ];
            format!("{header}{}", decls.into_iter().map(decl).collect::<Vec<_>>().join("\n"))
        }),
    ]
}

#[test]
fn generated_types_are_current() {
    let write = std::env::var_os("DEBRIEF_WRITE_TYPES").is_some();
    let mut stale = Vec::new();
    for (name, text) in files() {
        let path = dir().join(name);
        let on_disk = std::fs::read_to_string(&path).unwrap_or_default().replace("\r\n", "\n");
        if on_disk == text {
            continue;
        }
        if write {
            std::fs::create_dir_all(dir()).unwrap();
            std::fs::write(&path, &text).unwrap();
        } else {
            stale.push(name);
        }
    }
    assert!(
        stale.is_empty(),
        "generated TypeScript types are stale: {stale:?}; run `DEBRIEF_WRITE_TYPES=1 cargo test -p debrief-app --lib generated_types`"
    );
}
