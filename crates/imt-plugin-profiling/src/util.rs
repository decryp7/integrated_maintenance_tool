//! Helpers shared by the importers: bounded decompression, text decoding and
//! cheap sniffing of JSON heads.

use imt_profile::{Limits, ProfileError, Result};
use std::borrow::Cow;
use std::io::Read;

/// gzip magic.
pub const GZIP: [u8; 2] = [0x1f, 0x8b];

pub fn is_gzip(bytes: &[u8]) -> bool {
    bytes.starts_with(&GZIP)
}

/// Inflates gzip data, refusing more than `limits.max_input_bytes` of output
/// (a decompression bomb is an error, not an out-of-memory abort).
pub fn gunzip(bytes: &[u8], limits: &Limits, format: &'static str) -> Result<Vec<u8>> {
    let cap = limits.max_input_bytes;
    let mut out = Vec::new();
    let mut reader = flate2::read::MultiGzDecoder::new(bytes).take(cap.saturating_add(1));
    reader
        .read_to_end(&mut out)
        .map_err(|e| ProfileError::malformed(format, format!("gzip data is damaged: {e}")))?;
    if out.len() as u64 > cap {
        return Err(ProfileError::LimitExceeded {
            what: "decompressed size",
            limit: cap,
            hint: "open a smaller capture",
        });
    }
    Ok(out)
}

/// The bytes, inflated when they are gzip.
pub fn maybe_gunzip<'a>(bytes: &'a [u8], limits: &Limits, format: &'static str) -> Result<Cow<'a, [u8]>> {
    if is_gzip(bytes) {
        Ok(Cow::Owned(gunzip(bytes, limits, format)?))
    } else {
        Ok(Cow::Borrowed(bytes))
    }
}

/// UTF-8 text without a byte order mark.
pub fn text<'a>(bytes: &'a [u8], format: &'static str) -> Result<&'a str> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    std::str::from_utf8(bytes).map_err(|e| ProfileError::malformed(format, format!("the file is not UTF-8 text ({e})")))
}

/// The head as (lossy) text without BOM and leading whitespace, for sniffing.
pub fn head_text(head: &[u8]) -> Cow<'_, str> {
    let head = head.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(head);
    match String::from_utf8_lossy(head) {
        Cow::Borrowed(s) => Cow::Borrowed(s.trim_start()),
        Cow::Owned(s) => Cow::Owned(s.trim_start().to_string()),
    }
}

/// Parses JSON with serde_json (which limits nesting to 128 levels, so deep
/// input is an error rather than a stack overflow).
pub fn json<T: serde::de::DeserializeOwned>(bytes: &[u8], format: &'static str) -> Result<T> {
    let bytes = bytes.strip_prefix(b"\xEF\xBB\xBF").unwrap_or(bytes);
    serde_json::from_slice(bytes).map_err(|e| ProfileError::malformed(format, format!("invalid JSON: {e}")))
}

/// The file name's extension, lower case.
pub fn extension(file_name: &str) -> String {
    let base = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    base.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn gz(data: &[u8]) -> Vec<u8> {
        let mut e = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        e.write_all(data).unwrap();
        e.finish().unwrap()
    }

    #[test]
    fn gunzip_is_bounded() {
        let data = vec![b'a'; 10_000];
        let z = gz(&data);
        assert!(is_gzip(&z));
        assert_eq!(gunzip(&z, &Limits::default(), "t").unwrap(), data);
        assert_eq!(&*maybe_gunzip(&z, &Limits::default(), "t").unwrap(), &data[..]);
        assert_eq!(&*maybe_gunzip(b"plain", &Limits::default(), "t").unwrap(), b"plain");
        let small = Limits {
            max_input_bytes: 100,
            ..Limits::default()
        };
        assert!(matches!(gunzip(&z, &small, "t"), Err(ProfileError::LimitExceeded { .. })));
        assert!(gunzip(&[0x1f, 0x8b, 0, 0], &Limits::default(), "t").is_err());
    }

    #[test]
    fn text_json_and_names() {
        assert_eq!(text(b"\xEF\xBB\xBFabc", "t").unwrap(), "abc");
        assert!(text(&[0xff, 0xfe], "t").is_err());
        assert_eq!(head_text(b"\xEF\xBB\xBF  {\"a\"").as_ref(), "{\"a\"");
        assert_eq!(head_text(b" \xff{").as_ref(), "\u{fffd}{");
        let v: serde_json::Value = json(b"{\"a\":1}", "t").unwrap();
        assert_eq!(v["a"], 1);
        assert!(json::<serde_json::Value>(b"{", "t").is_err());
        let deep = "[".repeat(10_000);
        assert!(json::<serde_json::Value>(deep.as_bytes(), "t").is_err(), "deep nesting is an error");
        assert_eq!(extension("a/b/C.CpuProfile"), "cpuprofile");
        assert_eq!(extension("noext"), "");
        assert_eq!(extension(r"c:\x.y\z.pb.gz"), "gz");
    }
}
