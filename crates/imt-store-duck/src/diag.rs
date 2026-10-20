//! The `Diagnostics` port: import problems as rows (the `diagnostics` table plus the files no importer
//! recognised), each with an engine-checked fix where one is known, and the evaluation of a proposed
//! fix (a profile override) against the stored events it would repair (ADR 0014 §5).

use crate::{engine, DuckStore, Inner};
use duckdb::params;
use imt_core::api::TimestampSuggestion;
use imt_core::batch::{SourceId, TsState};
use imt_core::importers::ImportCtx;
use imt_core::ingest::{Problem, ProblemKind, MAX_SAMPLES};
use imt_core::ports::{DiagnosticFix, DiagnosticRow, Diagnostics, FixEvaluation, QuickFix, StoreError, StoreResult};
use imt_core::profile::CompiledProfile;
use imt_core::timestamp_issue::suggest;
use std::path::Path;

/// Most stored events a fix is checked against.
pub const MAX_FIX_CHECKS: u64 = 10_000;
/// Characters of an event's text a timestamp is looked for in.
const SAMPLE_CHARS: usize = 512;

/// The raw text of up to `limit` untimed events of `source` (narrow: `raw` only, by line).
fn untimed_raw(conn: &duckdb::Connection, source: u32, limit: u64) -> StoreResult<Vec<String>> {
    let mut stmt = conn
        .prepare("SELECT raw FROM events WHERE source = ? AND ts_state = ? ORDER BY line LIMIT ?")
        .map_err(engine)?;
    let rows = stmt
        .query_map(params![source, TsState::Untimed.code(), limit], |r| r.get::<_, String>(0))
        .map_err(engine)?;
    rows.collect::<Result<Vec<_>, _>>().map_err(engine)
}

/// The texts of an event a timestamp may be: each cell of its first line (split at `,;|` and tabs) and
/// the first one to six words of each cell.
fn timestamp_candidates(raw: &str) -> Vec<&str> {
    let first = raw.lines().next().unwrap_or("");
    let first = first.char_indices().nth(SAMPLE_CHARS * 2).map_or(first, |(i, _)| &first[..i]);
    let mut out = Vec::new();
    for cell in first.split([',', ';', '|', '\t']).map(str::trim).filter(|c| !c.is_empty()) {
        out.push(cell);
        for word in cell.split_whitespace().take(5) {
            // The cell up to the end of this word (words are subslices of `cell`).
            let end = word.as_ptr() as usize - cell.as_ptr() as usize + word.len();
            out.push(&cell[..end]);
        }
    }
    out
}

/// The override that adds `format` to the profile's timestamp formats (lists replace in an override,
/// so the current ones are repeated).
fn timestamp_override(profile: &CompiledProfile, format: &str) -> String {
    let mut formats: Vec<toml::Value> = profile
        .profile
        .parse
        .timestamp_formats
        .iter()
        .map(|f| toml::Value::String(f.clone()))
        .collect();
    formats.push(toml::Value::String(format.to_string()));
    format!("[parse]\ntimestamp_formats = {}\n", toml::Value::Array(formats))
}

