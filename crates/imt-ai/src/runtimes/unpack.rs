//! Unpacking a verified runtime archive (`.zip` on Windows, `.tar.gz` on Linux and macOS) into one flat
//! folder, defensively: only the files llama-server needs (the server, shared libraries, licences) are
//! written, every name is checked (no paths, no `..`, no hidden files: zip-slip cannot happen), symbolic
//! links may only point at a sibling, and the number of files and their sizes are capped.

use crate::error::{AiError, Result};
use std::io::{Read, Write};
use std::path::Path;

#[derive(Debug, Clone)]
pub struct UnpackLimits {
    pub max_entries: usize,
    pub max_file_bytes: u64,
    pub max_total_bytes: u64,
}

impl Default for UnpackLimits {
    fn default() -> Self {
        UnpackLimits {
            max_entries: 4096,
            max_file_bytes: 4 * 1024 * 1024 * 1024,
            max_total_bytes: 8 * 1024 * 1024 * 1024,
        }
    }
}

/// What was unpacked.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Unpacked {
    pub files: Vec<String>,
    pub bytes: u64,
}

fn bad(m: impl Into<String>) -> AiError {
    AiError::Integrity(format!("the runtime archive is not usable: {}", m.into()))
}

/// A plain file name that is safe to create in the target folder.
fn safe_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('.')
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '+'))
}

/// Whether llama-server needs the file: the server itself, shared libraries (but not the libraries of
/// the other llama.cpp tools), licences.
pub fn is_expected(name: &str) -> bool {
    if !safe_name(name) {
        return false;
    }
    if name == "llama-server" || name == "llama-server.exe" || name.starts_with("LICENSE") {
        return true;
    }
    let lib = name.ends_with(".dll") || name.ends_with(".dylib") || name.ends_with(".so") || name.contains(".so.");
    let other_tool = name.contains("-impl.") && !name.trim_start_matches("lib").starts_with("llama-server-impl.");
    lib && !other_tool
}

/// The file name of an archive entry with at most one leading folder (`llama-b11669/libggml.so`), or
/// `None` for anything deeper or unsafe.
fn flat_name(path: &str) -> Option<&str> {
    let path = path.trim_start_matches("./");
    if path.starts_with('/') || path.starts_with('\\') || path.contains('\\') || path.contains(':') {
        return None;
    }
    let parts: Vec<&str> = path.split('/').filter(|p| !p.is_empty()).collect();
    match parts.as_slice() {
        [name] => Some(name),
        [dir, name] if safe_name(dir) => Some(name),
        _ => None,
    }
}

struct Writer<'a> {
    dest: &'a Path,
    limits: &'a UnpackLimits,
    out: Unpacked,
    links: Vec<(String, String)>,
    entries: usize,
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
        if self.out.files.iter().any(|f| f == name) {
            return Err(bad(format!("{name} appears twice")));
        }
        let path = self.dest.join(name);
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|e| AiError::Io(format!("cannot write {}: {e}", path.display())))?;
        let n = std::io::copy(&mut r.take(self.limits.max_file_bytes + 1), &mut f).map_err(|e| bad(e.to_string()))?;
        f.flush()?;
        if n > self.limits.max_file_bytes {
            return Err(bad(format!("{name} is too large")));
        }
        self.out.bytes += n;
        if self.out.bytes > self.limits.max_total_bytes {
            return Err(bad("it expands to too much data"));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))?;
        }
        self.out.files.push(name.to_string());
        Ok(())
    }

    /// Symbolic links become copies of their (sibling) target once everything is written.
    fn finish_links(&mut self) -> Result<()> {
        // A link may point at another link: resolve in rounds.
        for _ in 0..8 {
            let pending = std::mem::take(&mut self.links);
            for (name, target) in pending {
                if self.out.files.contains(&target) {
                    if self.out.files.contains(&name) {
                        return Err(bad(format!("{name} appears twice")));
                    }
                    let n = std::fs::copy(self.dest.join(&target), self.dest.join(&name))?;
                    self.out.bytes += n;
                    if self.out.bytes > self.limits.max_total_bytes {
                        return Err(bad("it expands to too much data"));
                    }
                    self.out.files.push(name);
                } else {
                    self.links.push((name, target));
                }
            }
            if self.links.is_empty() {
                return Ok(());
            }
        }
        // Links to files that were not unpacked (other tools) are simply dropped.
        Ok(())
    }
}

