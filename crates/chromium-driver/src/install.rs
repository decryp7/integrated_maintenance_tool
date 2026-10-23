//! Installing a pinned Chromium archive: SHA-256 check, safe extraction, locating the executable and a self-test.
//! Downloading is the caller's job (it owns the network stack and the progress UI).

use crate::discover::{Browser, Flavor, Os};
use crate::{Error, Result};
use sha2::{Digest, Sha256};
use std::io::Read;
use std::path::{Path, PathBuf};

/// Largest total size an archive may expand to.
pub const MAX_EXPANDED: u64 = 3 << 30;

fn err(m: impl Into<String>) -> Error {
    Error::Install(m.into())
}

/// Lowercase hex SHA-256 of a file.
pub fn sha256_file(path: &Path) -> Result<String> {
    let mut f = std::fs::File::open(path).map_err(|e| err(format!("the download could not be read: {e}")))?;
    let mut h = Sha256::new();
    let mut buf = [0u8; 1 << 16];
    loop {
        let n = f.read(&mut buf).map_err(|e| err(e.to_string()))?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Ok(h.finalize().iter().map(|b| format!("{b:02x}")).collect())
}

/// A writer that hashes what passes through it, so a download is verified without reading the file again.
pub struct HashingWriter<W: std::io::Write> {
    inner: W,
    hash: Sha256,
}

impl<W: std::io::Write> HashingWriter<W> {
    pub fn new(inner: W) -> Self {
        HashingWriter {
            inner,
            hash: Sha256::new(),
        }
    }

    /// The writer and the lowercase hex SHA-256 of everything written.
    pub fn finish(self) -> (W, String) {
        (self.inner, self.hash.finalize().iter().map(|b| format!("{b:02x}")).collect())
    }
}

impl<W: std::io::Write> std::io::Write for HashingWriter<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let n = self.inner.write(buf)?;
        self.hash.update(&buf[..n]);
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Reports `(done, total)` bytes of a stage; returning false cancels it.
pub type Progress<'a> = &'a mut dyn FnMut(u64, u64) -> bool;

/// Verifies the archive against `expected` (hex, any case) and extracts it into `dest`; paths that would leave
/// `dest` and archives that expand beyond [`MAX_EXPANDED`] are refused.
pub fn extract_verified(archive: &Path, expected_sha256: &str, dest: &Path) -> Result<()> {
    let got = sha256_file(archive)?;
    extract_checked(archive, &got, expected_sha256, dest, &mut |_, _| true)
}

/// [`extract_verified`] with the archive's SHA-256 already known (hashed while it was written: no second read) and
/// byte progress of the unpacking; `progress` returning false stops it with an error (the caller removes `dest`).
pub fn extract_checked(archive: &Path, got_sha256: &str, expected_sha256: &str, dest: &Path, progress: Progress) -> Result<()> {
    if !got_sha256.eq_ignore_ascii_case(expected_sha256.trim()) {
        return Err(err("the download does not match its pinned checksum and was discarded"));
    }
    let mut zip = zip::ZipArchive::new(std::fs::File::open(archive).map_err(|e| err(e.to_string()))?)
        .map_err(|e| err(format!("the download is not a valid archive: {e}")))?;
    std::fs::create_dir_all(dest).map_err(|e| err(e.to_string()))?;
    let mut total = 0u64;
    let expanded: u64 = (0..zip.len()).filter_map(|i| zip.by_index_raw(i).ok().map(|f| f.size())).sum();
    let mut done = 0u64;
    for i in 0..zip.len() {
        let mut f = zip.by_index(i).map_err(|e| err(e.to_string()))?;
        let rel = f.enclosed_name().ok_or_else(|| err("the archive contains an unsafe path"))?;
        total += f.size();
        if total > MAX_EXPANDED {
            return Err(err("the archive expands to more than the allowed size"));
        }
        let out = dest.join(rel);
        if f.is_dir() {
            std::fs::create_dir_all(&out).map_err(|e| err(e.to_string()))?;
            continue;
        }
        if let Some(p) = out.parent() {
            std::fs::create_dir_all(p).map_err(|e| err(e.to_string()))?;
        }
        let mut o = std::fs::File::create(&out).map_err(|e| err(e.to_string()))?;
        let mut buf = vec![0u8; 1 << 16];
        loop {
            let n = f.read(&mut buf).map_err(|e| err(e.to_string()))?;
            if n == 0 {
                break;
            }
            std::io::Write::write_all(&mut o, &buf[..n]).map_err(|e| err(e.to_string()))?;
            done += n as u64;
            if !progress(done, expanded) {
                return Err(err("the installation was cancelled"));
            }
        }
        #[cfg(unix)]
        if let Some(mode) = f.unix_mode() {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
        }
    }
    Ok(())
}

/// The executable inside an extracted tree.
pub fn locate_executable(root: &Path, os: Os) -> Option<Browser> {
    let want: &[&str] = match os {
        Os::Windows => &["chrome.exe"],
        Os::Mac => &["Google Chrome for Testing", "Chromium"],
        Os::Linux => &["chrome"],
    };
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        for e in std::fs::read_dir(&d).ok()?.flatten() {
            let p: PathBuf = e.path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().and_then(|n| n.to_str()).is_some_and(|n| want.contains(&n)) && !p.to_string_lossy().contains("Helper") {
                return Some(Browser {
                    flavor: Flavor::Chromium,
                    path: p,
                });
            }
        }
    }
    None
}

