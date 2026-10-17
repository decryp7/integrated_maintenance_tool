//! Dry-run of an unsaved extractor against the current records.

use super::{ExtractPreview, ExtractPreviewRequest, ExtractSample, ExtractSpan, Workspace};
use crate::profile::{CompiledProfile, Profile};
use anyhow::{anyhow, Result};

impl Workspace {
    /// Runs one extractor of an (unsaved) profile against the current records.
    pub fn preview_extractor(&self, req: &ExtractPreviewRequest) -> ExtractPreview {
        let run = || -> Result<ExtractPreview> {
            let profile = Profile::from_toml(&req.toml).map_err(|e| anyhow!(e.to_string()))?;
            let compiled = CompiledProfile::compile(profile)?;
            let ex = compiled
                .extractors
                .iter()
                .find(|e| e.name == req.extractor)
                .ok_or_else(|| anyhow!("no [[extract]] named \"{}\"", req.extractor))?;
            let ids = self.matching(&req.query)?;
            let mut out = ExtractPreview::default();
            for &id in ids.iter() {
                let r = &self.records[id as usize];
                let source = self.source_name(r.source);
                if !ex.applies(r, source) {
                    continue;
                }
                out.candidates += 1;
                let Some(text) = r.field_text(&ex.from, source) else {
                    out.failed += 1;
                    continue;
                };
                match ex.regex.captures(&text) {
                    Some(caps) => {
                        out.matches += 1;
                        if out.samples.len() < req.limit {
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
            Ok(out)
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
