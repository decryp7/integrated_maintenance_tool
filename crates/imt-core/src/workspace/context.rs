//! The original lines around an event, read back from its source file.

use super::Workspace;
use crate::api::{ContextRequest, SourceContext, SourceLine};
use crate::archive::{self, ENTRY_SEPARATOR};
use crate::importers::decode;
use anyhow::{anyhow, Context, Result};

/// Most lines returned on either side of the event.
const MAX_RADIUS: usize = 500;

impl Workspace {
    pub fn source_context(&self, req: &ContextRequest) -> Result<SourceContext> {
        let record = self
            .records
            .get(req.record_id as usize)
            .ok_or_else(|| anyhow!("no event #{}", req.record_id))?;
        let source = self
            .sources
            .iter()
            .find(|s| s.id == record.source)
            .ok_or_else(|| anyhow!("the event's file was removed"))?;
        let (file, entry) = match source.path.split_once(ENTRY_SEPARATOR) {
            Some((f, e)) => (f, Some(e)),
            None => (source.path.as_str(), None),
        };
        let bytes = std::fs::read(file).with_context(|| format!("cannot read {}", source.name))?;
        let file_name = std::path::Path::new(file)
            .file_name()
            .map(|f| f.to_string_lossy().into_owned())
            .unwrap_or_default();
        let bytes = match entry {
            Some(e) => archive::read_entry(bytes, &file_name, e)?,
            None if archive::is_gzip(&bytes) => archive::expand(&file_name, bytes)?.remove(0).bytes,
            None => bytes,
        };
        let text = decode(&bytes).ok_or_else(|| anyhow!("{} is no longer a text file", source.name))?;
        let line = record.line.max(1) as usize;
        let first = line.saturating_sub(req.before.min(MAX_RADIUS)).max(1);
        let last = line + req.after.min(MAX_RADIUS);
        let lines = text
            .lines()
            .enumerate()
            .skip(first - 1)
            .take(last + 1 - first)
            .map(|(i, l)| SourceLine {
                line: i as u32 + 1,
                text: self.view_line(l.trim_end_matches('\r').chars().take(10_000).collect()),
            })
            .collect();
        Ok(SourceContext {
            source_name: source.name.clone(),
            line: record.line,
            lines,
            note: None,
        })
    }
}
