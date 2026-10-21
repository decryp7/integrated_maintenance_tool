//! Reading, verifying and writing `.imtplugin` files.
//!
//! A package is a zip archive with exactly these entries (directories `bin/` and `bin/<target>/` may also appear):
//!
//! ```text
//! manifest.toml                         the manifest (crate::manifest)
//! bin/<target>/<binary>[.exe]           one program per target the manifest lists
//! signature                             optional: key, public key and Ed25519 signature of the digest
//! ```
//!
//! Reading is bounded and never trusts the archive: the file size, the number of entries, each entry's size
//! (counted while reading, not taken from the header) and the total are capped; names must be exactly the listed
//! ones (no traversal, links, encrypted or duplicate entries), and each entry is hashed as it streams. The
//! **digest** is the SHA-256 of a canonical text listing every signed file with its size and SHA-256
//! ([`Package::digest`]); the signature covers that digest, so changing any byte of the manifest or of any program
//! breaks it.

use crate::keys::{from_hex, to_hex, PublicKey, SecretKey};
use crate::manifest::PackageManifest;
use crate::PackageError;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Seek, Write};
use std::path::Path;

/// Largest package file (256 MiB).
pub const MAX_PACKAGE_BYTES: u64 = 256 << 20;
/// Largest program in a package (128 MiB).
pub const MAX_BINARY_BYTES: u64 = 128 << 20;
/// Largest total of all entries, uncompressed (512 MiB).
pub const MAX_TOTAL_BYTES: u64 = 512 << 20;
/// Largest `signature` file.
pub const MAX_SIGNATURE_BYTES: u64 = 4096;
/// Most entries (files and directories).
pub const MAX_ENTRIES: usize = 64;
/// The manifest's entry name.
pub const MANIFEST: &str = "manifest.toml";
/// The signature's entry name.
pub const SIGNATURE: &str = "signature";
/// First line of the canonical digest text; its version changes with the digest's definition.
const DIGEST_HEADER: &str = "imt-plugin-package-v1\n";

/// The `signature` file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignatureFile {
    /// Format of this file (1).
    pub format: u32,
    /// The signing key's id (must equal the manifest's `plugin.key`).
    pub key: String,
    /// The signing public key, text form ([`PublicKey::to_text`]).
    pub public_key: String,
    /// The Ed25519 signature of the digest, hex.
    pub signature: String,
}

/// One signed file of a package.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    /// Its name in the archive.
    pub path: String,
    /// Its size in bytes.
    pub size: u64,
    /// Its SHA-256.
    pub sha256: [u8; 32],
}

/// Whether and by whom a package is signed (its signature is valid; a broken one is [`PackageError::Tampered`]).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signature {
    /// No signature.
    Unsigned,
    /// A valid signature by `key`.
    Signed {
        /// The signing key.
        key: PublicKey,
    },
}

/// A read and verified package (the programs stay in the file; [`Package::extract`] copies one out).
#[derive(Debug, Clone)]
pub struct Package {
    /// The manifest.
    pub manifest: PackageManifest,
    /// The signed files, by name (the manifest and each program).
    pub entries: BTreeMap<String, Entry>,
    /// The signature, verified against [`Package::digest`].
    pub signature: Signature,
    digest: [u8; 32],
}

fn not_a_package(e: impl std::fmt::Display) -> PackageError {
    PackageError::NotAPackage(e.to_string())
}

/// Reads `r` to its end through SHA-256, refusing more than `limit` bytes; `sink` receives the bytes.
fn hash_bounded(
    mut r: impl Read,
    limit: u64,
    what: &str,
    mut sink: impl FnMut(&[u8]) -> std::io::Result<()>,
) -> Result<(u64, [u8; 32]), PackageError> {
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf).map_err(|e| not_a_package(format!("{what}: {e}")))?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > limit {
            return Err(PackageError::TooLarge { what: what.into(), limit });
        }
        hasher.update(&buf[..n]);
        sink(&buf[..n])?;
    }
    Ok((total, hasher.finalize().into()))
}

/// The canonical digest of `entries` (see the module docs).
pub fn digest_of<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> [u8; 32] {
    let mut sorted: Vec<&Entry> = entries.into_iter().collect();
    sorted.sort_by(|a, b| a.path.cmp(&b.path));
    let mut text = String::from(DIGEST_HEADER);
    for e in sorted {
        text.push_str(&format!("{} {} {}\n", e.path, e.size, to_hex(&e.sha256)));
    }
    Sha256::digest(text.as_bytes()).into()
}

