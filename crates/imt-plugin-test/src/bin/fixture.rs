//! A plugin that misbehaves on request, for the harness's own tests (never bundled).
//!
//! The text importer `fixture` reads the first line: `panic` panics, `hang` never answers (and ignores
//! cancellation), `fail` fails with an error, anything else gives one record per line. The tool `fixture.tool` has
//! `echo` (its arguments) and `read` (tries to read the path in `path`: what the sandbox allows). It asks for the
//! `network` permission, so tests can grant or withhold it.

use anyhow::{bail, Result};
use imt_plugin_api::importer::{ImportCtx, Imported, TextImporter};
use imt_plugin_api::manifest::{Permission, PermissionRequest};
use imt_plugin_api::model::Record;
use imt_plugin_api::tools::{OpSpec, Schema, Tool, ToolCtx, ToolInfo};
use imt_plugin_api::{Contributions, Plugin};
use serde_json::{json, Value};
use std::sync::Arc;

struct Fixture;

impl TextImporter for Fixture {
    fn id(&self) -> &str {
        "fixture"
    }

    fn sniff(&self, _: &ImportCtx, head: &str) -> u8 {
        if head.starts_with("#fixture") {
            100
        } else {
            10
        }
    }

    fn import(&self, _: &ImportCtx, content: &str) -> Result<Imported> {
        match content.lines().next().unwrap_or("") {
            "panic" => panic!("the fixture importer panicked"),
            "hang" => loop {
                std::thread::sleep(std::time::Duration::from_secs(3600));
            },
            "fail" => bail!("the fixture failed on purpose"),
            _ => Ok(Imported {
                records: content
                    .lines()
                    .enumerate()
                    .map(|(i, l)| {
                        let mut r = Record::new(i as u32 + 1);
                        r.message = l.to_string();
                        r
                    })
                    .collect(),
                ..Default::default()
            }),
        }
    }
}

struct FixtureTool;

impl Tool for FixtureTool {
    fn id(&self) -> &str {
        "fixture.tool"
    }

    fn describe(&self) -> ToolInfo {
        ToolInfo::new("fixture.tool", "Fixture")
            .op(OpSpec::new("echo", "Echo").args(Schema::Any))
            .op(OpSpec::new("read", "Read").args(Schema::object().field("path", Schema::text(4096))))
    }

    fn invoke(&self, op: &str, args: Value, _: &ToolCtx) -> Result<Value> {
        match op {
            "echo" => Ok(args),
            "read" => {
                let path = args["path"].as_str().unwrap_or_default();
                Ok(match std::fs::read(path) {
                    Ok(b) => json!({"read": b.len()}),
                    Err(e) => json!({"denied": e.to_string()}),
                })
            }
            _ => bail!("no operation {op}"),
        }
    }
}

struct FixturePlugin;

impl Plugin for FixturePlugin {
    fn id(&self) -> &str {
        "net.decryptology.test-fixture"
    }

    fn name(&self) -> &str {
        "Harness fixture"
    }

    fn version(&self) -> &str {
        "1.0.0"
    }

    fn permissions(&self) -> Vec<PermissionRequest> {
        vec![PermissionRequest {
            permission: Permission::Network,
            reason: "Tests the network permission.".into(),
        }]
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_importer(Fixture);
        c.add_tool(Arc::new(FixtureTool));
        Ok(())
    }
}

fn main() -> std::io::Result<()> {
    imt_plugin_api::serve(Arc::new(FixturePlugin))
}
