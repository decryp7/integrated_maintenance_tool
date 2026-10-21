//! The open documents of one window and the queries over them. Importing runs outside the store (it can take
//! long): pick the importer with [`DumpDocs::pick`], import without holding the store, then [`DumpDocs::insert`].

use crate::api::*;
use crate::error::{DumpError, Result};
use crate::import::{DumpImporter, Produces, SNIFF_THRESHOLD};
use crate::limits::Limits;
use crate::model::{Document, Dump, UmdhKind, UmdhLog};
use crate::umdh::{self, Growth};
use std::sync::{Arc, Mutex};

/// Most documents open per window.
pub const MAX_DOCS: usize = 32;
/// Most rows one page returns.
pub const MAX_PAGE: u32 = 10_000;

struct Doc {
    id: u32,
    name: String,
    path: Option<String>,
    body: Document,
}

type GrowthKey = (Option<u32>, u32);

/// One window's dump documents.
pub struct DumpDocs {
    importers: Vec<Arc<dyn DumpImporter>>,
    limits: Limits,
    docs: Vec<Doc>,
    next_id: u32,
    /// The last growth computed (paging and tree expansion ask for the same pair again and again).
    growth: Mutex<Option<(GrowthKey, Arc<Vec<Growth>>)>>,
}

impl DumpDocs {
    pub fn new(importers: Vec<Arc<dyn DumpImporter>>, limits: Limits) -> DumpDocs {
        DumpDocs {
            importers,
            limits,
            docs: Vec::new(),
            next_id: 1,
            growth: Mutex::new(None),
        }
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Whether any importer is available (the dumps plugin is installed and not quarantined).
    pub fn has_importers(&self) -> bool {
        !self.importers.is_empty()
    }

    /// The best importer for a file, if one recognises it.
    pub fn pick(&self, file_name: &str, head: &[u8]) -> Option<Arc<dyn DumpImporter>> {
        self.importers
            .iter()
            .map(|i| (i.sniff(file_name, head), i))
            .filter(|(score, _)| *score >= SNIFF_THRESHOLD)
            .max_by_key(|(score, _)| *score)
            .map(|(_, i)| i.clone())
    }

    /// What a file is.
    pub fn sniff(&self, file_name: &str, head: &[u8]) -> DumpSniffKind {
        match self.pick(file_name, head).map(|i| i.produces()) {
            Some(Produces::Dump) => DumpSniffKind::Dump,
            Some(Produces::Umdh) => DumpSniffKind::Umdh,
            None => DumpSniffKind::None,
        }
    }

    /// Adds an imported document; one already open from the same path is replaced (reopening refreshes it).
    pub fn insert(&mut self, name: &str, path: Option<&str>, body: Document) -> Result<DumpDocSummary> {
        crate::wire::check(&body, &self.limits)?;
        if let Some(p) = path {
            if let Some(i) = self.docs.iter().position(|d| d.path.as_deref() == Some(p)) {
                self.docs.remove(i);
            }
        }
        if self.docs.len() >= MAX_DOCS {
            return Err(DumpError::limit(format!(
                "at most {MAX_DOCS} dumps and UMDH logs can be open; close one first"
            )));
        }
        let id = self.next_id;
        self.next_id += 1;
        self.docs.push(Doc {
            id,
            name: name.into(),
            path: path.map(str::to_string),
            body,
        });
        self.forget_growth();
        let doc = self.docs.last().ok_or_else(|| DumpError::invalid("no document"))?;
        Ok(summary(doc))
    }

    pub fn list(&self) -> Vec<DumpDocSummary> {
        self.docs.iter().map(summary).collect()
    }

    /// Closes a document; `false` when it was not open.
    pub fn close(&mut self, id: u32) -> bool {
        let before = self.docs.len();
        self.docs.retain(|d| d.id != id);
        self.forget_growth();
        before != self.docs.len()
    }

    fn forget_growth(&self) {
        *self.growth.lock().unwrap_or_else(|p| p.into_inner()) = None;
    }

    fn doc(&self, id: u32) -> Result<&Doc> {
        self.docs
            .iter()
            .find(|d| d.id == id)
            .ok_or_else(|| DumpError::not_found(format!("dump document {id} is not open (any more)")))
    }

    fn dump(&self, id: u32) -> Result<&Dump> {
        match &self.doc(id)?.body {
            Document::Dump(d) => Ok(d),
            Document::Umdh(_) => Err(DumpError::unsupported("this is a UMDH log, not a dump")),
        }
    }

    fn umdh(&self, id: u32) -> Result<&UmdhLog> {
        match &self.doc(id)?.body {
            Document::Umdh(u) => Ok(u),
            Document::Dump(_) => Err(DumpError::unsupported("this is a dump, not a UMDH log")),
        }
    }

    pub fn overview(&self, id: u32) -> Result<DumpOverview> {
        let doc = self.doc(id)?;
        let d = self.dump(id)?;
        Ok(crate::views::overview(d, summary(doc)))
    }

    pub fn threads(&self, req: &ThreadsRequest) -> Result<ThreadsPage> {
        Ok(crate::views::threads(self.dump(req.id)?, req, MAX_PAGE))
    }

    pub fn stack(&self, req: &StackRequest) -> Result<StackDto> {
        let d = self.dump(req.id)?;
        let t = d
            .threads
            .iter()
            .find(|t| t.id == req.thread_id)
            .ok_or_else(|| DumpError::not_found(format!("the dump has no thread {}", req.thread_id)))?;
        Ok(crate::views::stack(d, t))
    }

    pub fn modules(&self, req: &ModulesRequest) -> Result<ModulesPage> {
        Ok(crate::views::modules(self.dump(req.id)?, req, MAX_PAGE))
    }

    /// The UMDH logs of `ids`, oldest first (by their time, then as given).
    fn series(&self, ids: &[u32]) -> Result<Vec<(u32, &UmdhLog)>> {
        let mut logs = ids.iter().map(|id| Ok((*id, self.umdh(*id)?))).collect::<Result<Vec<_>>>()?;
        logs.sort_by_key(|(_, l)| l.time_ms.unwrap_or(i64::MIN));
        Ok(logs)
    }

    /// Findings of one dump, or of UMDH logs of one process.
    pub fn findings(&self, req: &DumpFindingsRequest) -> Result<Vec<DumpFinding>> {
        match req.ids.as_slice() {
            [] => Ok(Vec::new()),
            [id] if matches!(self.doc(*id)?.body, Document::Dump(_)) => Ok(crate::findings::findings(*id, self.dump(*id)?)),
            ids => umdh::findings(&self.series(ids)?),
        }
    }

    fn growth(&self, base: Option<u32>, target: u32) -> Result<Arc<Vec<Growth>>> {
        let key = (base, target);
        if let Some((k, rows)) = &*self.growth.lock().unwrap_or_else(|p| p.into_inner()) {
            if *k == key {
                return Ok(rows.clone());
            }
        }
        let b = base.map(|b| self.umdh(b)).transpose()?;
        let rows = Arc::new(umdh::growth(b, self.umdh(target)?)?);
        *self.growth.lock().unwrap_or_else(|p| p.into_inner()) = Some((key, rows.clone()));
        Ok(rows)
    }

    pub fn umdh_diff(&self, req: &UmdhDiffRequest) -> Result<UmdhDiffPage> {
        Ok(umdh::diff_page(&self.growth(req.base, req.target)?, req, MAX_PAGE))
    }

    pub fn umdh_stack(&self, req: &UmdhStackRequest) -> Result<UmdhStackDto> {
        umdh::stack(&self.growth(req.base, req.target)?, &req.key)
    }

    pub fn umdh_tree(&self, req: &UmdhTreeRequest) -> Result<Vec<UmdhTreeNode>> {
        if req.path.len() > self.limits.max_umdh_frames {
            return Err(DumpError::invalid("the tree path is deeper than any stack"));
        }
        Ok(umdh::tree(&self.growth(req.base, req.target)?, &req.path, req.limit.min(MAX_PAGE)))
    }

    /// Documents with a time, for the log timeline.
    pub fn markers(&self) -> Vec<DumpMarker> {
        self.docs
            .iter()
            .filter_map(|d| {
                let s = summary(d);
                Some(DumpMarker {
                    id: s.id,
                    name: s.name,
                    kind: s.kind,
                    time_ms: s.time_ms?,
                    headline: s.headline,
                    severity: s.severity,
                })
            })
            .collect()
    }

    /// The compact, redacted question "Explain with AI" sends about `ids`.
    pub fn explain(&self, req: &DumpFindingsRequest) -> Result<String> {
        let findings = self.findings(req)?;
        match req.ids.as_slice() {
            [] => Err(DumpError::invalid("nothing to explain")),
            [id] if matches!(self.doc(*id)?.body, Document::Dump(_)) => {
                let doc = self.doc(*id)?;
                Ok(crate::explain::dump_prompt(&doc.name, self.dump(*id)?, &findings))
            }
            ids => {
                let logs = self.series(ids)?;
                let names: Vec<&str> = logs
                    .iter()
                    .map(|(id, _)| self.doc(*id).map(|d| d.name.as_str()))
                    .collect::<Result<_>>()?;
                let bodies: Vec<&UmdhLog> = logs.iter().map(|(_, l)| *l).collect();
                Ok(crate::explain::umdh_prompt(&names, &bodies, &findings))
            }
        }
    }
}

fn summary(doc: &Doc) -> DumpDocSummary {
    let (kind, time_ms, headline, process, severity) = match &doc.body {
        Document::Dump(d) => (
            DocKind::Dump,
            d.time_ms,
            crate::views::headline(d),
            d.process.as_ref().map(|p| p.name.clone()),
            crate::findings::findings(doc.id, d)
                .iter()
                .map(|f| f.severity)
                .max()
                .unwrap_or(Severity::Info),
        ),
        Document::Umdh(u) => {
            let bytes: u64 = u.stacks.iter().map(|s| s.bytes).sum();
            let (kind, headline) = match u.kind {
                UmdhKind::Snapshot => (
                    DocKind::UmdhSnapshot,
                    format!("{} allocation stacks, {}", u.stacks.len(), umdh::fmt_bytes(bytes as i64)),
                ),
                UmdhKind::Diff => {
                    let old: u64 = u.stacks.iter().filter_map(|s| s.old_bytes).sum();
                    (
                        DocKind::UmdhDiff,
                        format!(
                            "{} across {} allocation stacks",
                            umdh::fmt_bytes(bytes as i64 - old as i64),
                            u.stacks.len()
                        ),
                    )
                }
            };
            let severity = umdh::findings(&[(doc.id, u)])
                .map(|f| f.iter().map(|f| f.severity).max().unwrap_or(Severity::Info))
                .unwrap_or(Severity::Info);
            (kind, u.time_ms, headline, u.process.as_ref().and_then(|p| p.name.clone()), severity)
        }
    };
    DumpDocSummary {
        id: doc.id,
        name: doc.name.clone(),
        path: doc.path.clone(),
        kind,
        time_ms,
        headline,
        process,
        severity,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::ErrorCode;
    use crate::import::{ImportOptions, ReadAt};
    use crate::testing::{sample_dump, sample_umdh};

    struct Fake(Produces, &'static [u8]);

    impl DumpImporter for Fake {
        fn id(&self) -> &str {
            "fake"
        }
        fn produces(&self) -> Produces {
            self.0
        }
        fn sniff(&self, _: &str, head: &[u8]) -> u8 {
            if head.starts_with(self.1) {
                90
            } else {
                10
            }
        }
        fn import(&self, _: &str, _: Arc<dyn ReadAt>, _: &ImportOptions) -> Result<Document> {
            Ok(Document::Dump(sample_dump()))
        }
    }

    fn docs() -> DumpDocs {
        DumpDocs::new(
            vec![Arc::new(Fake(Produces::Dump, b"MDMP")), Arc::new(Fake(Produces::Umdh, b"// UMDH"))],
            Limits::default(),
        )
    }

    #[test]
    fn importers_are_picked_by_sniffing() {
        let d = docs();
        assert!(d.has_importers());
        assert_eq!(d.sniff("a.dmp", b"MDMP...."), DumpSniffKind::Dump);
        assert_eq!(d.sniff("a.txt", b"// UMDH"), DumpSniffKind::Umdh);
        assert_eq!(d.sniff("a.txt", b"hello"), DumpSniffKind::None);
        assert_eq!(d.pick("a.dmp", b"MDMP").unwrap().id(), "fake");
        let none = DumpDocs::new(Vec::new(), Limits::default());
        assert!(!none.has_importers() && none.pick("a", b"MDMP").is_none());
        let opts = ImportOptions {
            limits: Limits::default(),
            file_time_ms: None,
        };
        assert!(d.pick("a.dmp", b"MDMP").unwrap().import("a", Arc::new(Vec::new()), &opts).is_ok());
    }

    #[test]
    fn documents_open_query_and_close() {
        let mut d = docs();
        let a = d.insert("crash.dmp", Some("/x/crash.dmp"), Document::Dump(sample_dump())).unwrap();
        assert_eq!((a.kind, a.severity), (DocKind::Dump, Severity::Critical));
        assert_eq!(a.headline, "ACCESS_VIOLATION in ntdll.dll!RtlFreeHeap+0x1d");
        // Reopening the same path replaces it.
        let again = d.insert("crash.dmp", Some("/x/crash.dmp"), Document::Dump(sample_dump())).unwrap();
        assert_ne!(again.id, a.id);
        assert_eq!(d.list().len(), 1);
        let o = d.overview(again.id).unwrap();
        assert_eq!(o.threads, 4);
        let page = d
            .threads(&ThreadsRequest {
                id: again.id,
                offset: 0,
                limit: 2,
                filter: None,
            })
            .unwrap();
        assert_eq!((page.rows.len(), page.total), (2, 4));
        let s = d
            .stack(&StackRequest {
                id: again.id,
                thread_id: 4121,
            })
            .unwrap();
        assert_eq!(s.frames.len(), 4);
        assert_eq!(
            d.stack(&StackRequest {
                id: again.id,
                thread_id: 1
            })
            .unwrap_err()
            .code,
            ErrorCode::NotFound
        );
        let m = d
            .modules(&ModulesRequest {
                id: again.id,
                offset: 0,
                limit: 100,
                filter: Some("shop".into()),
            })
            .unwrap();
        assert_eq!(m.total, 2);
        let f = d.findings(&DumpFindingsRequest { ids: vec![again.id] }).unwrap();
        assert_eq!(f[0].kind, FindingKind::CrashCause);
        assert!(d.findings(&DumpFindingsRequest { ids: vec![] }).unwrap().is_empty());
        assert!(d
            .explain(&DumpFindingsRequest { ids: vec![again.id] })
            .unwrap()
            .contains("crash.dmp"));
        assert!(d.explain(&DumpFindingsRequest { ids: vec![] }).is_err());
        assert_eq!(d.markers().len(), 1);
        assert_eq!(d.umdh_diff(&diff(None, again.id)).unwrap_err().code, ErrorCode::Unsupported);
        assert!(d.close(again.id));
        assert!(!d.close(again.id));
        assert_eq!(d.overview(again.id).unwrap_err().code, ErrorCode::NotFound);
    }

    fn diff(base: Option<u32>, target: u32) -> UmdhDiffRequest {
        UmdhDiffRequest {
            base,
            target,
            group: UmdhGroup::Stack,
            offset: 0,
            limit: 50,
            filter: None,
        }
    }

    #[test]
    fn umdh_logs_compare_and_explain() {
        let mut d = docs();
        let mib = 1u64 << 20;
        // Opened newest first: the series is ordered by time.
        let b = d
            .insert(
                "b.txt",
                None,
                Document::Umdh(sample_umdh(UmdhKind::Snapshot, 10, &[(1, 30 * mib, 30)])),
            )
            .unwrap();
        let a = d
            .insert(
                "a.txt",
                None,
                Document::Umdh(sample_umdh(UmdhKind::Snapshot, 0, &[(1, 10 * mib, 10)])),
            )
            .unwrap();
        assert_eq!(a.kind, DocKind::UmdhSnapshot);
        assert_eq!(a.headline, "1 allocation stacks, 10.0 MB");
        let page = d.umdh_diff(&diff(Some(a.id), b.id)).unwrap();
        assert_eq!(page.delta_bytes, 20 * mib as i64);
        // Cached: the same answer again.
        assert_eq!(d.umdh_diff(&diff(Some(a.id), b.id)).unwrap(), page);
        let key = page.rows[0].key.clone();
        let s = d
            .umdh_stack(&UmdhStackRequest {
                base: Some(a.id),
                target: b.id,
                key,
            })
            .unwrap();
        assert_eq!(s.ids, [1]);
        let roots = d
            .umdh_tree(&UmdhTreeRequest {
                base: Some(a.id),
                target: b.id,
                path: vec![],
                limit: 10,
            })
            .unwrap();
        assert_eq!(roots.len(), 1);
        assert!(d
            .umdh_tree(&UmdhTreeRequest {
                base: None,
                target: b.id,
                path: vec![String::new(); 1000],
                limit: 10
            })
            .is_err());
        let f = d.findings(&DumpFindingsRequest { ids: vec![b.id, a.id] }).unwrap();
        assert!(f.iter().any(|f| f.kind == FindingKind::LeakGrowth && f.evidence.doc_id == b.id));
        let text = d.explain(&DumpFindingsRequest { ids: vec![b.id, a.id] }).unwrap();
        assert!(text.contains("Logs: a.txt, b.txt."), "{text}");
        let diff_log = d
            .insert("d.txt", None, Document::Umdh(sample_umdh(UmdhKind::Diff, 0, &[(1, 4 * mib, 4)])))
            .unwrap();
        assert_eq!(diff_log.kind, DocKind::UmdhDiff);
        assert_eq!(diff_log.headline, "2.0 MB across 1 allocation stacks");
        assert_eq!(d.overview(a.id).unwrap_err().code, ErrorCode::Unsupported);
        assert_eq!(d.markers().len(), 3);
    }

    #[test]
    fn the_store_is_bounded_and_checks_documents() {
        let mut d = docs();
        for i in 0..MAX_DOCS {
            d.insert(&format!("{i}"), None, Document::Dump(sample_dump())).unwrap();
        }
        assert_eq!(
            d.insert("one more", None, Document::Dump(sample_dump())).unwrap_err().code,
            ErrorCode::Limit
        );
        let mut bad = sample_dump();
        bad.threads[0].frames[0].module = Some(99);
        let mut fresh = docs();
        assert_eq!(fresh.insert("bad", None, Document::Dump(bad)).unwrap_err().code, ErrorCode::Invalid);
        assert_eq!(fresh.limits().max_frames, Limits::default().max_frames);
    }
}
