//! Debrief plugin for web server logs and HTTP captures.
//!
//! * **W3C extended / IIS logs** (`#Fields:` header): the header drives the
//!   columns and may change mid-file. The time is `date` + `time` (UTC per the
//!   spec); fields keep the W3C names (`cs-uri-stem`, `sc-status` and
//!   `time-taken` as numbers, `-` placeholders omitted).
//! * **Apache / Nginx common and combined logs**, with an optional trailing
//!   request time. Fields: `ClientIp`, `User`, `Method`, `Path`, `Protocol`,
//!   `Status`, `Bytes`, `Referer`, `UserAgent`, `RequestTimeMs`. A trailing
//!   decimal is read as seconds (nginx `$request_time`), an integer as
//!   microseconds (Apache `%D`).
//! * **HAR** (`log.entries`): fields `Method`, `Url`, `Status`, `StatusText`,
//!   `MimeType`, `TimeMs`, `RequestBytes`, `ResponseBytes`, `ServerIp`.
//!
//! The level follows the HTTP status: 5xx Error, 4xx Warning, otherwise
//! Information (a HAR status of 0 is a failed request: Error).
//!
//! It runs in its own process (`debrief-plugins --plugin net.decryptology.web`, ADR 0017).

mod access;
mod har;
mod w3c;

pub use access::AccessLogImporter;
pub use har::HarImporter;
pub use w3c::W3cImporter;

use anyhow::Result;
use debrief_plugin_api::model::Level;
use debrief_plugin_api::{Contributions, Plugin};

pub struct WebPlugin;

impl Plugin for WebPlugin {
    fn id(&self) -> &'static str {
        "net.decryptology.web"
    }

    fn name(&self) -> &'static str {
        "Web server logs"
    }

    fn version(&self) -> &'static str {
        env!("CARGO_PKG_VERSION")
    }

    fn register(&self, c: &mut Contributions) -> Result<()> {
        c.add_importer(W3cImporter);
        c.add_importer(AccessLogImporter);
        c.add_importer(HarImporter);
        Ok(())
    }
}

/// 5xx Error, 4xx Warning, everything else Information.
pub(crate) fn level_for_status(status: i64) -> Level {
    match status {
        500..=599 => Level::Error,
        400..=499 => Level::Warning,
        _ => Level::Information,
    }
}

#[cfg(test)]
pub(crate) mod testutil {
    use debrief_plugin_api::importer::ImportCtx;
    use debrief_plugin_api::time::TimestampParser;
    use std::path::Path;

    pub fn with_ctx<T>(f: impl FnOnce(&ImportCtx) -> T) -> T {
        let ts = TimestampParser::default();
        let mut ctx = ImportCtx::new(Path::new("x"), &ts);
        ctx.file_date = None;
        f(&ctx)
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
        WebPlugin.register(&mut c).unwrap();
        let info = c.manifest(&WebPlugin);
        assert_eq!(&*info.id, "net.decryptology.web");
        assert_eq!(info.contributions().len(), 3);
        assert_eq!(WebPlugin.name(), "Web server logs");
        assert_eq!(WebPlugin.version(), env!("CARGO_PKG_VERSION"));
        assert_eq!(level_for_status(503), Level::Error);
        assert_eq!(level_for_status(404), Level::Warning);
        assert_eq!(level_for_status(200), Level::Information);
        assert_eq!(level_for_status(0), Level::Information);
    }
}
