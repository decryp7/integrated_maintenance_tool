//! Unpacking a verified FFmpeg archive (`.zip` on Windows, `.tar.xz` on Linux) into one flat folder, defensively:
//! only `ffmpeg`, `ffprobe`, shared libraries and licence files are written (not `ffplay`, headers or import
//! libraries), every name is checked (no absolute paths, no `..`, no hidden files), links may only point at a file of
//! the archive and become copies, and the number of entries and their sizes are capped.

use crate::error::{Result, VideoError};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, Clone, Copy)]
pub struct UnpackLimits {
    pub max_entries: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for UnpackLimits {
    fn default() -> Self {
        UnpackLimits {
            max_entries: 4096,
            max_file_bytes: 512 << 20,
            max_total_bytes: 2 << 30,
        }
    }
}

fn bad(m: impl Into<String>) -> VideoError {
    VideoError::Integrity(format!("the FFmpeg archive is not usable: {}", m.into()))
}

fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Whether the runtime needs the file.
pub fn is_expected(name: &str) -> bool {
    if !safe_name(name) {
        return false;
    }
    let program = matches!(name, "ffmpeg" | "ffmpeg.exe" | "ffprobe" | "ffprobe.exe");
    let lib = name.ends_with(".dll") || name.ends_with(".dylib") || name.ends_with(".so") || name.contains(".so.");
    let licence = name.starts_with("LICENSE") || name.starts_with("COPYING");
    program || lib || licence
}

/// The file name of an archive entry at most four folders deep (`ffmpeg-n9/bin/ffmpeg.exe`), or `None` for anything
/// unsafe.
fn base_name(path: &str) -> Option<&str> {
    let path = path.trim_start_matches("./");
    if path.starts_with('/') || path.contains('\\') || path.contains(':') {
        return None;
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    if parts.is_empty() || parts.len() > 5 || parts.iter().any(|p| *p == ".." || *p == ".") {
        return None;
    }
    parts.last().copied()
}

struct Writer<'a> {
    dest: &'a Path,
    limits: UnpackLimits,
    files: Vec<String>,
    bytes: u64,
    entries: usize,
    /// Link name → its target's file name (resolved once every file is written).
    links: BTreeMap<String, String>,
}

impl Writer<'_> {
    fn count(&mut self) -> Result<()> {
        self.entries += 1;
        if self.entries > self.limits.max_entries {
            return Err(bad("it has too many entries"));
        }
        Ok(())
    }

    fn file(&mut self, name: &str, r: &mut dyn Read) -> Result<()> {
        if self.files.iter().any(|f| f == name) {
            return Err(bad(format!("{name} appears twice")));
        }
        let path = self.dest.join(name);
        let mut f = std::fs::OpenOptions::new().write(true).create_new(true).open(&path)?;
        let n = std::io::copy(&mut r.take(self.limits.max_file_bytes + 1), &mut f).map_err(|e| bad(e.to_string()))?;
        f.flush()?;
        if n > self.limits.max_file_bytes {
            return Err(bad(format!("{name} is too large")));
        }
        self.bytes += n;
        if self.bytes > self.limits.max_total_bytes {
            return Err(bad("it expands to too much data"));
        }
        self.files.push(name.to_string());
        Ok(())
    }

    /// Links become copies of their (flat) target, which must be a written file; chains are followed a few steps.
    fn finish_links(&mut self) -> Result<()> {
        let links = std::mem::take(&mut self.links);
        for (name, target) in &links {
            if self.files.contains(name) {
                return Err(bad(format!("{name} appears twice")));
            }
            let mut t = target.clone();
            for _ in 0..8 {
                match links.get(&t) {
                    Some(next) => t = next.clone(),
                    None => break,
                }
            }
            if !self.files.contains(&t) {
                return Err(bad(format!("the link {name} points outside the runtime")));
            }
            let n = std::fs::copy(self.dest.join(&t), self.dest.join(name))?;
            self.bytes += n;
            if self.bytes > self.limits.max_total_bytes {
                return Err(bad("it expands to too much data"));
            }
            self.files.push(name.clone());
        }
        Ok(())
    }
}

#[cfg(unix)]
fn make_executable(dir: &Path, files: &[String]) -> Result<()> {
    use std::os::unix::fs::PermissionsExt;
    for f in files {
        std::fs::set_permissions(dir.join(f), std::fs::Permissions::from_mode(0o755))?;
    }
    Ok(())
}