/// Runs `<exe> --version` with a timeout and returns its output; the self-test of an installed browser.
pub fn self_test(exe: &Path) -> Result<String> {
    let mut child = std::process::Command::new(exe)
        .arg("--version")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| err(format!("the browser does not run: {e}")))?;
    let end = std::time::Instant::now() + std::time::Duration::from_secs(15);
    loop {
        match child.try_wait() {
            Ok(Some(s)) if s.success() => break,
            Ok(Some(_)) => return Err(err("the browser exited with an error")),
            Ok(None) if std::time::Instant::now() < end => std::thread::sleep(std::time::Duration::from_millis(50)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(err("the browser did not answer its self-test"));
            }
        }
    }
    let mut s = String::new();
    if let Some(mut o) = child.stdout.take() {
        let _ = o.read_to_string(&mut s);
    }
    Ok(s.trim().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn zip_with(path: &Path, name: &str, body: &[u8]) {
        let mut z = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        z.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        z.write_all(body).unwrap();
        z.finish().unwrap();
    }

    #[test]
    fn checksum_mismatch_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        zip_with(&a, "chrome-linux/chrome", b"x");
        let e = extract_verified(&a, "00", &d.path().join("out")).unwrap_err();
        assert!(e.to_string().contains("checksum"));
        assert!(!d.path().join("out").exists());
    }

    #[test]
    fn verified_archive_extracts_and_is_located() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        zip_with(&a, "chrome-linux/chrome", b"x");
        let sum = sha256_file(&a).unwrap().to_uppercase();
        extract_verified(&a, &sum, &d.path().join("out")).unwrap();
        let b = locate_executable(&d.path().join("out"), Os::Linux).unwrap();
        assert!(b.path.ends_with("chrome-linux/chrome"));
    }

    #[test]
    fn hashing_while_writing_equals_the_file_hash_and_needs_no_second_read() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        let mut w = HashingWriter::new(std::fs::File::create(&a).unwrap());
        for chunk in [&b"hello "[..], &b"world"[..]] {
            std::io::Write::write_all(&mut w, chunk).unwrap();
        }
        let (_, streamed) = w.finish();
        assert_eq!(streamed, sha256_file(&a).unwrap());
    }

    #[test]
    fn unpacking_reports_bytes_and_stops_when_cancelled() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        zip_with(&a, "chrome-linux/chrome", &vec![7u8; 200_000]);
        let sum = sha256_file(&a).unwrap();
        let mut seen = Vec::new();
        extract_checked(&a, &sum, &sum, &d.path().join("ok"), &mut |done, total| {
            seen.push((done, total));
            true
        })
        .unwrap();
        assert!(seen.len() > 1 && seen.windows(2).all(|w| w[0].0 < w[1].0), "{seen:?}");
        assert_eq!(seen.last().unwrap().0, 200_000);
        assert_eq!(seen.last().unwrap().1, 200_000);
        let e = extract_checked(&a, &sum, &sum, &d.path().join("no"), &mut |_, _| false).unwrap_err();
        assert!(e.to_string().contains("cancelled"));
    }

    #[test]
    fn zip_slip_is_refused() {
        let d = tempfile::tempdir().unwrap();
        let a = d.path().join("a.zip");
        zip_with(&a, "../evil", b"x");
        let sum = sha256_file(&a).unwrap();
        assert!(extract_verified(&a, &sum, &d.path().join("out")).is_err());
        assert!(!d.path().join("evil").exists());
    }

    #[test]
    fn self_test_fails_for_a_missing_program() {
        assert!(self_test(Path::new("/nonexistent/x")).is_err());
    }
}
