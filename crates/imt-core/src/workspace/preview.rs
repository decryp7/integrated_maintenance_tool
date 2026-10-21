//! Dry-run of an unsaved extractor against the current records.

use super::{ExtractPreview, ExtractPreviewRequest, ExtractSample, ExtractSpan, Workspace};
use crate::model::Record;
use crate::profile::{CompiledProfile, Extractor, Profile};
use anyhow::{anyhow, Result};

/// The extractor named `name` of a candidate profile (TOML), compiled with the installed plugins.
pub fn preview_profile(registry: &crate::PluginRegistry, toml: &str, name: &str) -> Result<(CompiledProfile, usize)> {
    let profile = Profile::from_toml(toml).map_err(|e| anyhow!(e.to_string()))?;
    let compiled = registry.compile_profile(profile)?;
    let i = compiled
        .extractors
        .iter()
        .position(|e| e.name == name)
        .ok_or_else(|| anyhow!("no [[extract]] named \"{name}\""))?;
    Ok((compiled, i))
}

/// Collects an extractor's dry run over events, one at a time.
pub struct ExtractPreviewer<'e> {
    ex: &'e Extractor,
    limit: usize,
    out: ExtractPreview,
}

impl<'e> ExtractPreviewer<'e> {
    pub fn new(ex: &'e Extractor, limit: usize) -> ExtractPreviewer<'e> {
        ExtractPreviewer {
            ex,
            limit,
            out: ExtractPreview::default(),
        }
    }

    pub fn add(&mut self, r: &Record, source: &str) {
        let (ex, out) = (self.ex, &mut self.out);
        if !ex.applies(r, source) {
            return;
        }
        out.candidates += 1;
        let Some(text) = r.field_text(&ex.from, source) else {
            out.failed += 1;
            return;
        };
        match ex.regex.captures(&text) {
            Some(caps) => {
                out.matches += 1;
                if out.samples.len() < self.limit {
                    let text: String = text.chars().take(1000).collect();
                    let spans = ex
                        .regex
                        .capture_names()
                        .flatten()
                        .filter_map(|n| caps.name(n).map(|m| (n, m)))
                        .filter(|(_, m)| m.end() <= text.len())
                        .map(|(n, m)| ExtractSpan {
                            name: n.to_string(),
                            start: utf16_len(&text[..m.start()]),
                            end: utf16_len(&text[..m.end()]),
                        })
                        .collect();
                    out.samples.push(ExtractSample {
                        record_id: r.id,
                        values: ex.convert(&caps).into_iter().collect(),
                        text,
                        spans,
                    });
                }
            }
            None => out.failed += 1,
        }
    }

    pub fn finish(self) -> ExtractPreview {
        self.out
    }
}

impl Workspace {
    /// Runs one extractor of an (unsaved) profile against the current records.
    pub fn preview_extractor(&self, req: &ExtractPreviewRequest) -> ExtractPreview {
        let run = || -> Result<ExtractPreview> {
            let (compiled, i) = preview_profile(&self.registry, &req.toml, &req.extractor)?;
            let ids = self.matching(&req.query)?;
            let mut previewer = ExtractPreviewer::new(&compiled.extractors[i], req.limit);
            for &id in ids.iter() {
                let r = &self.records[id as usize];
                previewer.add(r, self.source_name(r.source));
            }
            Ok(previewer.finish())
        };
        run().unwrap_or_else(|e| ExtractPreview {
            error: Some(format!("{e:#}")),
            ..Default::default()
        })
    }
}

fn utf16_len(s: &str) -> usize {
    s.encode_utf16().count()
}
