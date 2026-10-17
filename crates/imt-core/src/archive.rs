//! Transparent reading of compressed input: gzip files and zip archives are
//! detected by their magic bytes (not their extension) and expanded into the
//! files they contain. Decompressed sizes are capped so a hostile archive
//! (a "zip bomb") fails with a message instead of exhausting memory.

use anyhow::{anyhow, Context, Result};
use std::io::{Cursor, Read};

/// Largest decompressed size of one entry (1 GiB).
pub const MAX_ENTRY_BYTES: u64 = 1 << 30;
/// Largest total decompressed size of one archive (4 GiB).
pub const MAX_ARCHIVE_BYTES: u64 = 4 << 30;
/// Separator between an archive's path and an entry inside it.
pub const ENTRY_SEPARATOR: &str = "!/";

/// One file to import: the bytes, and the entry name when it came from an archive.
pub struct Entry {
    /// `None` for a plain file; the entry path for a zip member; the inner file
    /// name for a gzip file.
    pub name: Option<String>,
    pub bytes: Vec<u8>,
}

pub fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&[0x1f, 0x8b])
}

pub fn is_zip(bytes: &[u8]) -> bool {
    bytes.starts_with(b"PK\x03\x04")
}

/// Expands `bytes` (the content of `file_name`) into the files to import.
pub fn expand(file_name: &str, bytes: Vec<u8>) -> Result<Vec<Entry>> {
    if is_gzip(&bytes) {
        let inner = file_name
            .strip_suffix(".gz")
            .or_else(|| file_name.strip_suffix(".GZ"))
            .unwrap_or(file_name);
        let data = read_capped(flate2::read::MultiGzDecoder::new(Cursor::new(bytes)), MAX_ENTRY_BYTES).context("invalid gzip data")?;
        return Ok(vec![Entry {
            name: Some(inner.to_string()),
            bytes: data,
        }]);
    }
    if is_zip(&bytes) {
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).context("invalid zip archive")?;
        let mut out = Vec::new();
        let mut total = 0u64;
        for i in 0..zip.len() {
            let entry = zip.by_index(i).context("invalid zip entry")?;
            if entry.is_dir() {
                continue;
            }
            let name = entry.name().context("invalid zip entry name")?.replace('\\', "/");
            let data = read_capped(entry, MAX_ENTRY_BYTES).with_context(|| format!("zip entry {name}"))?;
            total += data.len() as u64;
            if total > MAX_ARCHIVE_BYTES {
                return Err(anyhow!("archive expands to more than {} GiB", MAX_ARCHIVE_BYTES >> 30));
            }
            out.push(Entry {
                name: Some(name),
                bytes: data,
            });
        }
        return Ok(out);
    }
    Ok(vec![Entry { name: None, bytes }])
}

/// Reads one entry of an archive again (for the context view).
pub fn read_entry(bytes: Vec<u8>, file_name: &str, entry: &str) -> Result<Vec<u8>> {
    expand(file_name, bytes)?
        .into_iter()
        .find(|e| e.name.as_deref() == Some(entry))
        .map(|e| e.bytes)
        .ok_or_else(|| anyhow!("{entry} is no longer in {file_name}"))
}

fn read_capped(reader: impl Read, cap: u64) -> Result<Vec<u8>> {
    let mut data = Vec::new();
    reader.take(cap + 1).read_to_end(&mut data)?;
    if data.len() as u64 > cap {
        return Err(anyhow!("expands to more than {} MiB", cap >> 20));
    }
    Ok(data)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    pub(crate) fn gzip(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    pub(crate) fn zip_of(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut w = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let opts = zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Deflated);
        w.add_directory("logs/", opts).unwrap();
        for (name, data) in files {
            w.start_file(*name, opts).unwrap();
            w.write_all(data).unwrap();
        }
        w.finish().unwrap().into_inner()
    }

    #[test]
    fn expands_gzip_zip_and_plain_files() {
        let plain = expand("a.log", b"hello".to_vec()).unwrap();
        assert_eq!((plain[0].name.as_deref(), plain[0].bytes.as_slice()), (None, &b"hello"[..]));

        let gz = expand("app.log.gz", gzip(b"line 1\nline 2\n")).unwrap();
        assert_eq!(
            (gz[0].name.as_deref(), gz[0].bytes.as_slice()),
            (Some("app.log"), &b"line 1\nline 2\n"[..])
        );
        // Detected by content: no .gz suffix needed.
        assert_eq!(expand("rotated.1", gzip(b"x")).unwrap()[0].name.as_deref(), Some("rotated.1"));

        let z = zip_of(&[("logs/a.log", b"A"), ("logs\\b.log", b"B")]);
        let entries = expand("bundle.zip", z.clone()).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.clone().unwrap()).collect();
        assert_eq!(names, ["logs/a.log", "logs/b.log"]);
        assert_eq!(read_entry(z.clone(), "bundle.zip", "logs/b.log").unwrap(), b"B");
        assert!(read_entry(z, "bundle.zip", "missing").is_err());
    }

    #[test]
    fn rejects_corrupt_and_oversized_data() {
        assert!(expand("bad.gz", vec![0x1f, 0x8b, 0, 1, 2]).is_err());
        assert!(expand("bad.zip", b"PK\x03\x04garbage".to_vec()).is_err());
        assert!(read_capped(Cursor::new(vec![0u8; 11]), 10)
            .unwrap_err()
            .to_string()
            .contains("more than"));
        assert_eq!(read_capped(Cursor::new(vec![0u8; 10]), 10).unwrap().len(), 10);
        assert!(is_gzip(&gzip(b"")) && is_zip(&zip_of(&[])) && !is_zip(b"PK"));
    }
}
