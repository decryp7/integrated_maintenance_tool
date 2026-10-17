//! Performance Monitor binary logs (`.blg`).
//!
//! The format is proprietary and only Windows can read it, so the importer asks
//! `relog` (shipped with Windows) to convert the file to CSV and parses that with
//! the PerfMon CSV logic. Elsewhere it explains how to convert the file.

use crate::command::{CommandRunner, SystemRunner, DEFAULT_TIMEOUT};
use crate::perfmon::parse_perfmon;
use anyhow::{anyhow, bail, Context, Result};
use imt_core::importers::{decode, BinaryImporter, ImportCtx, Imported};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

pub const UNSUPPORTED: &str =
    "BLG files can only be read on Windows (Performance Monitor's format). Convert with: relog file.blg -f csv -o file.csv";

pub struct BlgImporter {
    runner: Box<dyn CommandRunner>,
    windows: bool,
}

impl Default for BlgImporter {
    fn default() -> Self {
        BlgImporter::with_runner(Box::new(SystemRunner), cfg!(windows))
    }
}

impl BlgImporter {
    /// `windows` tells whether `relog` is available; injected so both paths are testable everywhere.
    pub fn with_runner(runner: Box<dyn CommandRunner>, windows: bool) -> Self {
        BlgImporter { runner, windows }
    }

