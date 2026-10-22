//! An example tool plugin: an epoch converter. Tools are operations the UI (or the AI assistant) calls with JSON
//! arguments checked against the operation's schema; the result is JSON.
//!
//! - `convert { value }`: a Unix time in seconds or milliseconds → its UTC date and time;
//! - `now`: the current time in milliseconds.

use anyhow::{bail, Result};
use debrief_plugin_api::prelude::*;
use debrief_plugin_api::tools::{OpSpec, Schema};
use serde_json::{json, Value};
use std::sync::Arc;

/// The tool.
pub struct Epoch;

/// `ms` as `YYYY-MM-DDTHH:MM:SS.mmmZ` (proleptic Gregorian, UTC).
pub fn iso(ms: i64) -> String {
    let days = ms.div_euclid(86_400_000);
    let rem = ms.rem_euclid(86_400_000);
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z",
        rem / 3_600_000,
        rem / 60_000 % 60,
        rem / 1000 % 60,
        rem % 1000
    )
}

impl Tool for Epoch {
    fn id(&self) -> &str {
        "epoch"
    }

    fn describe(&self) -> ToolInfo {
        ToolInfo::new("epoch", "Epoch converter")
            .op(OpSpec::new("convert", "Convert").args(Schema::object().field("value", Schema::int(-(1 << 52), 1 << 52))))
            .op(OpSpec::new("now", "Now"))
    }

    fn invoke(&self, op: &str, args: Value, _ctx: &ToolCtx) -> Result<Value> {
        match op {
            "convert" => {
                let v = args["value"].as_i64().unwrap_or_default();
                // Seconds until the year 5138; larger values are milliseconds.
                let ms = if v.abs() < 100_000_000_000 { v * 1000 } else { v };
                Ok(json!({ "ms": ms, "utc": iso(ms) }))
            }
            "now" => {
                let ms = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_millis() as i64)
                    .unwrap_or_default();
                Ok(json!({ "ms": ms, "utc": iso(ms) }))
            }
            _ => bail!("no operation \"{op}\""),
        }
    }
}

/// The plugin.
pub struct EpochPlugin;

impl Plugin for EpochPlugin {
    fn id(&self) -> &str {
        "net.decryptology.example.tool"
    }

    fn name(&self) -> &str {
        "Epoch converter (example)"
    }

    fn version(&self) -> &str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_tool(Arc::new(Epoch));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn iso_dates() {
        assert_eq!(super::iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(super::iso(1_792_576_800_123), "2026-10-21T10:00:00.123Z");
        assert_eq!(super::iso(-1), "1969-12-31T23:59:59.999Z");
    }
}
