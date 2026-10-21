//! File-level live tail events: renames, truncation, copies to adopt,
//! re-imports, new files and "start at end".

use super::identity::{hash, Fingerprint, ANCHOR_BYTES, HEAD_BYTES};
use super::reader::{count_lines, ByteSource, Encoding, FileBytes};
use super::{
    display_name, group_of, identity_of, import_file, mtime, source_file, Delta, Mode, TailFileState, TailReport, TailSeed, TailState,
};
use super::{RotatedSource, Workspace, HEAD_TEXT_BYTES};
use crate::importers::ImportCtx;
use std::path::{Path, PathBuf};
use std::time::Instant;

/// Whether `src` starts with the content `seed` describes up to its offset
/// (a copy of a tailed file, or the file itself unchanged).
fn same_content(seed: &TailSeed, src: &dyn ByteSource) -> bool {
    if src.len() < seed.offset {
        return false;
    }
    let Ok(head) = src.read(0, HEAD_BYTES as u64) else { return false };
    let from = seed.offset.saturating_sub(ANCHOR_BYTES);
    seed.head.matches(&head) && src.read(from, seed.offset).is_ok_and(|b| hash(&b) == seed.anchor)
}

impl Workspace {
    /// Points a source at its file's new name (rotation).
    pub(super) fn rename_source(&mut self, id: u32, to: &Path) {
        let name = display_name(to, &self.roots);
        if let Some(s) = self.sources.iter_mut().find(|s| s.id == id) {
            s.path = to.to_string_lossy().into_owned();
            s.group = group_of(&name);
            s.name = name.clone();
            // The rotated file keeps its clock offset (a new file at the old name too).
            if s.offset_ms != 0 {
                let _ = self.offsets.set(&s.path, s.offset_ms);
            }
        }
        if let Some(n) = self.names.get_mut(id as usize) {
            *n = name;
        }
        self.sources.sort_by(|a, b| a.name.cmp(&b.name));
    }

    /// Removes a source and its records (deleted file with `deletedFiles: drop`).
    pub(super) fn drop_source(&mut self, id: u32, delta: &mut Delta) {
        self.sources.retain(|s| s.id != id);
        self.tail.forget(id);
        delta.remove_from(id, None);
    }

    /// The file was truncated or rewritten in place: its records stay with a
    /// detached source; a copy of the old content (copy-truncate) is adopted,
    /// and the file itself is read again from its start as a new source.
    pub(super) fn truncated(&mut self, id: u32, st: &mut TailState, delta: &mut Delta, report: &mut TailReport, now: Instant) {
        st.attached = false;
        st.state = TailFileState::Truncated;
        st.carry_since = None;
        report.truncated.push(id);
        self.tail.restart.push(st.path.clone());
        for copy in std::mem::take(&mut st.copies) {
            let Ok(len) = std::fs::metadata(&copy).map(|m| m.len()) else {
                continue;
            };
            if !same_content(&st.seed, &FileBytes { path: &copy, len }) {
                continue;
            }
            let original = st.path.to_string_lossy().into_owned();
            self.adopt(id, st, &copy);
            report.rotated.push(RotatedSource {
                source_id: id,
                from: original,
                to: copy.to_string_lossy().into_owned(),
            });
            self.follow(id, st, delta, report, now);
            break;
        }
    }

    fn adopt(&mut self, id: u32, st: &mut TailState, to: &Path) {
        st.path = to.to_path_buf();
        st.seed.identity = identity_of(to);
        st.attached = true;
        st.state = TailFileState::Rotated;
        self.rename_source(id, to);
    }