#[cfg(not(unix))]
fn make_executable(_dir: &Path, _files: &[String]) -> Result<()> {
    Ok(())
}

fn unzip(archive: &Path, w: &mut Writer) -> Result<()> {
    let f = std::fs::File::open(archive)?;
    let mut zip = zip::ZipArchive::new(std::io::BufReader::new(f)).map_err(|e| bad(e.to_string()))?;
    for i in 0..zip.len() {
        w.count()?;
        let mut entry = zip.by_index(i).map_err(|e| bad(e.to_string()))?;
        if entry.is_dir() {
            continue;
        }
        let path = String::from_utf8_lossy(entry.name_raw()).into_owned();
        let Some(name) = base_name(&path) else {
            return Err(bad(format!("it contains the unsafe path {path}")));
        };
        if entry.is_symlink() {
            return Err(bad(format!("{path} is a link")));
        }
        if is_expected(name) {
            let name = name.to_string();
            w.file(&name, &mut entry)?;
        }
    }
    Ok(())
}

fn untar_xz(archive: &Path, dest: &Path, w: &mut Writer) -> Result<()> {
    // The tar is decompressed next to the staging folder first (xz is not seekable and lzma-rs writes, not reads).
    let tar_path = dest.with_extension("tar");
    let result = (|| -> Result<()> {
        {
            let mut input = std::io::BufReader::new(std::fs::File::open(archive)?);
            let out = std::fs::File::create(&tar_path)?;
            let mut capped = Capped {
                inner: std::io::BufWriter::new(out),
                left: w.limits.max_total_bytes.saturating_mul(2),
            };
            lzma_rs::xz_decompress(&mut input, &mut capped).map_err(|e| bad(format!("{e:?}")))?;
            capped.inner.flush()?;
        }
        let mut tar = tar::Archive::new(std::io::BufReader::new(std::fs::File::open(&tar_path)?));
        for entry in tar.entries().map_err(|e| bad(e.to_string()))? {
            w.count()?;
            let mut entry = entry.map_err(|e| bad(e.to_string()))?;
            let path = entry.path().map_err(|e| bad(e.to_string()))?.to_string_lossy().into_owned();
            let kind = entry.header().entry_type();
            if kind.is_dir() {
                continue;
            }
            let Some(name) = base_name(&path) else {
                return Err(bad(format!("it contains the unsafe path {path}")));
            };
            if !is_expected(name) {
                continue;
            }
            let name = name.to_string();
            if kind.is_symlink() || kind.is_hard_link() {
                let target = entry
                    .link_name()
                    .map_err(|e| bad(e.to_string()))?
                    .map(|t| t.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // Only a sibling (`libavcodec.so.62 -> libavcodec.so.62.3.100`) or the same folder by its archive path.
                let t = if kind.is_hard_link() {
                    base_name(&target)
                } else {
                    Some(target.as_str())
                };
                match t.filter(|t| safe_name(t) && !t.contains('/')) {
                    Some(t) => {
                        w.links.insert(name, t.to_string());
                    }
                    None => return Err(bad(format!("the link {name} points outside the runtime"))),
                }
            } else if kind.is_file() {
                w.file(&name, &mut entry)?;
            } else {
                return Err(bad(format!("{path} is not a plain file")));
            }
        }
        Ok(())
    })();
    let _ = std::fs::remove_file(&tar_path);
    result
}

/// A writer that refuses to grow past `left` bytes (an xz bomb).
struct Capped<W: Write> {
    inner: W,
    left: u64,
}

impl<W: Write> Write for Capped<W> {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        if buf.len() as u64 > self.left {
            return Err(std::io::Error::other("it expands to too much data"));
        }
        let n = self.inner.write(buf)?;
        self.left -= n as u64;
        Ok(n)
    }

    fn flush(&mut self) -> std::io::Result<()> {
        self.inner.flush()
    }
}