/// Unpacks `archive` into the existing, empty folder `dest`.
pub fn unpack(archive: &Path, dest: &Path, limits: &UnpackLimits) -> Result<Unpacked> {
    let name = archive.file_name().and_then(|n| n.to_str()).unwrap_or_default();
    let mut w = Writer {
        dest,
        limits,
        out: Unpacked::default(),
        links: vec![],
        entries: 0,
    };
    let file = std::fs::File::open(archive)?;
    if name.ends_with(".zip") {
        let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(|e| bad(e.to_string()))?;
        for i in 0..zip.len() {
            w.count()?;
            let mut entry = zip.by_index(i).map_err(|e| bad(e.to_string()))?;
            if entry.is_dir() || entry.is_symlink() {
                continue;
            }
            let raw = entry.name().map_err(|e| bad(e.to_string()))?.into_owned();
            let Some(n) = flat_name(&raw).filter(|n| is_expected(n)).map(str::to_string) else {
                continue;
            };
            w.file(&n, &mut entry)?;
        }
    } else if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        let mut tar = tar::Archive::new(flate2::read::GzDecoder::new(std::io::BufReader::new(file)));
        for entry in tar.entries().map_err(|e| bad(e.to_string()))? {
            w.count()?;
            let mut entry = entry.map_err(|e| bad(e.to_string()))?;
            let path = entry.path().map_err(|e| bad(e.to_string()))?.to_string_lossy().into_owned();
            let Some(n) = flat_name(&path).filter(|n| is_expected(n)).map(str::to_string) else {
                continue;
            };
            let kind = entry.header().entry_type();
            if kind.is_file() {
                w.file(&n, &mut entry)?;
            } else if kind.is_symlink() || kind.is_hard_link() {
                let target = entry
                    .link_name()
                    .map_err(|e| bad(e.to_string()))?
                    .map(|t| t.to_string_lossy().into_owned())
                    .unwrap_or_default();
                // Hard links name the archive path of their target; symbolic links must name a sibling.
                let t = match kind.is_hard_link() {
                    true => flat_name(&target),
                    false => (!target.contains(['/', '\\'])).then_some(target.as_str()),
                };
                match t.filter(|t| safe_name(t)) {
                    Some(t) => w.links.push((n, t.to_string())),
                    None => return Err(bad(format!("{n} links outside its folder"))),
                }
            }
        }
    } else {
        return Err(bad(format!("{name} is not a .zip or .tar.gz file")));
    }
    w.finish_links()?;
    Ok(w.out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expected_files() {
        for ok in [
            "llama-server.exe",
            "llama-server",
            "llama-server-impl.dll",
            "libllama-server-impl.so",
            "ggml-vulkan.dll",
            "libggml.so.0.27.0",
            "libggml-metal.0.dylib",
            "cudart64_13.dll",
            "LICENSE-LLVM-OpenMP",
        ] {
            assert!(is_expected(ok), "{ok}");
        }
        for skip in [
            "llama-cli.exe",
            "llama-cli-impl.dll",
            "libllama-bench-impl.so",
            "llama-bench",
            "README.md",
            ".hidden.dll",
            "a..dll",
            "x y.dll",
        ] {
            assert!(!is_expected(skip), "{skip}");
        }
    }

    #[test]
    fn names_are_flattened_safely() {
        assert_eq!(flat_name("llama-b1/libggml.so"), Some("libggml.so"));
        assert_eq!(flat_name("./ggml.dll"), Some("ggml.dll"));
        assert_eq!(flat_name("a/b/c.dll"), None);
        assert_eq!(flat_name("/etc/passwd"), None);
        assert_eq!(flat_name("..\\x.dll"), None);
        assert_eq!(flat_name("C:x.dll"), None);
        assert_eq!(flat_name("../x.dll"), None, "the folder must itself be safe");
    }
}