    /// Reads a file whole again, keeping its source id.
    pub(super) fn reimport(&mut self, id: u32, st: &mut TailState, reason: &str, delta: &mut Delta, report: &mut TailReport) {
        let name = self.source_name(id).to_string();
        let path_s = st.path.to_string_lossy().into_owned();
        let results = import_file(&self.registry.importers, &self.profile, &st.path, &name, &self.removed);
        match results.into_iter().find(|(p, ..)| *p == path_s).map(|(_, _, r)| r) {
            Some(Ok(loaded)) => {
                delta.remove_from(id, None);
                let offset = self.source_offset(id);
                // A tailed file had no routed events when opened; events routes would claim now stay in its own
                // source (nothing is lost) until the file is opened again.
                let routed = loaded.logical.into_iter().flat_map(|(_, records)| records);
                delta.insert.extend(loaded.imported.records.into_iter().chain(routed).map(|mut r| {
                    r.source = id;
                    r.ts = crate::skew::shift(r.ts, offset);
                    r
                }));
                if let Some(s) = self.sources.iter_mut().find(|s| s.id == id) {
                    s.importer = loaded.importer;
                    s.skipped = loaded.imported.skipped;
                    s.dropped = loaded.stats.first().map_or(0, |f| f.input) - loaded.stats.last().map_or(0, |l| l.output);
                    s.step_stats = loaded.stats;
                    s.error = None;
                }
                if let Some(seed) = loaded.seed {
                    st.state = match seed.mode {
                        Mode::Whole => TailFileState::Whole,
                        Mode::Append { .. } => TailFileState::Tailing,
                    };
                    st.seed = seed;
                }
                st.carry_since = None;
                st.last_event = Some(super::wall_ms());
                report.reimported.push(id);
            }
            Some(Err(e)) => {
                st.state = TailFileState::Error {
                    message: format!("{reason}; reading it again failed: {e}"),
                }
            }
            None => {
                st.state = TailFileState::Error {
                    message: format!("{reason}; the file is no longer readable"),
                }
            }
        }
    }

    /// A changed path not tailed yet: `Some` when it should be imported as a
    /// new source. Copies of tailed content are adopted or remembered instead,
    /// archives with untracked sources are re-read, unchanged unrecognised
    /// files are skipped.
    pub(super) fn new_candidate(&mut self, p: &Path, delta: &mut Delta, report: &mut TailReport) -> Option<PathBuf> {
        if !self.wanted(p) || self.tail.states.values().any(|s| s.attached && s.path == p) {
            return None;
        }
        let ps = p.to_string_lossy().into_owned();
        // Sources without tail state (zip members): read the archive again.
        let untracked: Vec<u32> = self
            .sources
            .iter()
            .filter(|s| source_file(&s.path) == ps && !self.tail.states.contains_key(&s.id))
            .map(|s| s.id)
            .collect();
        if !untracked.is_empty() {
            for id in untracked {
                self.drop_source(id, delta);
                report.reimported.push(id);
            }
            self.unrecognized.retain(|u| source_file(&u.path) != ps);
            return Some(p.to_path_buf());
        }
        if self.unrecognized.iter().any(|u| source_file(&u.path) == ps) {
            let seen = (std::fs::metadata(p).map_or(0, |m| m.len()), mtime(p));
            if self.tail.unrecognized_seen.get(&ps) == Some(&seen) {
                return None;
            }
            self.unrecognized.retain(|u| source_file(&u.path) != ps);
        }
        if self.adopt_copy(p, delta, report) {
            return None;
        }
        Some(p.to_path_buf())
    }

    /// Whether `p` holds content already tailed: a copy of a tailed file
    /// (remembered until the original is truncated) or the renamed/compressed
    /// content of a truncated or deleted one (adopted, reading only what is new).
    fn adopt_copy(&mut self, p: &Path, delta: &mut Delta, report: &mut TailReport) -> bool {
        let Ok(len) = std::fs::metadata(p).map(|m| m.len()) else {
            return false;
        };
        let file = FileBytes { path: p, len };
        let Ok(magic) = file.read(0, 4) else { return false };
        let expanded: Option<Vec<u8>> = if crate::archive::is_gzip(&magic) {
            let bytes = super::super::read_with_retry(p).ok();
            let name = p.file_name().map(|f| f.to_string_lossy().into_owned()).unwrap_or_default();
            bytes
                .and_then(|b| crate::archive::expand(&name, b).ok())
                .and_then(|mut e| (e.len() == 1).then(|| e.remove(0).bytes))
        } else {
            None
        };
        let src: &dyn ByteSource = match &expanded {
            Some(bytes) => bytes,
            None => &file,
        };
        let head = Fingerprint::of(&src.read(0, HEAD_BYTES as u64).unwrap_or_default());
        // Detached states first: they are waiting for their content to reappear.
        let mut ids: Vec<(bool, u32)> = self
            .tail
            .states
            .iter()
            .filter(|(_, s)| s.path != p && matches!(s.seed.mode, Mode::Append { .. }) && s.seed.offset > 0)
            .filter(|(_, s)| head.len >= s.seed.head.len)
            .map(|(id, s)| (s.attached, *id))
            .collect();
        ids.sort();
        for (attached, id) in ids {
            let Some(st) = self.tail.states.get(&id) else { continue };
            if !same_content(&st.seed, src) {
                continue;
            }
            if attached {
                if let Some(st) = self.tail.states.get_mut(&id) {
                    st.copies.push(p.to_path_buf());
                }
                return true;
            }
            let Some(mut st) = self.tail.states.remove(&id) else { continue };
            let from = st.path.clone();
            self.adopt(id, &mut st, p);
            report.rotated.push(RotatedSource {
                source_id: id,
                from: from.to_string_lossy().into_owned(),
                to: p.to_string_lossy().into_owned(),
            });
            // Lines written after the last read and before the rotation.
            self.follow_from(id, &mut st, src, delta, report, Instant::now());
            if expanded.is_some() {
                st.seed.mode = Mode::Whole;
                st.seed.len = len;
                st.seed.mtime = mtime(p);
                st.state = TailFileState::Rotated;
            }
            self.tail.states.insert(id, st);
            return true;
        }
        false
    }

