//! `latest.json`, the release information published with every release (see docs/releasing.md):
//!
//! ```json
//! { "version": "1.2.0", "notes": "...", "pub_date": "2026-10-17T12:00:00Z",
//!   "platforms": { "windows-x86_64-nsis": { "url": "https://...", "signature": "...", "size": 41234567 } } }
//! ```
//!
//! The updater plugin reads the same file; this module parses it again into typed, bounded values for
//! what IMT shows (notes, size) and refuses anything malformed before a download starts. `size` is
//! IMT's addition (the plugin ignores it). Parsing never panics, whatever the input.

use base64::Engine;
use serde_json::{Map, Value};
use std::collections::BTreeMap;

/// A `latest.json` larger than this is not release information.
#[cfg_attr(not(test), allow(dead_code))]
pub const MAX_MANIFEST_BYTES: usize = 1 << 20;
/// Release notes beyond this many characters are cut (with an ellipsis).
pub const MAX_NOTES_CHARS: usize = 20_000;
const MAX_PLATFORMS: usize = 64;
const MAX_URL_LEN: usize = 2048;
const MAX_SIGNATURE_LEN: usize = 8192;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub version: semver::Version,
    /// Plain text, control characters removed (except new lines and tabs), bounded.
    pub notes: String,
    /// RFC 3339, as published.
    pub pub_date: Option<String>,
    pub platforms: BTreeMap<String, Platform>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Platform {
    pub url: String,
    pub signature: String,
    pub size: Option<u64>,
}

impl Manifest {
    /// The platform entry whose download URL is `url` (the one the plugin chose).
    pub fn platform_by_url(&self, url: &str) -> Option<&Platform> {
        self.platforms.values().find(|p| p.url == url)
    }
}

/// Parses `latest.json` text (the plugin fetches it; this is for tests and tools).
#[cfg_attr(not(test), allow(dead_code))]
pub fn parse(text: &str) -> Result<Manifest, String> {
    if text.len() > MAX_MANIFEST_BYTES {
        return Err(format!("the release information is larger than {} KB", MAX_MANIFEST_BYTES / 1024));
    }
    let value: Value = serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
    from_value(&value)
}

/// Validates release information already parsed as JSON (the plugin's raw response).
pub fn from_value(value: &Value) -> Result<Manifest, String> {
    let root = value.as_object().ok_or("expected a JSON object")?;
    let version = root.get("version").and_then(Value::as_str).ok_or("missing \"version\"")?;
    let version = semver::Version::parse(version.strip_prefix('v').unwrap_or(version))
        .map_err(|e| format!("\"version\" is not a semantic version: {e}"))?;
    let notes = match root.get("notes") {
        None | Some(Value::Null) => String::new(),
        Some(Value::String(s)) => sanitize_notes(s),
        Some(_) => return Err("\"notes\" must be text".into()),
    };
    let pub_date = match root.get("pub_date") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) => {
            chrono::DateTime::parse_from_rfc3339(s).map_err(|e| format!("\"pub_date\" is not an RFC 3339 date: {e}"))?;
            Some(s.clone())
        }
        Some(_) => return Err("\"pub_date\" must be text".into()),
    };
    let platforms = root.get("platforms").and_then(Value::as_object).ok_or("missing \"platforms\"")?;
    if platforms.is_empty() {
        return Err("\"platforms\" is empty".into());
    }
    if platforms.len() > MAX_PLATFORMS {
        return Err(format!("more than {MAX_PLATFORMS} platforms"));
    }
    let platforms = platforms
        .iter()
        .map(|(key, p)| {
            platform(key, p)
                .map(|p| (key.clone(), p))
                .map_err(|e| format!("platform \"{}\": {e}", clip(key)))
        })
        .collect::<Result<_, _>>()?;
    Ok(Manifest {
        version,
        notes,
        pub_date,
        platforms,
    })
}

fn platform(key: &str, value: &Value) -> Result<Platform, String> {
    let key_ok = !key.is_empty()
        && key.len() <= 64
        && key
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if !key_ok {
        return Err("invalid platform name".into());
    }
    let p: &Map<String, Value> = value.as_object().ok_or("expected an object")?;
    let url = p.get("url").and_then(Value::as_str).ok_or("missing \"url\"")?;
    if url.len() > MAX_URL_LEN {
        return Err("\"url\" is too long".into());
    }
    let parsed = tauri::Url::parse(url).map_err(|e| format!("\"url\" is not a URL: {e}"))?;
    if parsed.scheme() != "https" {
        return Err("\"url\" must use https".into());
    }
    let signature = p.get("signature").and_then(Value::as_str).ok_or("missing \"signature\"")?;
    check_signature(signature)?;
    let size = match p.get("size") {
        None | Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().ok_or("\"size\" must be a whole number of bytes")?),
    };
    Ok(Platform {
        url: url.to_string(),
        signature: signature.to_string(),
        size,
    })
}

/// A Tauri updater signature is the base64 of a minisign signature file. The plugin verifies it; this
/// only rejects what cannot be one.
fn check_signature(signature: &str) -> Result<(), String> {
    if signature.is_empty() {
        return Err("\"signature\" is empty".into());
    }
    if signature.len() > MAX_SIGNATURE_LEN {
        return Err("\"signature\" is too long".into());
    }
    let decoded = base64::engine::general_purpose::STANDARD
        .decode(signature.trim())
        .map_err(|_| "\"signature\" is not base64")?;
    let text = String::from_utf8(decoded).map_err(|_| "\"signature\" is not a minisign signature")?;
    if !text.starts_with("untrusted comment:") {
        return Err("\"signature\" is not a minisign signature".into());
    }
    Ok(())
}

/// Release notes as plain text: control characters other than new lines and tabs removed, at most
/// [`MAX_NOTES_CHARS`] characters.
pub fn sanitize_notes(notes: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for c in notes.chars() {
        if c.is_control() && c != '\n' && c != '\t' {
            continue;
        }
        if count == MAX_NOTES_CHARS {
            out.push('…');
            break;
        }
        out.push(c);
        count += 1;
    }
    out.trim().to_string()
}

fn clip(s: &str) -> String {
    s.chars().take(40).collect()
}

/// Whether `key` is a valid minisign public key as Tauri stores it (base64 of the `.pub` file).
pub fn is_public_key(key: &str) -> bool {
    let engine = base64::engine::general_purpose::STANDARD;
    let Ok(decoded) = engine.decode(key.trim()) else { return false };
    let Ok(text) = String::from_utf8(decoded) else { return false };
    let mut lines = text.lines();
    let comment = lines.next().unwrap_or_default();
    let Some(key_line) = lines.next() else { return false };
    // "Ed" + 8 bytes key id + 32 bytes Ed25519 key.
    comment.starts_with("untrusted comment:") && engine.decode(key_line.trim()).is_ok_and(|k| k.len() == 42 && k.starts_with(b"Ed"))
}