/// Unpacks `archive` into the (empty) folder `dest`; returns the files written. The archive must hold `ffmpeg` and
/// `ffprobe`.
pub fn unpack(archive: &Path, dest: &Path, limits: UnpackLimits) -> Result<Vec<String>> {
    std::fs::create_dir_all(dest)?;
    let mut w = Writer {
        dest,
        limits,
        files: Vec::new(),
        bytes: 0,
        entries: 0,
        links: BTreeMap::new(),
    };
    let name = archive.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    if name.ends_with(".zip") {
        unzip(archive, &mut w)?;
    } else if name.ends_with(".tar.xz") {
        untar_xz(archive, dest, &mut w)?;
    } else {
        return Err(bad("it is neither a .zip nor a .tar.xz"));
    }
    w.finish_links()?;
    for p in ["ffmpeg", "ffprobe"] {
        let exe = crate::run::exe(p);
        if !w.files.contains(&exe) {
            return Err(bad(format!("it does not contain {exe}")));
        }
    }
    make_executable(dest, &w.files)?;
    w.files.sort();
    Ok(w.files)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::run::exe;

    pub(crate) fn zip_of(path: &Path, entries: &[(&str, &[u8])]) {
        let f = std::fs::File::create(path).unwrap();
        let mut z = zip::ZipWriter::new(f);
        for (name, data) in entries {
            z.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            z.write_all(data).unwrap();
        }
        z.finish().unwrap();
    }

    fn tar_xz_of(path: &Path, entries: &[(&str, Option<&str>, &[u8])]) {
        let mut tar_bytes = Vec::new();
        {
            let mut b = tar::Builder::new(&mut tar_bytes);
            for (name, link, data) in entries {
                let mut h = tar::Header::new_gnu();
                match link {
                    Some(t) => {
                        h.set_entry_type(tar::EntryType::Symlink);
                        h.set_size(0);
                        b.append_link(&mut h, name, t).unwrap();
                    }
                    None => {
                        h.set_size(data.len() as u64);
                        h.set_mode(0o644);
                        h.set_cksum();
                        b.append_data(&mut h, name, *data).unwrap();
                    }
                }
            }
            b.finish().unwrap();
        }
        let mut out = std::fs::File::create(path).unwrap();
        lzma_rs::xz_compress(&mut std::io::Cursor::new(tar_bytes), &mut out).unwrap();
    }

    #[test]
    fn a_zip_build_keeps_only_what_the_runtime_needs() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("ffmpeg-win64-lgpl-shared.zip");
        let (ffmpeg, ffprobe) = (format!("b/bin/{}", exe("ffmpeg")), format!("b/bin/{}", exe("ffprobe")));
        zip_of(
            &a,
            &[
                (&ffmpeg, b"ff"),
                (&ffprobe, b"fp"),
                ("b/bin/ffplay.exe", b"no"),
                ("b/bin/avcodec-62.dll", b"dll"),
                ("b/lib/avcodec.lib", b"no"),
                ("b/include/libavcodec/avcodec.h", b"no"),
                ("b/LICENSE.txt", b"LGPL"),
                ("b/doc/", b""),
            ],
        );
        let out = dir.path().join("out");
        let files = unpack(&a, &out, UnpackLimits::default()).unwrap();
        let mut want = vec!["LICENSE.txt".to_string(), "avcodec-62.dll".into(), exe("ffmpeg"), exe("ffprobe")];
        want.sort();
        assert_eq!(files, want);
        assert_eq!(std::fs::read(out.join(exe("ffmpeg"))).unwrap(), b"ff");
    }

    #[test]
    fn unsafe_or_incomplete_zips_are_refused() {
        let dir = tempfile::tempdir().unwrap();
        let ff = exe("ffmpeg");
        type Case<'a> = (&'a str, Vec<(String, &'a [u8])>, &'a str);
        let cases: Vec<Case> = vec![
            ("slip.zip", vec![("../evil.dll".into(), b"x")], "unsafe path"),
            ("deep.zip", vec![("a/b/c/d/e/f/x.dll".into(), b"x")], "unsafe path"),
            ("twice.zip", vec![(format!("a/{ff}"), b"x"), (format!("b/{ff}"), b"y")], "twice"),
            ("noprobe.zip", vec![(format!("a/{ff}"), b"x")], "does not contain"),
        ];
        for (name, entries, why) in cases {
            let a = dir.path().join(name);
            let refs: Vec<(&str, &[u8])> = entries.iter().map(|(n, d)| (n.as_str(), *d)).collect();
            zip_of(&a, &refs);
            let e = unpack(&a, &dir.path().join(format!("o-{name}")), UnpackLimits::default()).unwrap_err();
            assert!(e.to_string().contains(why), "{name}: {e}");
        }
        let big = dir.path().join("big.zip");
        let probe = format!("a/{}", exe("ffprobe"));
        zip_of(&big, &[(&format!("a/{ff}"), &[0u8; 2048]), (&probe, b"p")]);
        let tight = UnpackLimits {
            max_file_bytes: 1024,
            ..UnpackLimits::default()
        };
        assert!(unpack(&big, &dir.path().join("o-big"), tight)
            .unwrap_err()
            .to_string()
            .contains("too large"));
        let few = UnpackLimits {
            max_entries: 1,
            ..UnpackLimits::default()
        };
        assert!(unpack(&big, &dir.path().join("o-few"), few)
            .unwrap_err()
            .to_string()
            .contains("too many"));
        assert!(unpack(&dir.path().join("x.rar"), &dir.path().join("o-rar"), UnpackLimits::default()).is_err());
        std::fs::write(dir.path().join("junk.zip"), b"not a zip").unwrap();
        assert!(unpack(&dir.path().join("junk.zip"), &dir.path().join("o-junk"), UnpackLimits::default()).is_err());
    }

    #[test]
    fn a_tar_xz_build_turns_sibling_links_into_copies() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("ffmpeg-linux64-lgpl-shared.tar.xz");
        let (ff, fp) = (format!("b/bin/{}", exe("ffmpeg")), format!("b/bin/{}", exe("ffprobe")));
        tar_xz_of(
            &a,
            &[
                (&ff, None, b"ff"),
                (&fp, None, b"fp"),
                ("b/lib/libavcodec.so.62.3.100", None, b"so"),
                ("b/lib/libavcodec.so.62", Some("libavcodec.so.62.3.100"), b""),
                ("b/lib/libavcodec.so", Some("libavcodec.so.62"), b""),
                ("b/lib/pkgconfig/libavcodec.pc", None, b"no"),
            ],
        );
        let out = dir.path().join("out");
        let files = unpack(&a, &out, UnpackLimits::default()).unwrap();
        assert!(files.contains(&"libavcodec.so".to_string()) && files.contains(&"libavcodec.so.62".to_string()));
        assert_eq!(std::fs::read(out.join("libavcodec.so")).unwrap(), b"so");
        assert!(!out.join("libavcodec.pc").exists());
        assert!(!dir.path().join("out.tar").exists(), "the decompressed tar is removed");

        let escape = dir.path().join("escape.tar.xz");
        tar_xz_of(
            &escape,
            &[(&ff, None, b"ff"), (&fp, None, b"fp"), ("b/lib/libx.so", Some("/etc/passwd"), b"")],
        );
        let e = unpack(&escape, &dir.path().join("o-escape"), UnpackLimits::default()).unwrap_err();
        assert!(e.to_string().contains("outside"), "{e}");
        let dangling = dir.path().join("dangling.tar.xz");
        tar_xz_of(
            &dangling,
            &[
                (&ff, None, b"ff"),
                (&fp, None, b"fp"),
                ("b/lib/libx.so", Some("libmissing.so"), b""),
            ],
        );
        let e = unpack(&dangling, &dir.path().join("o-dangling"), UnpackLimits::default()).unwrap_err();
        assert!(e.to_string().contains("outside"), "{e}");
        std::fs::write(dir.path().join("junk.tar.xz"), b"not xz").unwrap();
        assert!(unpack(&dir.path().join("junk.tar.xz"), &dir.path().join("o-junk"), UnpackLimits::default()).is_err());
    }

    #[test]
    fn expected_names() {
        for ok in [
            "ffmpeg",
            "ffprobe.exe",
            "avutil-60.dll",
            "libavutil.so.60",
            "libx.dylib",
            "LICENSE.txt",
            "COPYING.LGPLv2.1",
        ] {
            assert!(is_expected(ok), "{ok}");
        }
        for no in ["ffplay", "ffplay.exe", "avutil.lib", ".hidden.dll", "a..b.dll", "x.h"] {
            assert!(!is_expected(no), "{no}");
        }
        assert_eq!(base_name("./a/b/c.dll"), Some("c.dll"));
        assert_eq!(base_name("C:/x.dll"), None);
        assert_eq!(base_name("a/./b.dll"), None);
    }
}
