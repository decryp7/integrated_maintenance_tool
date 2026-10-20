//! macOS: making a downloaded, already verified llama-server runnable (ADR 0009).
//!
//! IMT downloads with its own HTTP client, so the files normally carry no quarantine attribute; any that
//! do (a copy through a browser, a managed Mac) have it removed, but only after the SHA-256 check. Apple
//! Silicon runs only signed code: the official builds are ad-hoc signed by their linker, and when
//! `codesign --verify` rejects a file it is ad-hoc signed again here. llama-server runs as a separate
//! process, so IMT's hardened runtime and library validation do not apply to it. Elsewhere this is a no-op.

use crate::error::{AiError, Result};
use std::path::Path;
use std::process::Stdio;
use std::time::{Duration, Instant};

/// A command line.
pub type Argv = Vec<String>;

/// The tool invocations that prepare `files` in `dir`: remove quarantine from every file, then verify
/// and (when verification fails) ad-hoc sign every Mach-O file (everything but licences).
pub fn plan(dir: &Path, files: &[String]) -> (Vec<Argv>, Vec<(Argv, Argv)>) {
    let path = |f: &str| dir.join(f).to_string_lossy().into_owned();
    let unquarantine = files
        .iter()
        .map(|f| vec!["/usr/bin/xattr".into(), "-d".into(), "com.apple.quarantine".into(), path(f)])
        .collect();
    let signing = files
        .iter()
        .filter(|f| !f.starts_with("LICENSE") && f.as_str() != super::store::MARKER)
        .map(|f| {
            (
                vec!["/usr/bin/codesign".into(), "--verify".into(), path(f)],
                vec!["/usr/bin/codesign".into(), "--force".into(), "--sign".into(), "-".into(), path(f)],
            )
        })
        .collect();
    (unquarantine, signing)
}

/// Runs a tool with a timeout; `Ok(true)` when it succeeded.
fn run(argv: &[String], timeout: Duration) -> Result<bool> {
    let Some((exe, args)) = argv.split_first() else { return Ok(false) };
    let mut child = imt_core::process::quiet_command(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| AiError::Io(format!("cannot run {exe}: {e}")))?;
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait() {
            Ok(Some(s)) => return Ok(s.success()),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(AiError::Io(format!("{exe} did not finish in time")));
            }
        }
    }
}

/// Prepares an unpacked, verified build folder (macOS only).
pub fn after_verified_unpack(dir: &Path) -> Result<()> {
    if !cfg!(target_os = "macos") {
        return Ok(());
    }
    let files: Vec<String> = std::fs::read_dir(dir)?
        .flatten()
        .filter(|e| e.file_type().is_ok_and(|t| t.is_file()))
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    let (unquarantine, signing) = plan(dir, &files);
    for argv in unquarantine {
        // Fails harmlessly when the attribute is not there.
        let _ = run(&argv, Duration::from_secs(10))?;
    }
    for (verify, sign) in signing {
        if !run(&verify, Duration::from_secs(30))? && !run(&sign, Duration::from_secs(60))? {
            return Err(AiError::Refused(format!(
                "macOS refused to accept {} (code signing failed); download the build again or set a custom llama-server path",
                verify.last().map(String::as_str).unwrap_or("llama-server")
            )));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plan_unquarantines_everything_and_signs_code_only() {
        let files = vec![
            "llama-server".to_string(),
            "libllama.0.dylib".into(),
            "LICENSE".into(),
            "runtime.json".into(),
        ];
        let (q, s) = plan(Path::new("/r"), &files);
        assert_eq!(q.len(), 4);
        assert_eq!(q[0][..3], ["/usr/bin/xattr", "-d", "com.apple.quarantine"]);
        assert_eq!(s.len(), 2);
        assert_eq!(s[0].1[..4], ["/usr/bin/codesign", "--force", "--sign", "-"]);
        assert!(s[1].0[2].ends_with("libllama.0.dylib"));
    }

    #[test]
    fn a_no_op_elsewhere_and_tools_time_out() {
        let dir = tempfile::tempdir().unwrap();
        if !cfg!(target_os = "macos") {
            assert!(after_verified_unpack(dir.path()).is_ok());
        }
        assert!(run(&[], Duration::from_secs(1)).is_ok_and(|ok| !ok));
        assert!(run(&["/no/such/tool".to_string()], Duration::from_secs(1)).is_err());
    }
}
