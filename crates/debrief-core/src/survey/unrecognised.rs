//! Files no importer recognised, grouped by extension, with the first lines of one file per group (the
//! sample a format proposal is built and previewed on).

use super::dto::UnrecognisedGroup;
use crate::api::UnrecognizedFile;
use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

const MAX_GROUPS: usize = 16;
const MAX_NAMES: usize = 20;
const HEAD_BYTES: u64 = 16 * 1024;
const HEAD_LINES: usize = 30;
const LINE_CHARS: usize = 300;

pub(super) fn groups(files: &[UnrecognizedFile]) -> Vec<UnrecognisedGroup> {
    let mut by_ext: BTreeMap<String, Vec<&UnrecognizedFile>> = BTreeMap::new();
    for f in files {
        by_ext.entry(extension(&f.name)).or_default().push(f);
    }
    let mut out: Vec<UnrecognisedGroup> = by_ext
        .into_iter()
        .map(|(extension, files)| {
            let mut reasons: BTreeMap<&str, usize> = BTreeMap::new();
            for f in &files {
                *reasons.entry(f.reason.as_str()).or_default() += 1;
            }
            let reason = reasons
                .iter()
                .max_by_key(|(_, n)| **n)
                .map(|(r, _)| r.to_string())
                .unwrap_or_default();
            let (sample_file, sample_lines) = files
                .iter()
                .find_map(|f| head_lines(&f.path).filter(|l| !l.is_empty()).map(|l| (Some(f.name.clone()), l)))
                .unwrap_or((None, Vec::new()));
            UnrecognisedGroup {
                extension,
                file_count: files.len(),
                files: files.iter().take(MAX_NAMES).map(|f| f.name.clone()).collect(),
                reason,
                sample_file,
                sample_lines,
            }
        })
        .collect();
    out.sort_by(|a, b| b.file_count.cmp(&a.file_count).then(a.extension.cmp(&b.extension)));
    out.truncate(MAX_GROUPS);
    out
}

/// Lower-case extension of a file name, or `(none)`.
pub(super) fn extension(name: &str) -> String {
    let file = name.rsplit(['/', '\\']).next().unwrap_or(name);
    match file.rsplit_once('.') {
        Some((stem, ext)) if !stem.is_empty() && !ext.is_empty() && ext.len() <= 12 => ext.to_ascii_lowercase(),
        _ => "(none)".into(),
    }
}

/// The first text lines of a file the user opened (archive members and binary files give none).
pub(super) fn head_lines(path: &str) -> Option<Vec<String>> {
    if path.contains(crate::archive::ENTRY_SEPARATOR) {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(Path::new(path))
        .ok()?
        .take(HEAD_BYTES)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.is_empty() || crate::archive::is_zip(&bytes) || crate::archive::is_gzip(&bytes) {
        return None;
    }
    let text = crate::importers::decode(&bytes)?;
    let lines: Vec<String> = text
        .lines()
        .map(|l| l.trim_end())
        .filter(|l| !l.is_empty())
        .take(HEAD_LINES)
        .map(|l| l.chars().take(LINE_CHARS).collect())
        .collect();
    Some(lines)
}