impl Inner {
    fn source_paths(&self, conn: &duckdb::Connection) -> StoreResult<Vec<(u32, String, String)>> {
        let mut stmt = conn.prepare("SELECT id, name, path FROM sources").map_err(engine)?;
        let rows = stmt.query_map([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?))).map_err(engine)?;
        rows.collect::<Result<Vec<_>, _>>().map_err(engine)
    }

    /// A timestamp format verified against the source's untimed events.
    fn timestamp_fix(&self, conn: &duckdb::Connection, source: u32, profile: &CompiledProfile) -> StoreResult<Option<DiagnosticFix>> {
        let raws = untimed_raw(conn, source, MAX_SAMPLES as u64)?;
        let firsts: Vec<String> = raws
            .iter()
            .map(|r| r.lines().next().unwrap_or("").chars().take(SAMPLE_CHARS).collect())
            .collect();
        let refs: Vec<&str> = firsts.iter().map(String::as_str).collect();
        let Some(p) = suggest::propose(&refs, &profile.timestamps) else {
            return Ok(None);
        };
        let profile_override = timestamp_override(profile, &p.format);
        Ok(Some(DiagnosticFix {
            title: format!(
                "Read timestamps like \"{}\" ({})",
                p.texts.first().cloned().unwrap_or_default(),
                p.format
            ),
            timestamp: Some(TimestampSuggestion {
                profile_snippet: profile_override.clone(),
                parsed_samples: p.texts.len(),
                checked_samples: refs.len(),
                sample: p.texts.first().cloned().unwrap_or_default(),
                format: p.format,
            }),
            profile_override,
        }))
    }

    fn problems(&self) -> StoreResult<Vec<DiagnosticRow>> {
        let conn = self.conn()?;
        let profile = self.profile();
        let mut stmt = conn
            .prepare(
                "SELECT d.source, s.name, s.path, d.kind, d.count, d.samples FROM diagnostics d JOIN sources s ON s.id = d.source ORDER BY s.name, d.kind",
            )
            .map_err(engine)?;
        let rows = stmt
            .query_map([], |r| {
                Ok((
                    r.get::<_, u32>(0)?,
                    r.get::<_, String>(1)?,
                    r.get::<_, String>(2)?,
                    r.get::<_, String>(3)?,
                    r.get::<_, u64>(4)?,
                    r.get::<_, String>(5)?,
                ))
            })
            .map_err(engine)?;
        let mut out = Vec::new();
        for row in rows {
            let (source, name, path, kind, count, samples) = row.map_err(engine)?;
            // Rows of kinds this build does not know (a newer writer) are skipped.
            let Some(kind) = ProblemKind::from_id(&kind) else { continue };
            let mut sample_lines: Vec<u32> = serde_json::from_str(&samples).unwrap_or_default();
            sample_lines.truncate(MAX_SAMPLES);
            let mut d = DiagnosticRow::new(SourceId(source), name, path, Problem { kind, count, sample_lines });
            if kind == ProblemKind::TimestampUnparsed {
                d.fix = self.timestamp_fix(&conn, source, &profile)?;
            }
            out.push(d);
        }
        for u in self.unrecognized() {
            out.push(DiagnosticRow {
                source: None,
                source_name: u.name,
                path: u.path,
                kind: ProblemKind::UnknownFormat,
                severity: ProblemKind::UnknownFormat.severity(),
                count: 1,
                sample_lines: Vec::new(),
                detail: Some(u.reason),
                fix: None,
            });
        }
        Ok(out)
    }

    /// Re-parses the stored text of the untimed events of the fix's sources (all sources with unreadable
    /// timestamps when it names none) with the candidate profile.
    fn evaluate_fix(&self, fix: &QuickFix) -> StoreResult<FixEvaluation> {
        let active = self.profile();
        let candidate = active
            .profile
            .with_override(&fix.profile_override)
            .map_err(|e| StoreError::Invalid(format!("the fix is not a valid profile override: {e}")))?;
        let compiled = self
            .registry()
            .compile_profile(candidate)
            .map_err(|e| StoreError::Invalid(format!("{e:#}")))?;
        let conn = self.conn()?;
        let wanted: Vec<u32> = fix.sources.iter().map(|s| s.0).collect();
        let affected: Vec<(u32, u64)> = {
            let mut stmt = conn
                .prepare("SELECT source, count FROM diagnostics WHERE kind = ? ORDER BY source")
                .map_err(engine)?;
            let rows = stmt
                .query_map(params![ProblemKind::TimestampUnparsed.id()], |r| Ok((r.get(0)?, r.get(1)?)))
                .map_err(engine)?;
            rows.collect::<Result<Vec<(u32, u64)>, _>>()
                .map_err(engine)?
                .into_iter()
                .filter(|(s, _)| wanted.is_empty() || wanted.contains(s))
                .collect()
        };
        let names = self.source_paths(&conn)?;
        let mut eval = FixEvaluation {
            fixed: 0,
            checked: 0,
            unchecked: 0,
            notes: Vec::new(),
        };
        for (source, count) in affected {
            let Some((_, name, path)) = names.iter().find(|(id, _, _)| *id == source) else {
                continue;
            };
            let budget = MAX_FIX_CHECKS.saturating_sub(eval.checked);
            let raws = untimed_raw(&conn, source, budget)?;
            eval.unchecked += count.saturating_sub(raws.len() as u64);
            if raws.is_empty() {
                continue;
            }
            let file = Path::new(path.split(imt_core::archive::ENTRY_SEPARATOR).next().unwrap_or(path));
            let ctx = ImportCtx {
                profile: &compiled,
                path: file,
                file_date: None,
            };
            let head: String = raws.iter().take(50).map(String::as_str).collect::<Vec<_>>().join("\n");
            let Some(importer) = self.registry().importers().select(&ctx, &head) else {
                eval.notes.push(format!("{name}: no importer reads it with this profile"));
                eval.checked += raws.len() as u64;
                continue;
            };
            let mut by_formats = false;
            for raw in &raws {
                eval.checked += 1;
                // A crashing or failing importer means "not fixed", never a failed evaluation.
                let parsed = imt_core::guard::catch("the importer", || importer.import(&ctx, raw)).ok();
                if parsed.is_some_and(|i| i.records.iter().any(|r| r.ts.is_some())) {
                    eval.fixed += 1;
                    continue;
                }
                // A single event cannot always be re-read alone (a CSV line without its header): then the
                // fix counts when the candidate's timestamp formats read a timestamp the active ones do not.
                let candidates = timestamp_candidates(raw);
                if candidates.iter().any(|c| compiled.timestamps.parse(c).is_some())
                    && !candidates.iter().any(|c| active.timestamps.parse(c).is_some())
                {
                    eval.fixed += 1;
                    by_formats = true;
                }
            }
            if by_formats {
                eval.notes.push(format!(
                    "{name}: some events were checked by the timestamp formats alone (they cannot be re-read without the rest of the file)"
                ));
            }
        }
        if eval.checked == 0 {
            eval.notes
                .push("no stored events have unreadable timestamps in these sources (skipped lines are not stored)".into());
        }
        Ok(eval)
    }
}

