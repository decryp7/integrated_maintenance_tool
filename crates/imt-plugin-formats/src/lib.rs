//! IMT plugin for common application, database and container log formats.
//!
//! * **log4j / log4net XML events** (`log4j-xml`): `<log4j:event>` elements as
//!   written by log4j 1.x `XMLLayout`, log4net `XmlLayoutSchemaLog4j` and
//!   NLog `Log4JXmlEventLayout`, plus log4net's own `XmlLayout`
//!   (`<log4net:event>`, ISO timestamps). Events are read one at a time, so a
//!   missing root element, a truncated last event or one malformed event only
//!   costs that event. Fields: `Logger`, `Thread`, `NDC`, `Class`, `Method`,
//!   `File`, `Line`, `MachineName`, `Application`, properties by name.
//! * **SQL Server ERRORLOG** (`sql-errorlog`), UTF-16 LE or UTF-8:
//!   `2026-10-17 12:00:00.12 spid52      message`. Continuation lines join the
//!   record before them; `Error: 18456, Severity: 14, State: 8.` folds into the
//!   message that follows as `ErrorNumber`, `Severity`, `State`, and the
//!   severity sets the level. Times are local to the server and converted with
//!   the log's own `UTC adjustment` line (else the profile's assumed zone).
//! * **Docker json-file logs** (`docker-json`) and **Kubernetes CRI logs**
//!   (`cri`): partial lines are joined, `Stream` is a field, the level comes
//!   from the message (`ERROR`, `level=warn`, `"level":"error"`, klog `E1017`)
//!   or else from the stream (stderr Warning, stdout Information). Pod,
//!   namespace and container come from the standard log paths.
//!
//! It runs in its own process (`imt-plugins --plugin net.decryptology.formats`, ADR 0017).

mod container;
mod cri;
mod docker;
mod errorlog;
mod log4j;

pub use cri::CriImporter;
pub use docker::DockerJsonImporter;
pub use errorlog::SqlErrorLogImporter;
pub use log4j::Log4jXmlImporter;

use anyhow::Result;
use imt_plugin_api::{Contributions, Plugin};

/// Most fields one record keeps from properties or unknown elements, so a
/// hostile file cannot make one record arbitrarily wide.
pub(crate) const MAX_FIELDS: usize = 128;

pub struct FormatsPlugin;

impl Plugin for FormatsPlugin {
    fn id(&self) -> &'static str {
        "net.decryptology.formats"
    }

    fn name(&self) -> &'static str {
        "Application and container log formats"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_importer(Log4jXmlImporter);
        c.add_importer(SqlErrorLogImporter);
        c.add_importer(DockerJsonImporter);
        c.add_importer(CriImporter);
        Ok(())
    }
}

/// The first non-blank line, without a byte order mark.
pub(crate) fn first_line(head: &str) -> Option<&str> {
    head.trim_start_matches('\u{feff}')
        .lines()
        .map(str::trim_end)
        .find(|l| !l.trim().is_empty())
}

#[cfg(test)]
pub(crate) mod testutil {
    use imt_plugin_api::importer::ImportCtx;
    use imt_plugin_api::time::TimestampParser;
    use std::path::Path;

    pub fn with_ctx<T>(f: impl FnOnce(&ImportCtx) -> T) -> T {
        with_path("x", f)
    }

    pub fn with_path<T>(path: &str, f: impl FnOnce(&ImportCtx) -> T) -> T {
        let ts = TimestampParser::default();
        let mut ctx = ImportCtx::new(Path::new(path), &ts);
        ctx.file_date = None;
        f(&ctx)
    }

    /// Epoch milliseconds of an RFC 3339 time.
    pub fn ms(s: &str) -> i64 {
        chrono::DateTime::parse_from_rfc3339(s).unwrap().timestamp_millis()
    }

    /// Deterministic junk built from format-relevant fragments.
    pub fn garbage(pieces: &[&str], rounds: usize) -> Vec<String> {
        let mut x: u32 = 99;
        (0..rounds)
            .map(|_| {
                (0..60)
                    .map(|_| {
                        x = x.wrapping_mul(1664525).wrapping_add(1013904223);
                        pieces[(x >> 16) as usize % pieces.len()]
                    })
                    .collect()
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plugin_registers_its_importers() {
        let mut c = Contributions::default();
        FormatsPlugin.register(&mut c).unwrap();
        let info = c.manifest(&FormatsPlugin);
        assert_eq!(&*info.id, "net.decryptology.formats");
        assert_eq!(info.contributions().len(), 4);
        assert_eq!(FormatsPlugin.name(), "Application and container log formats");
        assert_eq!(FormatsPlugin.version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(first_line("\u{feff}\n  \n a \nb"), Some(" a"));
        assert_eq!(first_line(""), None);
    }
}