/// Whether `name` is a directory entry the layout allows (`bin/`, `bin/<target>/`).
fn allowed_dir(name: &str, m: &PackageManifest) -> bool {
    name == "bin/" || m.targets.iter().any(|t| name == format!("bin/{t}/"))
}

impl Package {
    /// Reads and verifies the package at `path`: layout, limits, manifest and signature (a signature that does
    /// not match is [`PackageError::Tampered`]). Whether the signer is trusted is [`crate::trust`]'s question.
    pub fn open(path: &Path) -> Result<Package, PackageError> {
        let file = std::fs::File::open(path).map_err(|e| PackageError::Io(format!("cannot open {}: {e}", path.display())))?;
        let len = file.metadata()?.len();
        if len > MAX_PACKAGE_BYTES {
            return Err(PackageError::TooLarge {
                what: "the package".into(),
                limit: MAX_PACKAGE_BYTES,
            });
        }
        Package::read(std::io::BufReader::new(file))
    }

    /// [`Package::open`] for a package in memory or any seekable reader.
    pub fn read(r: impl Read + Seek) -> Result<Package, PackageError> {
        let mut zip = zip::ZipArchive::new(r).map_err(not_a_package)?;
        if zip.len() > MAX_ENTRIES {
            return Err(PackageError::TooLarge {
                what: format!("the package's list of {} entries", zip.len()),
                limit: MAX_ENTRIES as u64,
            });
        }
        // The manifest first: it says which other entries may exist.
        let manifest_text = {
            let mut e = zip.by_name(MANIFEST).map_err(|_| PackageError::MissingFile(MANIFEST.into()))?;
            if e.encrypted() {
                return Err(PackageError::UnsafeEntry(MANIFEST.into()));
            }
            let mut bytes = Vec::new();
            hash_bounded(&mut e, crate::manifest::MAX_MANIFEST_BYTES, MANIFEST, |b| {
                bytes.extend_from_slice(b);
                Ok(())
            })?;
            String::from_utf8(bytes).map_err(|_| PackageError::InvalidManifest("not UTF-8 text".into()))?
        };
        let manifest = PackageManifest::parse(&manifest_text)?;
        let programs: Vec<String> = manifest.targets.iter().map(|t| manifest.binary_path(t)).collect();
        let mut entries: BTreeMap<String, Entry> = BTreeMap::new();
        let mut signature_text: Option<String> = None;
        let mut total = 0u64;
        let mut seen = std::collections::HashSet::new();
        for i in 0..zip.len() {
            let mut e = zip.by_index(i).map_err(not_a_package)?;
            let name = e.name().map_err(not_a_package)?.into_owned();
            if !seen.insert(name.clone()) {
                return Err(PackageError::UnsafeEntry(name));
            }
            if e.is_dir() {
                if allowed_dir(&name, &manifest) {
                    continue;
                }
                return Err(PackageError::UnlistedFile(name));
            }
            if e.encrypted() || e.is_symlink() {
                return Err(PackageError::UnsafeEntry(name));
            }
            let limit = if name == MANIFEST {
                crate::manifest::MAX_MANIFEST_BYTES
            } else if name == SIGNATURE {
                MAX_SIGNATURE_BYTES
            } else if programs.contains(&name) {
                MAX_BINARY_BYTES
            } else {
                return Err(
                    if name.contains("..") || name.starts_with('/') || name.contains('\\') || name.contains(':') {
                        PackageError::UnsafeEntry(name)
                    } else {
                        PackageError::UnlistedFile(name)
                    },
                );
            };
            let mut text = Vec::new();
            let keep = name == SIGNATURE;
            let (size, sha256) = hash_bounded(&mut e, limit.min(MAX_TOTAL_BYTES - total), &name, |b| {
                if keep {
                    text.extend_from_slice(b);
                }
                Ok(())
            })?;
            total += size;
            if keep {
                signature_text = Some(String::from_utf8(text).map_err(|_| PackageError::Tampered("the signature is not text".into()))?);
            } else {
                entries.insert(name.clone(), Entry { path: name, size, sha256 });
            }
        }
        for p in &programs {
            if !entries.contains_key(p) {
                return Err(PackageError::MissingFile(p.clone()));
            }
        }
        let digest = digest_of(entries.values());
        let signature = verify(&manifest, signature_text.as_deref(), &digest)?;
        Ok(Package {
            manifest,
            entries,
            signature,
            digest,
        })
    }

    /// The canonical digest the signature covers, hex (also what the user approved: install compares it again).
    pub fn digest(&self) -> String {
        to_hex(&self.digest)
    }

    /// The program for `target`, when the package has one.
    pub fn program(&self, target: &str) -> Option<&Entry> {
        self.entries.get(&self.manifest.binary_path(target))
    }