impl Diagnostics for DuckStore {
    fn problems(&self) -> StoreResult<Vec<DiagnosticRow>> {
        self.inner.problems()
    }

    fn evaluate_fix(&self, fix: &QuickFix) -> StoreResult<FixEvaluation> {
        self.inner.evaluate_fix(fix)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timestamp_candidates_are_cells_and_leading_words() {
        let c = timestamp_candidates("Sat Nov 08 00:04:37 2025,INFO,started\nmore");
        assert!(c.contains(&"Sat Nov 08 00:04:37 2025"), "{c:?}");
        assert!(c.contains(&"INFO") && c.contains(&"Sat Nov"));
        assert!(timestamp_candidates("").is_empty());
        let words = timestamp_candidates("2026-10-14 12:00:01 Something happened here today");
        assert!(words.contains(&"2026-10-14 12:00:01"), "{words:?}");
    }

    #[test]
    fn timestamp_overrides_keep_the_current_formats() {
        let profile =
            CompiledProfile::compile(imt_core::Profile::from_toml("[parse]\ntimestamp_formats = [\"%Y-%m-%d %H:%M:%S\"]").unwrap())
                .unwrap();
        let o = timestamp_override(&profile, "%d|%m|%Y %H.%M.%S");
        assert_eq!(o, "[parse]\ntimestamp_formats = [\"%Y-%m-%d %H:%M:%S\", \"%d|%m|%Y %H.%M.%S\"]\n");
        let merged = profile.profile.with_override(&o).unwrap();
        assert_eq!(merged.parse.timestamp_formats.len(), 2);
    }
}
