//! Writing the events matching a query with one of the registered exporters,
//! and access to the other plugin contributions the workspace runs.

use super::Workspace;
use crate::api::{ExportRequest, ExportResult};
use crate::export::ExportRow;
use anyhow::{anyhow, Context, Result};
use std::io::BufWriter;

impl Workspace {
    /// Settings of the installed plugins, sharing the workspace registry's
    /// guards: a crash while applying settings counts against the same
    /// health record as the plugin's importers and transforms.
    pub fn plugin_settings(&self) -> Vec<crate::settings::PluginSettings> {
        self.registry.plugin_settings()
    }

    /// Streams every record matching `req.query` to `req.path` in `req.format`.
    pub fn export(&self, req: &ExportRequest) -> Result<ExportResult> {
        let exporter = self
            .registry
            .exporter(&req.format)
            .ok_or_else(|| anyhow!("unknown export format \"{}\"", req.format))?
            .clone();
        let ids = self.matching(&req.query)?;
        let cq = self.compile(&req.query)?;
        let file = std::fs::File::create(&req.path).with_context(|| format!("cannot create {}", req.path))?;
        let mut out = BufWriter::new(file);
        let order: Box<dyn Iterator<Item = &u32>> = if req.descending {
            Box::new(ids.iter().rev())
        } else {
            Box::new(ids.iter())
        };
        let mut rows = order.map(|&id| {
            let record = &self.records[id as usize];
            let source_name = self.source_name(record.source);
            ExportRow {
                record,
                source_name,
                group: cq.group_key(record, source_name),
            }
        });
        let events = exporter.export(&mut rows, &mut out)?;
        Ok(ExportResult {
            format: exporter.id().into(),
            path: req.path.clone(),
            events,
        })
    }
}