    /// Copies the program for `target` out of the package at `path` into `dest`, checking that it is still the
    /// program [`Package::open`] verified (the file may have changed since). Returns the program's SHA-256.
    pub fn extract(&self, path: &Path, target: &str, dest: &Path) -> Result<[u8; 32], PackageError> {
        let Some(expected) = self.program(target).cloned() else {
            return Err(PackageError::WrongTarget {
                target: target.into(),
                supported: self.manifest.targets.join(", "),
            });
        };
        let file = std::fs::File::open(path).map_err(|e| PackageError::Io(format!("cannot open {}: {e}", path.display())))?;
        let mut zip = zip::ZipArchive::new(std::io::BufReader::new(file)).map_err(not_a_package)?;
        let entry = zip
            .by_name(&expected.path)
            .map_err(|_| PackageError::MissingFile(expected.path.clone()))?;
        let mut out = std::fs::File::create(dest).map_err(|e| PackageError::Io(format!("cannot write {}: {e}", dest.display())))?;
        let (size, sha256) = hash_bounded(entry, MAX_BINARY_BYTES, &expected.path, |b| out.write_all(b))?;
        out.sync_all()?;
        drop(out);
        if size != expected.size || sha256 != expected.sha256 {
            let _ = std::fs::remove_file(dest);
            return Err(PackageError::Tampered(format!(
                "{} changed while it was being installed",
                expected.path
            )));
        }
        Ok(sha256)
    }
}

/// Checks the signature file against the manifest and the digest.
fn verify(manifest: &PackageManifest, signature: Option<&str>, digest: &[u8; 32]) -> Result<Signature, PackageError> {
    let tampered = |m: &str| PackageError::Tampered(m.to_string());
    let (text, key_id) = match (signature, &manifest.key) {
        (None, None) => return Ok(Signature::Unsigned),
        (None, Some(_)) => return Err(tampered("the manifest names a signing key but the package has no signature")),
        (Some(_), None) => return Err(tampered("the package has a signature but its manifest names no signing key")),
        (Some(t), Some(k)) => (t, k),
    };
    let file: SignatureFile = toml::from_str(text).map_err(|_| tampered("the signature file is not valid"))?;
    if file.format != 1 {
        return Err(tampered("the signature file has an unknown format"));
    }
    let key = PublicKey::parse(&file.public_key).map_err(|_| tampered("the signature's public key is not valid"))?;
    if key.id() != file.key || &file.key != key_id {
        return Err(tampered("the signature's key is not the key the manifest names"));
    }
    let sig = from_hex::<64>(&file.signature).ok_or_else(|| tampered("the signature is not valid"))?;
    if !key.verifies(digest, &sig) {
        return Err(tampered("the signature does not match the contents"));
    }
    Ok(Signature::Signed { key })
}

/// What [`build`] packs.
#[derive(Debug, Clone)]
pub struct Build<'a> {
    /// The manifest; its `key` is set from `sign` (or cleared without it).
    pub manifest: PackageManifest,
    /// The program for each target the manifest lists.
    pub programs: Vec<(String, &'a Path)>,
    /// Sign with this key.
    pub sign: Option<&'a SecretKey>,
}