    /// Registers text files at their current end instead of importing them;
    /// returns the files that must be imported normally (binary, archives,
    /// unrecognised, or importers that cannot append).
    pub(in crate::workspace) fn skip_history(&mut self, files: Vec<PathBuf>) -> Vec<PathBuf> {
        let mut rest = Vec::new();
        for path in files {
            let seed = crate::guard::catch("IMT", || Ok(self.seed_at_end(&path))).ok().flatten();
            let Some((importer, seed)) = seed else {
                rest.push(path);
                continue;
            };
            let name = display_name(&path, &self.roots);
            let id = self.add_source(
                path.to_string_lossy().into_owned(),
                name.clone(),
                importer.to_string(),
                0,
                Vec::new(),
            );
            self.tail.track(id, &path, &name, seed);
        }
        self.sources.sort_by(|a, b| a.name.cmp(&b.name));
        rest
    }

    fn seed_at_end(&self, path: &Path) -> Option<(String, TailSeed)> {
        let len = std::fs::metadata(path).ok()?.len();
        let src = FileBytes { path, len };
        let head = src.read(0, HEAD_TEXT_BYTES).ok()?;
        if head.is_empty() || crate::archive::is_gzip(&head) || crate::archive::is_zip(&head) {
            return None;
        }
        let file_date = mtime(path)
            .and_then(|t| t.duration_since(std::time::SystemTime::UNIX_EPOCH).ok())
            .and_then(|d| crate::time::utc_date_of(d.as_millis() as i64));
        let ctx = ImportCtx {
            profile: &self.profile,
            path,
            file_date,
        };
        if self.registry.importers.select_binary(&ctx, &head).is_some() {
            return None;
        }
        let text = crate::importers::decode(&head)?;
        let importer = self.registry.importers.select(&ctx, &text)?;
        importer.import_tail(&ctx, &text, "")?.ok()?;
        let (encoding, bom) = Encoding::detect(&head[..head.len().min(512)]);
        // Start after the last complete line.
        // Start of the last 64 KiB, on a code-unit boundary.
        let unit = if encoding == Encoding::Utf8 { 1 } else { 2 };
        let from = bom + (len.saturating_sub(HEAD_TEXT_BYTES).max(bom) - bom) / unit * unit;
        let last = src.read(from, len).ok()?;
        let complete = encoding.complete_len(&last) as u64;
        let offset = if complete == 0 && from == bom { bom } else { from + complete };
        let next_line = count_lines(&src, encoding, offset).ok()? + 1;
        let anchor = hash(&src.read(offset.saturating_sub(ANCHOR_BYTES), offset).ok()?);
        Some((
            importer.id().to_string(),
            TailSeed {
                identity: identity_of(path),
                head: Fingerprint::of(&head),
                len: offset,
                mtime: mtime(path),
                offset,
                anchor,
                resume: None,
                next_line,
                encoding,
                bom,
                mode: Mode::Append {
                    importer: importer.id().to_string(),
                },
            },
        ))
    }
}