    fn relog(&self, input: &Path) -> Result<Vec<u8>> {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let tmp: PathBuf = std::env::temp_dir().join(format!(
            "imt-blg-{}-{}.csv",
            std::process::id(),
            COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let args: Vec<String> = [
            input.to_string_lossy().as_ref(),
            "-f",
            "csv",
            "-o",
            tmp.to_string_lossy().as_ref(),
            "-y",
        ]
        .map(String::from)
        .to_vec();
        let result = (|| {
            let out = self
                .runner
                .run("relog", &args, DEFAULT_TIMEOUT)
                .context("could not run relog (is it on the PATH?)")?;
            if !out.success() {
                let detail = if out.stderr.trim().is_empty() {
                    out.stdout.trim()
                } else {
                    out.stderr.trim()
                };
                bail!(
                    "relog failed ({}): {detail}",
                    out.code.map_or("killed".to_string(), |c| c.to_string())
                );
            }
            std::fs::read(&tmp).map_err(|e| anyhow!("relog produced no CSV: {e}"))
        })();
        let _ = std::fs::remove_file(&tmp);
        result
    }
}

impl BinaryImporter for BlgImporter {
    fn id(&self) -> &'static str {
        "blg"
    }

    fn sniff(&self, ctx: &ImportCtx, head: &[u8]) -> u8 {
        let blg = ctx
            .path
            .extension()
            .is_some_and(|e| e.to_string_lossy().eq_ignore_ascii_case("blg"));
        if blg && decode(head).is_none() {
            90
        } else {
            0
        }
    }

    fn import(&self, ctx: &ImportCtx, _bytes: &[u8]) -> Result<Imported> {
        if !self.windows {
            bail!(UNSUPPORTED);
        }
        let csv = self.relog(ctx.path)?;
        let text = decode(&csv).ok_or_else(|| anyhow!("relog produced an unreadable CSV"))?;
        parse_perfmon(&text, &ctx.profile.timestamps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::fake::{fail, ok, FakeRunner};
    use crate::command::CommandOutput;
    use imt_core::profile::{CompiledProfile, Profile};
    use std::sync::Arc;

    const CSV: &str = "\"(PDH-CSV 4.0) (UTC)(0)\",\"\\\\M\\Memory\\Pages/sec\"\n\"10/14/2026 09:00:00.000\",\"3.5\"\n";

    fn profile() -> CompiledProfile {
        CompiledProfile::compile(Profile::from_toml("").unwrap()).unwrap()
    }

    /// A fake relog that writes `csv` to the `-o` path.
    fn relog_writing(csv: &'static str) -> Arc<FakeRunner> {
        Arc::new(FakeRunner::new(move |program, args| {
            assert_eq!(program, "relog");
            let out = args.iter().position(|a| a == "-o").map(|i| &args[i + 1]).unwrap();
            std::fs::write(out, csv).unwrap();
            ok("")
        }))
    }

    struct Shared(Arc<FakeRunner>);
    impl CommandRunner for Shared {
        fn run(&self, p: &str, a: &[String], t: std::time::Duration) -> std::io::Result<CommandOutput> {
            self.0.run(p, a, t)
        }
    }

    fn importer(fake: &Arc<FakeRunner>, windows: bool) -> BlgImporter {
        BlgImporter::with_runner(Box::new(Shared(fake.clone())), windows)
    }

    #[test]
    fn sniffs_by_extension_and_binary_content() {
        let p = profile();
        let ctx = |name: &'static str| ImportCtx {
            profile: &p,
            path: Path::new(name),
            file_date: None,
        };
        let blg = BlgImporter::default();
        assert_eq!(blg.id(), "blg");
        let binary = [0u8, 1, 2, 3, 4, 5, 6, 7, 0, 0, 0, 1, 2, 3, 0xFF];
        assert_eq!(blg.sniff(&ctx("DataCollector01.BLG"), &binary), 90);
        assert_eq!(blg.sniff(&ctx("x.log"), &binary), 0);
        assert_eq!(blg.sniff(&ctx("x.blg"), b"plain text"), 0);
    }

    #[test]
    fn converts_with_relog_and_parses_the_csv() {
        let fake = relog_writing(CSV);
        let p = profile();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("C:\\logs\\a.blg"),
            file_date: None,
        };
        let out = importer(&fake, true).import(&ctx, &[0, 1, 2]).unwrap();
        assert_eq!(out.records.len(), 1);
        assert_eq!(out.records[0].fields["Value"].as_f64(), Some(3.5));
        let call = &fake.calls()[0];
        assert!(
            call.starts_with("relog C:\\logs\\a.blg -f csv -o ") && call.ends_with(".csv -y"),
            "{call}"
        );
        // The temporary CSV is removed.
        let tmp = call.split(" -o ").nth(1).unwrap().trim_end_matches(" -y");
        assert!(!Path::new(tmp).exists());
    }

    #[test]
    fn reports_relog_problems() {
        let p = profile();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("a.blg"),
            file_date: None,
        };
        let run = |fake: Arc<FakeRunner>| importer(&fake, true).import(&ctx, &[]).err().unwrap().to_string();
        let msg = run(Arc::new(FakeRunner::new(|_, _| fail(1, "", "Error: file is corrupt"))));
        assert!(msg.contains("relog failed (1): Error: file is corrupt"), "{msg}");
        let msg = run(Arc::new(FakeRunner::new(|_, _| fail(2, "only stdout", ""))));
        assert!(msg.contains("only stdout"), "{msg}");
        let msg = run(Arc::new(FakeRunner::new(|_, _| Ok(CommandOutput::default()))));
        assert!(msg.contains("relog failed (killed)"), "{msg}");
        let msg = run(Arc::new(FakeRunner::new(|_, _| ok(""))));
        assert!(msg.contains("produced no CSV"), "{msg}");
        let msg = run(Arc::new(FakeRunner::new(|_, _| {
            Err(std::io::Error::new(std::io::ErrorKind::NotFound, "nope"))
        })));
        assert!(msg.contains("could not run relog"), "{msg}");
        let msg = run(relog_writing("\u{0}\u{1}\u{2}\u{3}\u{4}\u{5}\u{6}\u{7}\u{0}\u{0}"));
        assert!(msg.contains("unreadable CSV"), "{msg}");
    }

    #[test]
    fn explains_the_limitation_off_windows() {
        let fake = Arc::new(FakeRunner::new(|_, _| ok("")));
        let p = profile();
        let ctx = ImportCtx {
            profile: &p,
            path: Path::new("a.blg"),
            file_date: None,
        };
        let msg = importer(&fake, false).import(&ctx, &[]).err().unwrap().to_string();
        assert_eq!(
            msg,
            "BLG files can only be read on Windows (Performance Monitor's format). Convert with: relog file.blg -f csv -o file.csv"
        );
        assert!(fake.calls().is_empty());
    }
}