/// Writes a package to `out`: the manifest, one program per target and, with a key, the signature. The archive is
/// deterministic (fixed times, sorted entries). Returns the package's digest (hex).
pub fn build(b: Build, out: &Path) -> Result<String, PackageError> {
    let mut manifest = b.manifest;
    manifest.key = b.sign.map(|k| k.public().id());
    let text = manifest.to_toml();
    // The result must read back: validate what is written.
    let manifest = PackageManifest::parse(&text)?;
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    files.insert(MANIFEST.into(), text.into_bytes());
    for t in &manifest.targets {
        let Some((_, path)) = b.programs.iter().find(|(target, _)| target == t) else {
            return Err(PackageError::MissingFile(format!("the program for {t}")));
        };
        let len = std::fs::metadata(path)
            .map_err(|e| PackageError::Io(format!("{}: {e}", path.display())))?
            .len();
        if len > MAX_BINARY_BYTES {
            return Err(PackageError::TooLarge {
                what: path.display().to_string(),
                limit: MAX_BINARY_BYTES,
            });
        }
        let bytes = std::fs::read(path).map_err(|e| PackageError::Io(format!("{}: {e}", path.display())))?;
        files.insert(manifest.binary_path(t), bytes);
    }
    if let Some((t, _)) = b.programs.iter().find(|(t, _)| !manifest.targets.contains(t)) {
        return Err(PackageError::UnlistedFile(format!(
            "a program for {t}, which the manifest does not list"
        )));
    }
    let entries: Vec<Entry> = files
        .iter()
        .map(|(path, bytes)| Entry {
            path: path.clone(),
            size: bytes.len() as u64,
            sha256: Sha256::digest(bytes).into(),
        })
        .collect();
    let digest = digest_of(&entries);
    if let Some(key) = b.sign {
        let sig = SignatureFile {
            format: 1,
            key: key.public().id(),
            public_key: key.public().to_text(),
            signature: to_hex(&key.sign(&digest)),
        };
        files.insert(SIGNATURE.into(), toml::to_string(&sig).unwrap_or_default().into_bytes());
    }
    let tmp = out.with_extension("imtplugin.part");
    let written = (|| -> Result<(), PackageError> {
        let file = std::fs::File::create(&tmp).map_err(|e| PackageError::Io(format!("cannot write {}: {e}", tmp.display())))?;
        let mut zip = zip::ZipWriter::new(file);
        let opts = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default())
            .unix_permissions(0o755);
        for (name, bytes) in &files {
            zip.start_file(name.as_str(), opts).map_err(|e| PackageError::Io(e.to_string()))?;
            zip.write_all(bytes)?;
        }
        zip.finish().map_err(|e| PackageError::Io(e.to_string()))?.sync_all()?;
        Ok(())
    })();
    if let Err(e) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    std::fs::rename(&tmp, out).map_err(|e| PackageError::Io(format!("cannot write {}: {e}", out.display())))?;
    Ok(to_hex(&digest))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::manifest::tests::SAMPLE;

    /// A package of the sample manifest with tiny programs, written to a temp folder.
    pub(crate) fn sample(dir: &Path, key: Option<&SecretKey>) -> std::path::PathBuf {
        let win = dir.join("kv-lines.exe");
        let lin = dir.join("kv-lines");
        std::fs::write(&win, b"MZ windows program").unwrap();
        std::fs::write(&lin, b"\x7fELF linux program").unwrap();
        let out = dir.join("kv.imtplugin");
        build(
            Build {
                manifest: PackageManifest::parse(SAMPLE).unwrap(),
                programs: vec![("x86_64-pc-windows-msvc".into(), &win), ("x86_64-unknown-linux-gnu".into(), &lin)],
                sign: key,
            },
            &out,
        )
        .unwrap();
        out
    }

    /// Rewrites the zip at `path` with `edit` applied to its (name, bytes) list.
    pub(crate) fn rewrite(path: &Path, edit: impl FnOnce(&mut Vec<(String, Vec<u8>)>)) {
        let mut zip = zip::ZipArchive::new(std::fs::File::open(path).unwrap()).unwrap();
        let mut files = Vec::new();
        for i in 0..zip.len() {
            let mut e = zip.by_index(i).unwrap();
            let mut b = Vec::new();
            e.read_to_end(&mut b).unwrap();
            files.push((e.name().unwrap().into_owned(), b));
        }
        drop(zip);
        edit(&mut files);
        let mut w = zip::ZipWriter::new(std::fs::File::create(path).unwrap());
        for (n, b) in files {
            w.start_file(n, zip::write::SimpleFileOptions::default()).unwrap();
            w.write_all(&b).unwrap();
        }
        w.finish().unwrap();
    }

    #[test]
    fn a_signed_package_builds_reads_back_and_verifies() {
        let t = tempfile::tempdir().unwrap();
        let key = SecretKey::generate().unwrap();
        let p = sample(t.path(), Some(&key));
        let pkg = Package::open(&p).unwrap();
        assert_eq!(pkg.signature, Signature::Signed { key: key.public() });
        assert_eq!(pkg.manifest.key.as_deref(), Some(key.public().id().as_str()));
        assert_eq!(pkg.entries.len(), 3, "{:?}", pkg.entries.keys());
        assert_eq!(pkg.digest().len(), 64);
        let dest = t.path().join("out.exe");
        let sha = pkg.extract(&p, "x86_64-pc-windows-msvc", &dest).unwrap();
        assert_eq!(std::fs::read(&dest).unwrap(), b"MZ windows program");
        assert_eq!(sha, pkg.program("x86_64-pc-windows-msvc").unwrap().sha256);
        assert_eq!(pkg.extract(&p, "aarch64-apple-darwin", &dest).unwrap_err().code(), "wrongTarget");
        // Deterministic: the same inputs give the same bytes.
        let first = std::fs::read(&p).unwrap();
        let again = sample(t.path(), Some(&key));
        assert_eq!(std::fs::read(again).unwrap(), first);
    }

    #[test]
    fn an_unsigned_package_is_unsigned() {
        let t = tempfile::tempdir().unwrap();
        let pkg = Package::open(&sample(t.path(), None)).unwrap();
        assert_eq!(pkg.signature, Signature::Unsigned);
        assert_eq!(pkg.manifest.key, None);
    }

    #[test]
    fn tampering_with_a_program_or_the_manifest_is_detected() {
        let t = tempfile::tempdir().unwrap();
        let key = SecretKey::generate().unwrap();
        let p = sample(t.path(), Some(&key));
        rewrite(&p, |f| f.iter_mut().find(|(n, _)| n.ends_with(".exe")).unwrap().1.push(0));
        let e = Package::open(&p).unwrap_err();
        assert_eq!(e.code(), "tampered", "{e}");
        let p = sample(t.path(), Some(&key));
        rewrite(&p, |f| {
            let m = &mut f.iter_mut().find(|(n, _)| n == MANIFEST).unwrap().1;
            *m = String::from_utf8(m.clone()).unwrap().replace("1.2.0", "9.9.9").into_bytes();
        });
        assert_eq!(Package::open(&p).unwrap_err().code(), "tampered");
        // Removing the signature but keeping the manifest's key.
        let p = sample(t.path(), Some(&key));
        rewrite(&p, |f| f.retain(|(n, _)| n != SIGNATURE));
        assert_eq!(Package::open(&p).unwrap_err().code(), "tampered");
        // A signature by another key, with the manifest pointing at it: still the wrong signature.
        let p = sample(t.path(), Some(&key));
        let other = SecretKey::generate().unwrap();
        rewrite(&p, |f| {
            let s = &mut f.iter_mut().find(|(n, _)| n == SIGNATURE).unwrap().1;
            let mut file: SignatureFile = toml::from_str(std::str::from_utf8(s).unwrap()).unwrap();
            file.public_key = other.public().to_text();
            *s = toml::to_string(&file).unwrap().into_bytes();
        });
        assert_eq!(Package::open(&p).unwrap_err().code(), "tampered");
    }

    #[test]
    fn layouts_that_are_not_allowed_are_refused() {
        let t = tempfile::tempdir().unwrap();
        type Edit = Box<dyn Fn(&mut Vec<(String, Vec<u8>)>)>;
        let cases: Vec<(Edit, &str)> = vec![
            (Box::new(|f| f.push(("extra.txt".into(), b"x".to_vec()))), "unlistedFile"),
            (Box::new(|f| f.push(("../evil.exe".into(), b"x".to_vec()))), "unsafeEntry"),
            (
                Box::new(|f| f.push(("bin/x86_64-pc-windows-msvc/other.exe".into(), b"x".to_vec()))),
                "unlistedFile",
            ),
            (Box::new(|f| f.retain(|(n, _)| !n.ends_with("kv-lines"))), "missingFile"),
            (Box::new(|f| f.retain(|(n, _)| n != MANIFEST)), "missingFile"),
        ];
        for (edit, code) in cases {
            let p = sample(t.path(), None);
            rewrite(&p, edit);
            let e = Package::open(&p).unwrap_err();
            assert_eq!(e.code(), code, "{e}");
        }
        std::fs::write(t.path().join("no.imtplugin"), b"not a zip").unwrap();
        assert_eq!(Package::open(&t.path().join("no.imtplugin")).unwrap_err().code(), "notAPackage");
        assert_eq!(Package::open(&t.path().join("missing.imtplugin")).unwrap_err().code(), "io");
    }

    #[test]
    fn building_needs_every_listed_program_and_nothing_else() {
        let t = tempfile::tempdir().unwrap();
        let prog = t.path().join("p");
        std::fs::write(&prog, b"x").unwrap();
        let m = PackageManifest::parse(SAMPLE).unwrap();
        let missing = build(
            Build {
                manifest: m.clone(),
                programs: vec![("x86_64-pc-windows-msvc".into(), &prog)],
                sign: None,
            },
            &t.path().join("a.imtplugin"),
        );
        assert_eq!(missing.unwrap_err().code(), "missingFile");
        let extra = build(
            Build {
                manifest: m,
                programs: vec![
                    ("x86_64-pc-windows-msvc".into(), &prog),
                    ("x86_64-unknown-linux-gnu".into(), &prog),
                    ("aarch64-apple-darwin".into(), &prog),
                ],
                sign: None,
            },
            &t.path().join("b.imtplugin"),
        );
        assert_eq!(extra.unwrap_err().code(), "unlistedFile");
    }
}
