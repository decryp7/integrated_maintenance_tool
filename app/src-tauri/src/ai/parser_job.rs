//! What a parser generation reads (ADR 0032): the file of an open source, bounded, with its modification time and
//! the offsets other files of the same folder write; or, without a source, the sample the UI sent. Only paths of
//! sources the user opened are read. The engine cuts the samples; only its small model sample reaches the model.

use super::commands::sample_of;
use super::error::{AiCmdError, AiResult as Ai};
use imt_ai::StoreRef;
use imt_core::api::{ContextRequest, SourceInfo};
use imt_core::parser::verify::{zone, FileFacts};
use std::io::Read;
use std::path::Path;

/// Most bytes of a file read for a parser generation.
pub(crate) const MAX_FILE_BYTES: u64 = 16 << 20;
/// Most bytes read of another file of the same folder (for its offsets).
const SIBLING_BYTES: u64 = 16 << 10;
/// Most other files of the same folder looked at.
const MAX_SIBLINGS: usize = 8;

/// The text, name and facts of the file to generate a parser for.
#[derive(Debug, Default)]
pub(crate) struct ParserInput {
    pub content: String,
    pub file_name: String,
    pub facts: FileFacts,
}

/// The first `max` bytes of `path` as text (lossy UTF-8).
fn read_head(path: &Path, max: u64) -> std::io::Result<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(max).read_to_end(&mut bytes)?;
    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn modified_ms(path: &Path) -> Option<i64> {
    let t = std::fs::metadata(path).ok()?.modified().ok()?;
    let d = t.duration_since(std::time::UNIX_EPOCH).ok()?;
    i64::try_from(d.as_millis()).ok()
}

/// The file of `source` (an open source) with its facts; `None` when it cannot be read as a plain file.
pub(crate) fn source_input(source: &SourceInfo, all: &[SourceInfo]) -> Option<ParserInput> {
    let path = std::fs::canonicalize(&source.path).ok()?;
    if !path.is_file() {
        return None;
    }
    let content = read_head(&path, MAX_FILE_BYTES).ok()?;
    let folder = path.parent()?.to_path_buf();
    let machine_offsets = all
        .iter()
        .filter(|s| s.id != source.id)
        .filter_map(|s| std::fs::canonicalize(&s.path).ok())
        .filter(|p| p.parent() == Some(folder.as_path()) && p.is_file())
        .take(MAX_SIBLINGS)
        .filter_map(|p| read_head(&p, SIBLING_BYTES).ok())
        .flat_map(|text| zone::offsets_in(&text))
        .collect();
    Some(ParserInput {
        content,
        file_name: source.name.clone(),
        facts: FileFacts {
            modified_ms: modified_ms(&path),
            machine_offsets,
        },
    })
}

/// The input for a generation: the source's file (by id, or the source of `record_id`), else the sample.
pub(crate) fn parser_input(
    store: StoreRef<'_>,
    record_id: Option<u32>,
    source_id: Option<u32>,
    sample: Option<String>,
    file_name: Option<String>,
) -> Ai<ParserInput> {
    let sources = store.with(|s| s.summary()).map(|s| s.sources).unwrap_or_default();
    let source = match (source_id, record_id) {
        (Some(id), _) => Some(
            sources
                .iter()
                .find(|s| s.id == id)
                .ok_or_else(|| AiCmdError::not_found(format!("source {id} is not open")))?,
        ),
        (None, Some(record)) => store
            .with(|s| {
                s.source_context(&ContextRequest {
                    record_id: record,
                    before: 0,
                    after: 0,
                })
            })
            .ok()
            .and_then(|ctx| sources.iter().find(|s| s.name == ctx.source_name)),
        _ => None,
    };
    if let Some(input) = source.and_then(|s| source_input(s, &sources)) {
        return Ok(input);
    }
    // No readable file (an archive member, a removed file): the lines the UI sent or around the event.
    let (content, file_name) = sample_of(store, record_id, sample, file_name)?;
    Ok(ParserInput {
        content,
        file_name,
        facts: FileFacts::default(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn source(id: u32, path: &Path) -> SourceInfo {
        SourceInfo {
            id,
            name: path.file_name().unwrap().to_string_lossy().into_owned(),
            path: path.to_string_lossy().into_owned(),
            group: String::new(),
            importer: "text".into(),
            records: 1,
            skipped: 0,
            split_events: 0,
            dropped: 0,
            step_stats: Vec::new(),
            error: None,
            from: None,
            to: None,
            offset_ms: 0,
            file_kind: Default::default(),
            timestamp_issue: None,
            routed_from: None,
            problems: Vec::new(),
            identity: Vec::new(),
            parser: None,
        }
    }

    #[test]
    fn a_source_is_read_bounded_with_its_time_and_the_offsets_of_its_folder() {
        let dir = std::env::temp_dir().join(format!("imt-parser-job-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("other")).unwrap();
        let (app, web, far) = (dir.join("app.log"), dir.join("web.log"), dir.join("other/db.log"));
        std::fs::write(&app, "2026-10-14 10:00:00 INFO start\n").unwrap();
        std::fs::write(
            &web,
            "2026-10-14 10:00:00.000+09:00 [INF] up\n2026-10-14 10:00:01.000+09:00 [INF] ok\n",
        )
        .unwrap();
        std::fs::write(&far, "2026-10-14 10:00:00.000-05:00 x\n").unwrap();
        let all = [source(1, &app), source(2, &web), source(3, &far)];
        let input = source_input(&all[0], &all).unwrap();
        assert_eq!(input.content, "2026-10-14 10:00:00 INFO start\n");
        assert_eq!(input.file_name, "app.log");
        assert_eq!(input.facts.machine_offsets, [540, 540], "the same folder only");
        assert!(input.facts.modified_ms.is_some());
        assert!(source_input(&source(4, &dir.join("missing.log")), &all).is_none());
        assert!(source_input(&source(5, &dir), &all).is_none(), "a folder is not a file");
        std::fs::remove_dir_all(&dir).ok();
    }
}
